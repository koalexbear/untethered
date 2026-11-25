use super::{Command as Operation, Commands, Events};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{env, fs, net::SocketAddr, path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    time,
};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

// The Arch package ships gomuks next to the frontend's installation prefix,
// at ../lib/untethered/gomuks relative to bin/. Dev launchers supply both paths.
#[cfg(target_os = "linux")]
fn bundled_backend() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let path = exe.parent()?.parent()?.join("lib/untethered/gomuks");
    path.is_file().then_some(path)
}
#[cfg(target_os = "linux")]
fn default_profile() -> Option<PathBuf> {
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
        .map(|p| p.join("untethered-iced"))
}
#[cfg(not(target_os = "linux"))]
fn bundled_backend() -> Option<PathBuf> {
    None
}
#[cfg(not(target_os = "linux"))]
fn default_profile() -> Option<PathBuf> {
    None
}

pub async fn run(commands: Commands, events: Events) -> Result<(), String> {
    let exe = env::var_os("UNTETHERED_BACKEND")
        .map(PathBuf::from)
        .or_else(bundled_backend)
        .ok_or("Use the Rust client launcher or set UNTETHERED_BACKEND to the pinned gomuks executable.")?;
    let root = env::var_os("UNTETHERED_PROFILE")
        .map(PathBuf::from)
        .or_else(default_profile)
        .ok_or("Set UNTETHERED_PROFILE to a separate Rust client profile directory.")?;
    let mut dir = fs::DirBuilder::new();
    dir.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut dir, 0o700);
    dir.create(&root)
        .map_err(|_| "Cannot create the profile directory.")?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("rust-client.lock"))
        .map_err(|_| "Cannot open the profile lock.")?;
    lock.try_lock()
        .map_err(|_| "This profile is already open.")?;

    let key = uuid::Uuid::new_v4().to_string() + &uuid::Uuid::new_v4().to_string();
    let mut child = Command::new(exe);
    #[cfg(windows)]
    child.creation_flags(0x08000000); // CREATE_NO_WINDOW for the console backend.
    for (k, _) in env::vars_os() {
        if k.to_string_lossy().starts_with("GOMUKS_") {
            child.env_remove(k);
        }
    }
    let mut child = child
        .arg("--desktop")
        .env("GOMUKS_ROOT", &root)
        .env("GOMUKS_DESKTOP_KEY", &key)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "Cannot launch gomuks.")?;
    let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
    let addr = time::timeout(Duration::from_secs(15), async {
        while let Some(line) = stdout
            .next_line()
            .await
            .map_err(|_| "Cannot read gomuks startup.")?
        {
            let Ok(frame) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if frame["started"] == true {
                return frame["address"]
                    .as_str()
                    .and_then(|s| s.parse::<SocketAddr>().ok())
                    .filter(|a| a.ip().is_loopback() && a.port() != 0)
                    .ok_or("Invalid gomuks loopback address.");
            }
        }
        Err("gomuks stopped during startup.")
    })
    .await
    .map_err(|_| "gomuks startup timed out.")??;
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| "Cannot initialize HTTP.")?;
    let response = http
        .post(format!("http://{addr}/_gomuks/auth?insecure_cookie=true"))
        .basic_auth("desktop-key", Some(key))
        .send()
        .await
        .map_err(|_| "Cannot reach gomuks.")?;
    if !response.status().is_success() {
        return Err("gomuks rejected local authentication.".into());
    }
    let cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|h| h.to_str().ok()?.split(';').next())
        .find(|s| s.starts_with("gomuks_auth="))
        .ok_or("Missing gomuks session cookie.")?
        .to_owned();
    let mut request = format!("ws://{addr}/_gomuks/websocket")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Cookie", cookie.parse().map_err(|_| "Invalid cookie.")?);
    let (mut ws, _) = time::timeout(
        Duration::from_secs(8),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .map_err(|_| "gomuks connection timed out.")?
    .map_err(|_| "Cannot connect to gomuks.")?;
    let mut heartbeat = time::interval(Duration::from_secs(15));
    let mut last_received = 0;
    let mut last_frame = time::Instant::now();
    let mut transfers = tokio::task::JoinSet::new();
    // --desktop also writes notification previews to stdout. Delivery uses
    // the WebSocket's sync/rule decisions; drain this second copy without
    // logging message text or allowing a full pipe to stall gomuks.
    transfers.spawn(async move {
        let _ = tokio::io::copy(&mut stdout.into_inner(), &mut tokio::io::sink()).await;
    });
    loop {
        let outgoing = tokio::select! {
            frame = ws.next() => {
                match frame {
                    Some(Ok(Message::Text(text))) => {
                        let frame: Value = serde_json::from_str(&text).map_err(|_| "Invalid gomuks frame.")?;
                        if let Some(id) = frame["request_id"].as_i64().filter(|&id| id < 0) { last_received = id; }
                        last_frame = time::Instant::now();
                        if events.send(frame).await.is_err() { break; }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return Err("Disconnected from gomuks. Reopen the client to reconnect.".into()),
                    _ => {}
                }
                continue;
            }
            cmd = commands.recv() => match cmd {
                Ok(Operation::Rpc(v))=>v,
                #[cfg(target_os = "linux")]
                Ok(Operation::Voice{room,target,controls})=>{
                    let cookie=cookie.clone(); let events=events.clone();
                    transfers.spawn(async move {
                        let result=crate::desktop_call::run(addr,&cookie,&room,target,controls,&events).await;
                        let _=events.send(json!({"command":"desktop_call","data":{"room":room,"ended":true,"label":result.unwrap_or_else(|e| e)}})).await;
                    });continue
                }
                Ok(Operation::Inline{source,result})=>{
                    let http=http.clone();let cookie=cookie.clone();
                    transfers.spawn(async move{
                        let image=inline(&http,addr,&cookie,source).await;
                        let _=result.send(image).await;
                    });continue
                }
                Ok(op)=>{
                    let http=http.clone();let cookie=cookie.clone();let events=events.clone();
                    transfers.spawn(async move {
                        let (id,result)=match op {
                            Operation::Upload{id,file,encrypt,limit,cancel} => {
                                let upload=upload(&http,addr,&cookie,file,encrypt,limit,|fraction|{
                                    let _=events.try_send(json!({"command":"upload_progress","request_id":id,"data":fraction}));
                                });
                                tokio::select!{
                                    biased;
                                    _=cancel.recv()=>return,
                                    result=upload=>(id,result),
                                }
                            }
                            Operation::Download{id,media,destination} => (id,download(&http,addr,&cookie,media,destination).await),
                            Operation::Rpc(_) | Operation::Inline{..}=>unreachable!(),
                            #[cfg(target_os = "linux")]
                            Operation::Voice{..}=>unreachable!(),
                        };
                        let (cmd,data)=match result {Ok(v)=>("response",v),Err(why)=>("error",json!(why))};
                        let _=events.send(json!({"command":cmd,"request_id":id,"data":data})).await;
                    });
                    continue;
                },
                Err(_) => break
            },
            Some(_) = transfers.join_next(), if !transfers.is_empty() => continue,
            _ = heartbeat.tick() => {
                if last_frame.elapsed() > Duration::from_secs(75) { return Err("gomuks stopped responding.".into()); }
                json!({"command":"ping","request_id":0,"data":{"last_received_id":last_received}})
            }
            _ = child.wait() => return Err("gomuks exited.".into()),
        };
        ws.send(Message::Text(outgoing.to_string().into()))
            .await
            .map_err(|_| "Cannot send to gomuks.")?;
    }
    let _ = ws.close(None).await;
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status()
            .await;
        if time::timeout(Duration::from_secs(3), child.wait())
            .await
            .is_ok()
        {
            return Ok(());
        }
    }
    let _ = child.kill().await;
    Ok(())
}

