//go:build js && wasm

// Browser adapter for the pinned gomuks worker. Go's default fetch transport
// buffers the entire request before fetch, so a counting io.Reader would
// measure local encryption/copying, not network upload. Only media uploads
// opt in to this transport; gomuks still encrypts and authenticates them.
package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync"
	"syscall/js"
	"time"

	"go.mau.fi/gomuks/pkg/gomuks"
)

var activeUploads sync.Map // request ID -> context.CancelFunc

func uploadContext(id int64) (context.Context, func()) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Minute)
	activeUploads.Store(id, cancel)
	ctx = context.WithValue(ctx, gomuks.UntetheredUploadClientKey{}, &http.Client{Transport: uploadXHR(id)})
	return ctx, func() {
		activeUploads.Delete(id)
		cancel()
	}
}

func cancelUpload(id int64) {
	if cancel, ok := activeUploads.Load(id); ok {
		cancel.(context.CancelFunc)()
	}
}

type uploadXHR int64

func (id uploadXHR) RoundTrip(req *http.Request) (resp *http.Response, err error) {
	if req.Body == nil {
		return nil, errors.New("missing upload body")
	}
	defer req.Body.Close()
	if err := req.Context().Err(); err != nil {
		return nil, err
	}
	// Turn JS DOM exceptions (e.g. refused URL) into an ordinary RPC error.
	defer func() {
		if recover() != nil {
			resp, err = nil, errors.New("browser rejected upload")
		}
	}()
	const max = 64 << 20
	payload, err := io.ReadAll(io.LimitReader(req.Body, max+1))
	if err != nil || len(payload) > max {
		return nil, errors.New("could not read upload within size limit")
	}
	bytes := js.Global().Get("Uint8Array").New(len(payload))
	js.CopyBytesToJS(bytes, payload)
	xhr := js.Global().Get("XMLHttpRequest").New()
	xhr.Call("open", req.Method, req.URL.String(), true)
	// The Matrix HTTP client owns the request URL, bearer and content type.
	for key, values := range req.Header {
		switch http.CanonicalHeaderKey(key) {
		case "Content-Length", "User-Agent", "Host", "Cookie":
			continue // browser-managed
		}
		for _, value := range values {
			xhr.Call("setRequestHeader", key, value)
		}
	}
	done := make(chan bool, 1)
	report := func(ok bool) {
		select {
		case done <- ok:
		default:
		}
	}
	load := js.FuncOf(func(_ js.Value, _ []js.Value) any { report(true); return nil })
	fail := js.FuncOf(func(_ js.Value, _ []js.Value) any { report(false); return nil })
	last := time.Time{}
	progress := js.FuncOf(func(_ js.Value, args []js.Value) any {
		e := args[0]
		if !e.Get("lengthComputable").Bool() {
			return nil
		}
		n, total := e.Get("loaded").Float(), e.Get("total").Float()
		if total > 0 && (n == total || time.Since(last) >= 150*time.Millisecond) {
			last = time.Now()
			postMessage("upload_progress", int64(id), n/total)
		}
		return nil
	})
	defer func() {
		for _, field := range []string{"onload", "onerror", "onabort", "ontimeout"} {
			xhr.Set(field, js.Null())
		}
		xhr.Get("upload").Set("onprogress", js.Null())
		load.Release()
		fail.Release()
		progress.Release()
	}()
	xhr.Set("onload", load)
	xhr.Set("onerror", fail)
	xhr.Set("onabort", fail)
	xhr.Set("ontimeout", fail)
	xhr.Get("upload").Set("onprogress", progress)
	xhr.Call("send", bytes)
	select {
	case <-req.Context().Done():
		xhr.Call("abort")
		return nil, req.Context().Err()
	case ok := <-done:
		if !ok {
			return nil, errors.New("browser upload interrupted")
		}
	}
	status := xhr.Get("status").Int()
	return &http.Response{
		StatusCode: status, Status: fmt.Sprintf("%d %s", status, http.StatusText(status)),
		Header: http.Header{"Content-Type": []string{"application/json"}},
		Body:   io.NopCloser(strings.NewReader(xhr.Get("responseText").String())), Request: req,
	}, nil
}
