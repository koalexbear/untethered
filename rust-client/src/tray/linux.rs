//! The tray on Linux: a StatusNotifierItem over D-Bus, through ksni. KDE
//! Plasma hosts it, GNOME with the AppIndicator extension, and most other
//! panels (Waybar, XFCE, …). Without a host the item can't register, and the
//! app treats the desktop as having no tray.
use super::{Event, Snapshot, icon};
use async_channel::Sender;
use ksni::{
    MenuItem, Status, ToolTip, TrayMethods,
    menu::{CheckmarkItem, StandardItem},
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

/// The running item, for the app to send what it shows.
#[derive(Clone)]
pub struct Link {
    handle: ksni::Handle<Item>,
    shown: Arc<Mutex<Snapshot>>,
}
impl Link {
    pub fn show(&self, now: Snapshot) -> iced::Task<crate::Msg> {
        *self.shown.lock().unwrap() = now;
        let handle = self.handle.clone();
        // The item reads the shared snapshot, so updates that land out of
        // order still show the newest.
        iced::Task::future(async move {
            handle.update(|_| ()).await;
        })
        .discard()
    }
    /// Take the icon away now; the app exits shortly.
    pub fn shutdown(&self) {
        drop(self.handle.shutdown());
    }
}

/// Register the item. At sign-in (`patient`) the panel may start after us,
/// so keep trying for a while before the app falls back to its window.
pub async fn start(events: Sender<Event>, patient: bool) -> Option<Link> {
    let shown = Arc::new(Mutex::new(Snapshot::default()));
    let mut tries = if patient { 10 } else { 1 };
    loop {
        let item = Item {
            shown: shown.clone(),
            events: events.clone(),
        };
        // A sandbox may not let us own the item's well-known name.
        let spawn = item
            .disable_dbus_name(std::path::Path::new("/.flatpak-info").exists())
            .spawn();
        tries -= 1;
        match tokio::time::timeout(Duration::from_secs(5), spawn).await {
            Ok(Ok(handle)) => return Some(Link { handle, shown }),
            Ok(Err(_)) | Err(_) if tries > 0 => tokio::time::sleep(Duration::from_secs(1)).await,
            Ok(Err(e)) => {
                eprintln!("No system tray, so closing the window quits: {e}");
                return None;
            }
            Err(_) => {
                eprintln!("The system tray didn't answer, so closing the window quits.");
                return None;
            }
        }
    }
}

struct Item {
    shown: Arc<Mutex<Snapshot>>,
    events: Sender<Event>,
}
impl Item {
    fn now(&self) -> Snapshot {
        self.shown.lock().unwrap().clone()
    }
    fn send(&self, event: Event) {
        let _ = self.events.try_send(event);
    }
}
fn send(event: Event) -> Box<dyn Fn(&mut Item) + Send> {
    Box::new(move |item: &mut Item| item.send(event.clone()))
}
impl ksni::Tray for Item {
    fn id(&self) -> String {
        crate::app_id::APP_ID.into()
    }
    fn title(&self) -> String {
        "Untethered".into()
    }
    fn category(&self) -> ksni::Category {
        ksni::Category::Communications
    }
    fn status(&self) -> Status {
        // A ringing call shows even where the panel tucks icons away.
        if self.now().ringing.is_some() {
            Status::NeedsAttention
        } else {
            Status::Active
        }
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let now = self.now();
        [16, 22, 24, 32, 48, 64]
            .into_iter()
            .map(|size| {
                let img = icon::draw(size, &now);
                // ARGB32 in network byte order.
                let data = img
                    .rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|&[r, g, b, a]| [a, r, g, b])
                    .collect();
                ksni::Icon {
                    width: img.size as i32,
                    height: img.size as i32,
                    data,
                }
            })
            .collect()
    }
    fn attention_icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon_pixmap()
    }
    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Untethered".into(),
            description: self.now().summary(),
            ..Default::default()
        }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Event::Open);
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let now = self.now();
        let call = now.call.as_ref();
        let mut items = vec![
            StandardItem {
                label: "Open Untethered".into(),
                activate: send(Event::Open),
                ..Default::default()
            }
            .into(),
        ];
        if let Some(caller) = &now.ringing {
            items.extend([
                MenuItem::Separator,
                StandardItem {
                    label: format!("Answer {caller}"),
                    icon_name: "call-start".into(),
                    activate: send(Event::Answer),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Decline".into(),
                    icon_name: "call-stop".into(),
                    activate: send(Event::Decline),
                    ..Default::default()
                }
                .into(),
            ]);
        }
        items.extend([
            MenuItem::Separator,
            CheckmarkItem {
                label: "Mute".into(),
                enabled: call.is_some_and(|c| !c.deafened),
                checked: call.is_some_and(|c| c.muted || c.deafened),
                activate: send(Event::Mute),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Deafen".into(),
                enabled: call.is_some(),
                checked: call.is_some_and(|c| c.deafened),
                activate: send(Event::Deafen),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Do Not Disturb".into(),
                enabled: now.dnd.is_some(),
                checked: now.dnd == Some(true),
                activate: send(Event::Dnd),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit Untethered".into(),
                icon_name: "application-exit".into(),
                activate: send(Event::Quit),
                ..Default::default()
            }
            .into(),
        ]);
        items
    }
    fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
        // Keep the item: it registers again when a panel comes back.
        self.send(Event::Host(false));
        true
    }
    fn watcher_online(&self) {
        self.send(Event::Host(true));
    }
}
