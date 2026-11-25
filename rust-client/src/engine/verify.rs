//! Emoji (SAS) verification between this account's own devices.
//!
//! One verification runs at a time. This device can ask the account's other
//! devices, or accept a request from one of them. When the flow verifies our
//! own identity, matrix-sdk-crypto asks the other device for the missing
//! secrets (cross-signing and backup keys) and signs this device itself.
use super::{Event, Failure, Stage, Verification};
use async_channel::Sender;
use futures_util::StreamExt;
use matrix_sdk::{
    Client,
    encryption::verification::{
        CancelInfo, SasState, SasVerification, Verification as Flow, VerificationRequest,
        VerificationRequestState,
    },
    ruma::events::key::verification::{
        VerificationMethod, request::ToDeviceKeyVerificationRequestEvent,
    },
};
use std::sync::{Arc, Weak};
use tokio::sync::Mutex;

#[derive(Default)]
struct Current {
    request: Option<VerificationRequest>,
    sas: Option<SasVerification>,
    incoming: bool,
    device: Option<String>,
    finished: bool,
    watcher: Option<super::rt::JoinHandle<()>>,
}

pub struct Verifier {
    client: Client,
    events: Sender<Event>,
    current: Mutex<Current>,
}

impl Drop for Verifier {
    fn drop(&mut self) {
        if let Some(task) = self.current.get_mut().watcher.take() {
            task.abort();
        }
    }
}

impl Verifier {
    pub fn new(client: Client, events: Sender<Event>) -> Arc<Self> {
        let me = Arc::new(Self {
            client: client.clone(),
            events,
            current: Mutex::default(),
        });
        let weak: Weak<Self> = Arc::downgrade(&me);
        client.add_event_handler(
            move |event: ToDeviceKeyVerificationRequestEvent, client: Client| {
                let weak = weak.clone();
                async move {
                    let Some(me) = weak.upgrade() else { return };
                    let request = client
                        .encryption()
                        .get_verification_request(&event.sender, &event.content.transaction_id)
                        .await;
                    if let Some(request) = request {
                        me.adopt(request, true).await;
                    }
                }
            },
        );
        me
    }

    /// Ask this account's other devices to verify this one.
    pub async fn request(self: &Arc<Self>) -> Result<(), Failure> {
        let user = self
            .client
            .user_id()
            .ok_or(Failure::Unavailable)?
            .to_owned();
        let encryption = self.client.encryption();
        let identity = match encryption.get_user_identity(&user).await {
            Ok(Some(identity)) => identity,
            _ => encryption
                .request_user_identity(&user)
                .await
                .map_err(|_| Failure::Unreachable)?
                .ok_or(Failure::NoIdentity)?,
        };
        let request = identity
            .request_verification_with_methods(vec![VerificationMethod::SasV1])
            .await
            .map_err(|_| Failure::Failed)?;
        self.adopt(request, false).await;
        Ok(())
    }

    async fn adopt(self: &Arc<Self>, request: VerificationRequest, incoming: bool) {
        // Only this account's devices: verifying other people is a later
        // step. Ignore the rest rather than cancel them: a cancel from this
        // device would also end the request for the user's other clients.
        if !request.is_self_verification() {
            return;
        }
        let mut current = self.current.lock().await;
        let busy = current
            .request
            .as_ref()
            .is_some_and(|r| r.flow_id() != request.flow_id())
            && !current.finished;
        if busy {
            return;
        }
        if let Some(task) = current.watcher.take() {
            task.abort();
        }
        *current = Current {
            request: Some(request.clone()),
            incoming,
            ..Current::default()
        };
        current.watcher = Some(super::rt::spawn(watch(Arc::downgrade(self), request)));
    }

    pub async fn accept(&self) {
        let request = self.current.lock().await.request.clone();
        if let Some(request) = request {
            let _ = request
                .accept_with_methods(vec![VerificationMethod::SasV1])
                .await;
        }
    }

    pub async fn confirm(&self, matches: bool) {
        let sas = self.current.lock().await.sas.clone();
        if let Some(sas) = sas {
            let _ = if matches {
                sas.confirm().await
            } else {
                sas.mismatch().await
            };
        }
    }

    pub async fn cancel(&self) {
        let (request, sas) = {
            let current = self.current.lock().await;
            (current.request.clone(), current.sas.clone())
        };
        if let Some(sas) = sas {
            let _ = sas.cancel().await;
        } else if let Some(request) = request {
            let _ = request.cancel().await;
        }
    }

