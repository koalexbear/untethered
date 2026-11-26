//! The tray on Windows: an icon in the notification area, through tray-icon.
//! The icon and its menu belong to the thread that made them, which must
//! pump window messages, so they get a thread of their own; the app wakes it
//! with a thread message when there's something new to show.
use super::{Event, Snapshot, icon};
use async_channel::Sender;
use std::sync::{Arc, Mutex};
use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};
use windows_sys::Win32::{
    System::Threading::GetCurrentThreadId,
    UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_APP, WM_QUIT,
    },
};

/// Asks the tray thread to show the newest snapshot.
const SHOW: u32 = WM_APP + 1;

#[derive(Clone)]
pub struct Link {
    thread: u32,
    shown: Arc<Mutex<Snapshot>>,
}
impl Link {
    pub fn show(&self, now: Snapshot) -> iced::Task<crate::Msg> {
        *self.shown.lock().unwrap() = now;
        // SAFETY: posting to a thread that has a message queue (it made a
        // window for the icon before it reported in).
        unsafe { PostThreadMessageW(self.thread, SHOW, 0, 0) };
        iced::Task::none()
    }
    /// End the tray thread, which takes the icon away: one left behind by a
    /// process that exits stays in the notification area until hovered.
    pub fn shutdown(&self) {
        // SAFETY: as in `show`.
        unsafe { PostThreadMessageW(self.thread, WM_QUIT, 0, 0) };
    }
}

pub async fn start(events: Sender<Event>, _patient: bool) -> Option<Link> {
    let shown = Arc::new(Mutex::new(Snapshot::default()));
    let (ready, started) = async_channel::bounded(1);
    let state = shown.clone();
    std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || run(events, state, ready))
        .ok()?;
    let thread = started.recv().await.ok()??;
    Some(Link { thread, shown })
}

struct Entries {
    answer: MenuItem,
    decline: MenuItem,
    mute: CheckMenuItem,
    deafen: CheckMenuItem,
    dnd: CheckMenuItem,
}

fn picture(now: &Snapshot) -> Option<tray_icon::Icon> {
    // Windows scales it to the notification area's 16 to 24 pixels.
    let img = icon::draw(32, now);
    tray_icon::Icon::from_rgba(img.rgba, img.size, img.size).ok()
}

fn apply(tray: &TrayIcon, e: &Entries, now: &Snapshot) {
    let _ = tray.set_icon(picture(now));
    let summary = now.summary();
    let tip = if summary.is_empty() {
        "Untethered".to_owned()
    } else {
        format!("Untethered\n{summary}")
    };
    let _ = tray.set_tooltip(Some(tip));
    let call = now.call.as_ref();
    e.answer.set_enabled(now.ringing.is_some());
    e.answer.set_text(match &now.ringing {
        Some(caller) => format!("Answer {caller}"),
        None => "Answer call".to_owned(),
    });
    e.decline.set_enabled(now.ringing.is_some());
    e.mute.set_enabled(call.is_some_and(|c| !c.deafened));
    e.mute
        .set_checked(call.is_some_and(|c| c.muted || c.deafened));
    e.deafen.set_enabled(call.is_some());
    e.deafen.set_checked(call.is_some_and(|c| c.deafened));
    e.dnd.set_enabled(now.dnd.is_some());
    e.dnd.set_checked(now.dnd == Some(true));
}

fn run(events: Sender<Event>, shown: Arc<Mutex<Snapshot>>, ready: Sender<Option<u32>>) {
    let open = MenuItem::new("Open Untethered", true, None);
    let entries = Entries {
        answer: MenuItem::new("Answer call", false, None),
        decline: MenuItem::new("Decline", false, None),
        mute: CheckMenuItem::new("Mute", false, false, None),
        deafen: CheckMenuItem::new("Deafen", false, false, None),
        dnd: CheckMenuItem::new("Do Not Disturb", false, false, None),
    };
    let quit = MenuItem::new("Quit Untethered", true, None);
    let menu = Menu::new();
    let built = menu.append_items(&[
        &open,
        &PredefinedMenuItem::separator(),
        &entries.answer,
        &entries.decline,
        &PredefinedMenuItem::separator(),
        &entries.mute,
        &entries.deafen,
        &entries.dnd,
        &PredefinedMenuItem::separator(),
        &quit,
    ]);
    let tray = built.ok().and_then(|_| {
        TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("Untethered")
            .with_icon(picture(&Snapshot::default())?)
            .build()
            .ok()
    });
    let Some(tray) = tray else {
        let _ = ready.try_send(None);
        return;
    };
    let ids = [
        (open.id().clone(), Event::Open),
        (entries.answer.id().clone(), Event::Answer),
        (entries.decline.id().clone(), Event::Decline),
        (entries.mute.id().clone(), Event::Mute),
        (entries.deafen.id().clone(), Event::Deafen),
        (entries.dnd.id().clone(), Event::Dnd),
        (quit.id().clone(), Event::Quit),
    ];
    let to_app = events.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if let Some((_, event)) = ids.iter().find(|(id, _)| *id == e.id) {
            let _ = to_app.try_send(event.clone());
        }
    }));
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = e
        {
            let _ = events.try_send(Event::Open);
        }
    }));
    // SAFETY: plain Win32 calls on this thread's own message queue.
    unsafe {
        let _ = ready.try_send(Some(GetCurrentThreadId()));
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if msg.hwnd.is_null() && msg.message == SHOW {
                let now = shown.lock().unwrap().clone();
                apply(&tray, &entries, &now);
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