async fn inline(
    http: &reqwest::Client,
    addr: SocketAddr,
    cookie: &str,
    source: crate::media::PreviewRef,
) -> Option<iced::widget::image::Handle> {
    use crate::media::{INLINE_BYTES, media_path};
    let path = media_path(&source.mxc)?;
    let mut resp = http
        .get(format!("http://{addr}{path}"))
        .header("Cookie", cookie)
        .query(&[("encrypted", source.encrypted)])
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    if resp
        .content_length()
        .is_some_and(|n| n > INLINE_BYTES as u64)
    {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        if chunk.len() > INLINE_BYTES - bytes.len() {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    crate::preview::inline(bytes).await
}

async fn upload(
    http: &reqwest::Client,
    addr: SocketAddr,
    cookie: &str,
    file: crate::media::LocalFile,
    encrypt: bool,
    limit: u64,
    mut progress: impl FnMut(f64),
) -> Result<Value, &'static str> {
    if file.size > limit {
        return Err("File exceeds the upload limit.");
    }
    let body = match &file.origin {
        crate::media::NativeSource::Clipboard(bytes) => {
            if bytes.len() as u64 != file.size {
                return Err("Clipboard image size mismatch.");
            }
            reqwest::Body::from(bytes.clone())
        }
        origin => {
            let path = origin.read_path().ok_or("File source unavailable.")?;
            let f = tokio::fs::File::open(path)
                .await
                .map_err(|_| "File cannot be opened.")?;
            let meta = f.metadata().await.map_err(|_| "Cannot read file size.")?;
            if !meta.is_file() || meta.len() != file.size {
                return Err("File changed. Remove and attach it again.");
            }
            reqwest::Body::from(f)
        }
    };
    let mut reply = http
        .post(format!("http://{addr}/_gomuks/upload"))
        .header("Cookie", cookie)
        .header("Content-Length", file.size)
        .query(&[
            ("progress", "true"),
            ("filename", file.name.as_str()),
            ("encrypt", if encrypt { "true" } else { "false" }),
        ])
        .timeout(Duration::from_secs(600))
        .body(body)
        .send()
        .await
        .map_err(|_| "Upload interrupted.")?;
    if !reply.status().is_success() {
        return Err("gomuks rejected the upload.");
    }
    // gomuks streams newline-separated JSON fractions from its homeserver
    // request, followed by the media descriptor (or a Matrix error object).
    // A 200 response alone is not upload success. Lines may span HTTP chunks.
    let mut line = Vec::new();
    while let Some(chunk) = reply.chunk().await.map_err(|_| "Upload interrupted.")? {
        for piece in chunk.split_inclusive(|b| *b == b'\n') {
            if line.len() + piece.len() > 256 * 1024 {
                return Err("Invalid upload response.");
            }
            line.extend_from_slice(piece);
            if line.ends_with(b"\n") {
                let value: Value =
                    serde_json::from_slice(&line).map_err(|_| "Invalid upload response.")?;
                line.clear();
                if let Some(fraction) = value.as_f64() {
                    progress(fraction);
                } else if value["msgtype"].is_string() {
                    return Ok(value);
                } else {
                    return Err("gomuks rejected the upload.");
                }
            }
        }
    }
    Err("Upload ended without a media descriptor.")
}
async fn download(
    http: &reqwest::Client,
    addr: SocketAddr,
    cookie: &str,
    media: crate::media::Media,
    destination: PathBuf,
) -> Result<Value, &'static str> {
    let path = crate::media::media_path(&media.mxc).ok_or("Invalid media address.")?;
    let mut resp = http
        .get(format!("http://{addr}{path}"))
        .header("Cookie", cookie)
        .query(&[("encrypted", media.encrypted)])
        .timeout(Duration::from_secs(600))
        .send()
        .await
        .map_err(|_| "Could not download attachment.")?
        .error_for_status()
        .map_err(|_| "gomuks rejected the download.")?;
    let tmp = tempfile::NamedTempFile::new_in(destination.parent().ok_or("Invalid destination.")?)
        .map_err(|_| "Cannot create download.")?;
    let (file, tmp) = tmp.into_parts();
    let mut file = tokio::fs::File::from_std(file);
    use tokio::io::AsyncWriteExt;
    while let Some(bytes) = resp.chunk().await.map_err(|_| "Download interrupted.")? {
        file.write_all(&bytes)
            .await
            .map_err(|_| "Cannot save download.")?;
    }
    file.flush().await.map_err(|_| "Cannot save download.")?;
    drop(file);
    tmp.persist(destination)
        .map_err(|_| "Cannot finish saving download.")?;
    Ok(Value::Null)
}
