use super::{Result, room_video::camera_remote};
use std::{fmt, process::Stdio, time::Duration};

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) enum CameraChoice {
    #[default]
    Default,
    Source {
        serial: u64,
        label: String,
    },
}
impl fmt::Display for CameraChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => f.write_str("System default"),
            Self::Source { label, .. } => f.write_str(label),
        }
    }
}
pub(crate) async fn list_cameras() -> Result<Vec<CameraChoice>> {
    // libwebrtc embeds incompatible PipeWire symbols. Keep the native PW
    // registry client in its own process, just like the GStreamer reader.
    let exe = std::env::current_exe().map_err(|_| "Could not locate camera helper.")?;
    let dir = exe.parent().ok_or("Could not locate camera helper.")?;
    let helper = if exe.file_name().is_some_and(|n| n == "untethered") {
        dir.join("../lib/untethered/untethered-cameras")
    } else {
        dir.join("untethered-cameras")
    };
    let fd = camera_remote().await?;
    let proc = tokio::process::Command::new(helper)
        .stdin(Stdio::from(fd))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "Camera helper unavailable. Build or install untethered-cameras.")?;
    let out = tokio::time::timeout(Duration::from_secs(5), proc.wait_with_output())
        .await
        .map_err(|_| "Camera list timed out.")?
        .map_err(|_| "Camera list unavailable.")?;
    if !out.status.success() {
        return Err("Could not list portal cameras.");
    }
    let entries: Vec<(String, u64)> =
        serde_json::from_slice(&out.stdout).map_err(|_| "Invalid camera list.")?;
    let mut result = vec![CameraChoice::Default];
    for (i, (name, serial)) in entries.iter().enumerate() {
        let duplicate =
            i > 0 && entries[i - 1].0 == *name || entries.get(i + 1).is_some_and(|n| n.0 == *name);
        result.push(CameraChoice::Source {
            serial: *serial,
            label: if duplicate {
                format!("{name} ({serial})")
            } else {
                name.clone()
            },
        });
    }
    Ok(result)
}
