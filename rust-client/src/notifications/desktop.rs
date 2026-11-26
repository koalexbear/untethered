use super::Notice;
use ashpd::{zbus, zvariant};
use std::collections::HashMap;

pub(super) async fn deliver(notices: Vec<Notice>) -> zbus::Result<()> {
    let bus = zbus::Connection::session().await?;
    let proxy = zbus::Proxy::new(
        &bus,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await?;
    for n in notices {
        let hints = HashMap::from([
            (
                "desktop-entry",
                zvariant::Value::from(crate::app_id::APP_ID),
            ),
            ("category", zvariant::Value::from("im.received")),
            ("suppress-sound", zvariant::Value::from(!n.sound)),
        ]);
        // Unlike browser notification bodies, the freedesktop body accepts markup.
        let body = n
            .body
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        let _: u32 = proxy
            .call(
                "Notify",
                &(
                    "Untethered",
                    0u32,
                    "internet-chat",
                    n.title,
                    body,
                    Vec::<String>::new(),
                    hints,
                    -1i32,
                ),
            )
            .await?;
    }
    Ok(())
}
