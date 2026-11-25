//! Single native participant. Obtain the JWT and peer's sender key from Matrix,
//! never from the browser observer or the runner's SFU signing secret.
use super::{publish, receive};
use anyhow::{Context, Result, bail, ensure};
use futures_util::{SinkExt, StreamExt};
use livekit::{
    e2ee::{E2eeOptions, EncryptionType},
    prelude::*,
};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

#[path = "../../../rust-client/src/rtc_keys.rs"]
mod keys;
#[path = "../../../rust-client/src/rtc_membership.rs"]
mod membership;
use keys::{EVENT as KEY, SenderKey};
impl SenderKey {
    async fn share(
        &self,
        rpc: &mut Rpc,
        room: &str,
        member: &str,
        device: &str,
        peer: &str,
        peer_device: &str,
    ) -> Result<()> {
        rpc.call(
            "send_to_device",
            self.to_device(room, member, device, peer, peer_device),
        )
        .await?;
        Ok(())
    }
}
struct Rpc {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    seq: u64,
    keys: Vec<Value>,
}
impl Rpc {
    async fn frame(&mut self) -> Result<Value> {
        loop {
            let frame = self.ws.next().await.context("gomuks disconnected")??;
            if let Message::Text(t) = frame {
                let f: Value = serde_json::from_str(&t)?;
                if f["command"] == "sync_complete" {
                    self.keys.extend(
                        f["data"]["to_device"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|e| e["encrypted"] == true && e["type"] == KEY)
                            .cloned(),
                    );
                }
                return Ok(f);
            }
        }
    }
    async fn call(&mut self, cmd: &str, body: Value) -> Result<Value> {
        self.seq += 1;
        self.ws
            .send(Message::Text(
                json!({"request_id":self.seq,"command":cmd,"data":body})
                    .to_string()
                    .into(),
            ))
            .await?;
        loop {
            let f = self.frame().await?;
            if f["request_id"] == self.seq {
                if f["command"] == "response" {
                    return Ok(f["data"].clone());
                }
                if f["command"] == "error" {
                    if cmd == "get_room_state" {
                        bail!("gomuks room-state query failed: {}", f["data"])
                    }
                    bail!("gomuks {cmd} failed")
                }
            }
        }
    }
}

