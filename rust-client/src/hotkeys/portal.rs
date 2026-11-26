//! Wayland: the XDG GlobalShortcuts portal. The compositor shows its own
//! binding dialog and owns the keys; it sends Activated on press and
//! Deactivated on release, whichever window has focus.
use super::{DEAFEN, Event, MUTE, Shortcut, TALK};
use ashpd::desktop::{
    Session,
    global_shortcuts::{GlobalShortcuts, NewShortcut, Shortcut as Bound},
};
use iced::futures::{SinkExt, StreamExt};

type Out = iced::futures::channel::mpsc::Sender<Event>;

pub(super) async fn run(shortcuts: &[Shortcut], out: &mut Out) -> Result<(), String> {
    let error = |e: ashpd::Error| e.to_string();
    let portal = GlobalShortcuts::new().await.map_err(error)?;
    // The process holds one shortcuts session at a time (this subscription),
    // so every signal on the connection is ours.
    let session = Closing(Some(portal.create_session().await.map_err(error)?));
    let mut activated = portal.receive_activated().await.map_err(error)?;
    let mut deactivated = portal.receive_deactivated().await.map_err(error)?;
    let mut changed = portal.receive_shortcuts_changed().await.map_err(error)?;
    let requested: Vec<_> = shortcuts
        .iter()
        .map(|s| NewShortcut::new(s.id, s.description).preferred_trigger(s.portal.as_str()))
        .collect();
    let bound = portal
        .bind_shortcuts(&session, &requested, None)
        .await
        .map_err(error)?
        .response()
        .map_err(error)?;
    out.send(Event::Ready(talk_trigger(bound.shortcuts())))
        .await
        .map_err(|e| e.to_string())?;
    // Kglobalaccel can repeat Activated while a key is held. Only edges count,
    // so a repeat can't reopen a gate that a manual mute closed.
    let mut talking = false;
    loop {
        let event = tokio::select! {
            Some(signal) = activated.next() => {
                match signal.shortcut_id() {
                    TALK if !talking => {
                        talking = true;
                        Event::Talk(true)
                    }
                    MUTE => Event::ToggleMute,
                    DEAFEN => Event::ToggleDeafen,
                    _ => continue,
                }
            }
            Some(signal) = deactivated.next() => {
                if signal.shortcut_id() != TALK || !talking {
                    continue;
                }
                talking = false;
                Event::Talk(false)
            }
            Some(signal) = changed.next() => Event::Ready(talk_trigger(signal.shortcuts())),
            else => return Err("the portal closed the shortcut session".into()),
        };
        if out.send(event).await.is_err() {
            return Ok(());
        }
    }
}

/// The compositor's own name for the push-to-talk trigger. Empty when the
/// person cleared it in the dialog.
fn talk_trigger(shortcuts: &[Bound]) -> Option<String> {
    shortcuts
        .iter()
        .find(|s| s.id() == TALK)
        .map(|s| s.trigger_description().trim().to_owned())
        .filter(|t| !t.is_empty())
}

/// Closes the portal session when the subscription ends (push-to-talk off).
struct Closing(Option<Session<'static, GlobalShortcuts<'static>>>);
impl std::ops::Deref for Closing {
    type Target = Session<'static, GlobalShortcuts<'static>>;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("open session")
    }
}
impl Drop for Closing {
    fn drop(&mut self) {
        if let (Some(session), Ok(runtime)) = (self.0.take(), tokio::runtime::Handle::try_current())
        {
            runtime.spawn(async move {
                let _ = session.close().await;
            });
        }
    }
}