    async fn publish(&self, stage: Stage) {
        let view = {
            let mut current = self.current.lock().await;
            current.finished = matches!(stage, Stage::Done | Stage::Cancelled(_));
            Verification {
                incoming: current.incoming,
                device: current.device.clone(),
                stage,
            }
        };
        let _ = self.events.send(Event::Verification(Some(view))).await;
    }

    async fn name_device(&self, id: &str, name: Option<&str>) {
        self.current.lock().await.device = Some(match name {
            Some(name) if !name.is_empty() => format!("{name} ({id})"),
            _ => id.to_owned(),
        });
    }
}

fn reason(info: &CancelInfo) -> &'static str {
    match info.cancel_code().as_str() {
        "m.user" if info.cancelled_by_us() => "Verification was cancelled on this device.",
        "m.user" => "The other device cancelled the verification.",
        "m.mismatched_sas" => "The emoji didn’t match, so nothing was verified.",
        "m.timeout" => "The verification timed out. Start it again on both devices.",
        "m.accepted" => "Another device already answered this request.",
        _ => "The verification stopped before it finished.",
    }
}

async fn watch(me: Weak<Verifier>, request: VerificationRequest) {
    let mut changes = request.changes();
    let mut state = request.state();
    loop {
        let Some(verifier) = me.upgrade() else { return };
        match &state {
            VerificationRequestState::Created { .. } => verifier.publish(Stage::Waiting).await,
            VerificationRequestState::Requested {
                other_device_data, ..
            } => {
                verifier
                    .name_device(
                        other_device_data.device_id().as_str(),
                        other_device_data.display_name(),
                    )
                    .await;
                verifier.publish(Stage::Requested).await;
            }
            VerificationRequestState::Ready {
                other_device_data, ..
            } => {
                verifier
                    .name_device(
                        other_device_data.device_id().as_str(),
                        other_device_data.display_name(),
                    )
                    .await;
                verifier.publish(Stage::Starting).await;
                // The device that asked starts the emoji comparison.
                if request.we_started()
                    && let Ok(Some(sas)) = request.start_sas().await
                {
                    verifier.current.lock().await.sas = Some(sas.clone());
                    drop(verifier);
                    return watch_sas(me, sas).await;
                }
            }
            VerificationRequestState::Transitioned { verification, .. } => {
                let Flow::SasV1(sas) = verification else {
                    let _ = request.cancel().await;
                    return verifier
                        .publish(Stage::Cancelled(
                            "The other device chose a method Untethered doesn’t support yet. Use emoji.",
                        ))
                        .await;
                };
                if !sas.we_started() {
                    let _ = sas.accept().await;
                }
                verifier.current.lock().await.sas = Some(sas.clone());
                drop(verifier);
                return watch_sas(me, sas.clone()).await;
            }
            VerificationRequestState::Done => return verifier.publish(Stage::Done).await,
            VerificationRequestState::Cancelled(info) => {
                return verifier.publish(Stage::Cancelled(reason(info))).await;
            }
        }
        drop(verifier);
        match changes.next().await {
            Some(next) => state = next,
            None => return,
        }
    }
}

async fn watch_sas(me: Weak<Verifier>, sas: SasVerification) {
    let mut changes = sas.changes();
    let mut state = sas.state();
    loop {
        let Some(verifier) = me.upgrade() else { return };
        let device = sas.other_device();
        verifier
            .name_device(device.device_id().as_str(), device.display_name())
            .await;
        match &state {
            SasState::Created { .. } | SasState::Started { .. } | SasState::Accepted { .. } => {
                verifier.publish(Stage::Starting).await
            }
            SasState::KeysExchanged { .. } => {
                let emoji = sas
                    .emoji()
                    .map(|list| {
                        list.iter()
                            .map(|e| (e.symbol.to_owned(), e.description.to_owned()))
                            .collect()
                    })
                    .unwrap_or_default();
                verifier.publish(Stage::Emoji(emoji)).await;
            }
            SasState::Confirmed => verifier.publish(Stage::Confirmed).await,
            SasState::Done { .. } => return verifier.publish(Stage::Done).await,
            SasState::Cancelled(info) => {
                return verifier.publish(Stage::Cancelled(reason(info))).await;
            }
        }
        drop(verifier);
        match changes.next().await {
            Some(next) => state = next,
            None => return,
        }
    }
}