pub async fn run() -> Result<()> {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let cfg: Value = serde_json::from_str(&line)?;
    let text = |k: &str| cfg[k].as_str().context("missing QA input");
    let ws = url::Url::parse(text("ws")?)?;
    let auth = url::Url::parse(text("service")?)?;
    ensure!(
        ws.scheme() == "ws"
            && ws.host_str() == Some("127.0.0.1")
            && auth.scheme() == "http"
            && auth.host_str() == Some("127.0.0.1"),
        "only disposable loopback services"
    );
    let mut req = ws.as_str().into_client_request()?;
    req.headers_mut().insert(
        "Cookie",
        format!("gomuks_auth={}", text("cookie")?).parse()?,
    );
    let mut rpc = Rpc {
        ws: connect_async(req).await?.0,
        seq: 0,
        keys: vec![],
    };
    let room = text("room")?;
    let peer = text("user")?;
    let peer_device = text("device")?;
    let me = rpc.call("get_state", json!({})).await?;
    ensure!(me["is_verified"] == true, "unverified test device");
    let user = me["user_id"].as_str().context("no Matrix user")?;
    let device = me["device_id"].as_str().context("no Matrix device")?;
    ensure!(user != peer);
    let my_id = format!("{user}:{device}");
    let peer_id = format!("{peer}:{peer_device}");
    rpc.call("listen_to_device", json!(true)).await?;
    // On startup client_state may precede the first persisted room snapshot.
    let events = tokio::time::timeout(Duration::from_secs(12),async {
        loop {
            let result=rpc.call("get_room_state",json!({"room_id":room,"refetch":true,"fetch_members":true,"include_members":true})).await;
            match result {
                Err(e) if e.to_string().contains("room not found") => tokio::time::sleep(Duration::from_millis(250)).await,
                other => return other,
            }
        }
    }).await.context("initial room sync")??;
    let state = events.as_array().context("no room state")?;
    ensure!(
        state.iter().any(|e| e["type"] == "m.room.encryption"
            && e["content"]["algorithm"] == "m.megolm.v1.aes-sha2")
    );
    ensure!(state.iter().any(|e| e["type"] == "m.room.member"
        && e["state_key"] == peer
        && e["content"]["membership"] == "join"));
    let member = state
        .iter()
        .find(|e| {
            e["type"] == membership::EVENT_TYPE
                && e["sender"] == peer
                && e["content"]["device_id"] == peer_device
                && e["content"]["application"] == "m.call"
                && e["content"]["call_id"] == ""
        })
        .context("Element's call membership missing")?;
    let peer_member = member["content"]["membershipID"]
        .as_str()
        .unwrap_or(&peer_id);
    ensure!(
        member["content"]["foci_preferred"][0]["livekit_service_url"] == text("service")?,
        "Element advertised another transport"
    );
    let my_member =
        membership::Membership::join(room, user, device, &auth, 30000, async |cmd, body| {
            rpc.call(cmd, body).await.map_err(|e| e.to_string())
        })
        .await
        .map_err(anyhow::Error::msg)?;
    let openid = rpc.call("request_openid_token", json!({})).await?;
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()?;
    let authorization: Value = http
        .post(auth.join("sfu/get")?)
        .json(&json!({"room":room,"device_id":device,"openid_token":openid}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let url = authorization["url"].as_str().context("missing SFU URL")?;
    ensure!(url == "ws://127.0.0.1:18820", "unexpected test SFU");
    let jwt = authorization["jwt"].as_str().context("missing SFU token")?;
    println!("Native: obtained SFU authorization with the verified gomuks device's OpenID token.");
    let mine = SenderKey::fresh(0).map_err(anyhow::Error::msg)?;
    let (index, theirs) = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            for e in rpc.keys.drain(..) {
                let c = &e["content"];
                if e["sender"] != peer || c["room_id"] != room {
                    continue;
                }
                if let Some((idx, bytes)) =
                    keys::receive(c, peer_device, peer_member).map_err(anyhow::Error::msg)?
                {
                    ensure!(bytes != mine.bytes, "sender keys must be distinct");
                    return Ok::<_, anyhow::Error>((idx, bytes));
                }
            }
            rpc.frame().await?;
        }
    })
    .await
    .context("no encrypted sender key from Element")??;
    println!(
        "Native: received Element's distinct sender key from gomuks via Olm at index {index}."
    );
    let provider = keys::provider();
    ensure!(
        provider.set_key(&peer_id.clone().into(), index, theirs)
            && provider.set_key(&my_id.clone().into(), 0, mine.bytes.to_vec())
    );
    let mut opts = RoomOptions::default();
    opts.encryption = Some(E2eeOptions {
        encryption_type: EncryptionType::Gcm,
        key_provider: provider.clone(),
    });
    let (lk, events) = Room::connect(url, jwt, opts)
        .await
        .context("SFU connection failed")?;
    ensure!(lk.local_participant().identity().0 == my_id);
    let tone = publish(&lk, 480).await?;
    my_member
        .heartbeat(async |cmd, body| rpc.call(cmd, body).await.map_err(|e| e.to_string()))
        .await
        .map_err(anyhow::Error::msg)?;
    tokio::time::timeout(
        Duration::from_secs(22),
        receive(events, 960., Some(&peer_id)),
    )
    .await
    .context("native did not decode Element's tone")??;
    println!(
        "Native decoded Element's 960 Hz microphone on its GCM-encrypted audio track (20 consecutive PCM frames)."
    );
    async fn ack(expected: &str) -> Result<()> {
        let line = tokio::task::spawn_blocking(|| {
            let mut s = String::new();
            std::io::stdin().read_line(&mut s).map(|_| s)
        })
        .await??;
        ensure!(line.trim() == expected, "browser observation failed");
        Ok(())
    }
    ack("release-sender-key").await?;
    mine.share(&mut rpc, room, &my_id, device, peer, peer_device)
        .await?;
    println!("Native: released sender key through gomuks to Element's call device only.");
    ack("rotate-after-480").await?;
    let mut lifecycle = lk.subscribe();
    let publication_sid = tone.publication.sid();
    let local = lk.local_participant().identity();
    let cryptor = lk
        .e2ee_manager()
        .frame_cryptors()
        .remove(&(local.clone(), publication_sid.clone()))
        .context("no cryptor for live sender")?;
    ensure!(
        cryptor.enabled()
            && cryptor.key_index() == 0
            && lk.connection_state() == ConnectionState::Connected
    );
    let next = SenderKey::fresh(1).map_err(anyhow::Error::msg)?;
    ensure!(next.bytes != mine.bytes);
    // Adding a key to the provider alone doesn't select it for an existing RTP sender.
    ensure!(provider.set_key(&local, i32::from(next.index), next.bytes.to_vec()));
    cryptor.set_key_index(i32::from(next.index));
    ensure!(cryptor.enabled() && cryptor.key_index() == 1);
    tone.change_to(730).await?;
    println!("ROTATION_ACTIVE index=1 tone_hz=730");
    ack("release-rotated-key").await?;
    next.share(&mut rpc, room, &my_id, device, peer, peer_device)
        .await?;
    ack("element-decoded-730").await?;
    ensure!(
        cryptor.enabled()
            && cryptor.key_index() == 1
            && tone.publication.encryption_type() == EncryptionType::Gcm
    );
    ensure!(
        lk.connection_state() == ConnectionState::Connected
            && tone.publication.sid() == publication_sid
    );
    while let Ok(event) = lifecycle.try_recv() {
        ensure!(
            !matches!(
                event,
                RoomEvent::Reconnecting
                    | RoomEvent::Reconnected
                    | RoomEvent::Disconnected { .. }
                    | RoomEvent::LocalTrackRepublished { .. }
                    | RoomEvent::LocalTrackUnpublished { .. }
                    | RoomEvent::ParticipantDisconnected(_)
            ),
            "native participant reconnected or publication replaced during rotation"
        );
    }
    println!("Native: same GCM publication at sender key index 1; no disconnection or republish.");
    drop(tone);
    lk.close().await?;
    my_member
        .leave(async |cmd, body| rpc.call(cmd, body).await.map_err(|e| e.to_string()))
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(())
}
