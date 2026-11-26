//! Media uploads over `XMLHttpRequest` in the browser, for their progress:
//! matrix-sdk's wasm transport (fetch) reports none, so the attachment card
//! would say "Preparing…" for the whole upload. XHR's `upload.onprogress`
//! works in the engine's worker as on a page.
//!
//! Only the bytes go over XHR. The request is the SDK's own
//! (`create_content::v3`, the session's access token), and like the SDK an
//! `M_UNKNOWN_TOKEN` answer refreshes the token once and tries again.
use super::Failure;
use async_channel::Sender;
use matrix_sdk::{Client, TransmissionProgress, ruma::OwnedMxcUri};
use std::time::Duration;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{ProgressEvent, XmlHttpRequest};

/// matrix-sdk's upload timeout: long enough for its slowest expected link,
/// and never under five minutes (`Media::reasonable_upload_timeout`).
fn timeout(length: usize) -> Duration {
    const SPEED: u64 = 125_000;
    Duration::from_secs(length as u64 / SPEED).max(Duration::from_secs(300))
}

/// Upload `data` to the media repository, sending each progress event of the
/// request body to `progress`. Dropping the future aborts the request, which
/// closes its connection.
pub async fn upload(
    client: &Client,
    mime: &mime::Mime,
    data: Vec<u8>,
    progress: Sender<TransmissionProgress>,
) -> Result<OwnedMxcUri, Failure> {
    let url = format!(
        "{}/_matrix/media/v3/upload",
        client.homeserver().as_str().trim_end_matches('/')
    );
    let timeout = timeout(data.len());
    // A copy outside wasm memory, kept for the retry after a refresh.
    let body = js_sys::Uint8Array::from(data.as_slice());
    drop(data);
    let mut refreshed = false;
    loop {
        let token = client.access_token();
        let (status, text) = Request::send(
            &url,
            token.as_deref(),
            mime.essence_str(),
            &body,
            timeout,
            &progress,
        )?
        .finished()
        .await;
        let answer: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        if (200..300).contains(&status) {
            let uri = answer["content_uri"].as_str().map(OwnedMxcUri::from);
            return uri
                .filter(|uri| uri.validate().is_ok())
                .ok_or(Failure::Unreachable);
        }
        if status == 401
            && answer["errcode"] == "M_UNKNOWN_TOKEN"
            && token.is_some()
            && !refreshed
            && client.refresh_access_token().await.is_ok()
        {
            refreshed = true;
            continue;
        }
        return Err(Failure::Unreachable);
    }
}

/// One request in flight; aborted if dropped before it ends.
struct Request {
    xhr: XmlHttpRequest,
    ended: async_channel::Receiver<()>,
    _progress: Closure<dyn FnMut(ProgressEvent)>,
    _end: Closure<dyn FnMut(ProgressEvent)>,
}

impl Request {
    fn send(
        url: &str,
        token: Option<&str>,
        mime: &str,
        body: &js_sys::Uint8Array,
        timeout: Duration,
        progress: &Sender<TransmissionProgress>,
    ) -> Result<Self, Failure> {
        let failed = |_| Failure::Failed;
        let xhr = XmlHttpRequest::new().map_err(failed)?;
        xhr.open_with_async("POST", url, true).map_err(failed)?;
        if let Some(token) = token {
            xhr.set_request_header("Authorization", &format!("Bearer {token}"))
                .map_err(failed)?;
        }
        xhr.set_request_header("Content-Type", mime)
            .map_err(failed)?;
        xhr.set_timeout(u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX));
        let progress = progress.clone();
        let progress = Closure::<dyn FnMut(ProgressEvent)>::new(move |e: ProgressEvent| {
            if e.length_computable() {
                let _ = progress.try_send(TransmissionProgress {
                    current: e.loaded() as usize,
                    total: e.total() as usize,
                });
            }
        });
        // `loadend` follows success, an error, a timeout and an abort alike.
        let (end, ended) = async_channel::bounded(1);
        let end = Closure::<dyn FnMut(ProgressEvent)>::new(move |_: ProgressEvent| {
            let _ = end.try_send(());
        });
        xhr.upload()
            .map_err(failed)?
            .set_onprogress(Some(progress.as_ref().unchecked_ref()));
        xhr.set_onloadend(Some(end.as_ref().unchecked_ref()));
        let request = Self {
            xhr,
            ended,
            _progress: progress,
            _end: end,
        };
        request
            .xhr
            .send_with_opt_buffer_source(Some(body))
            .map_err(failed)?;
        Ok(request)
    }

    /// The status (0 when no answer arrived) and the response body.
    async fn finished(self) -> (u16, String) {
        let _ = self.ended.recv().await;
        let status = self.xhr.status().unwrap_or(0);
        let text = self.xhr.response_text().ok().flatten().unwrap_or_default();
        (status, text)
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        // Detach the handlers before their closures go, then stop the request
        // if it's still running (after `loadend` this changes nothing).
        if let Ok(upload) = self.xhr.upload() {
            upload.set_onprogress(None);
        }
        self.xhr.set_onloadend(None);
        let _ = self.xhr.abort();
    }
}
