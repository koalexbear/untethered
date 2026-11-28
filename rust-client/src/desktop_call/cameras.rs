use super::{Result, room_video::camera_remote};
use std::{fmt, process::Stdio, time::Duration};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum CameraChoice {
    #[default]
    Default,
    /// `target` is what PipeWire connects to: the node's name, which
    /// survives restarts and is saved (call_settings.rs), else its serial.
    Source { target: String, label: String },
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
    let entries: Vec<(String, u64, Option<String>)> =
        serde_json::from_slice(&out.stdout).map_err(|_| "Invalid camera list.")?;
    Ok(choices(&entries))
}
/// A node name gst-launch can take as it is: it joins its arguments into
/// one pipeline description, so spaces or quotes would break it.
fn plain(target: &str) -> bool {
    !target.is_empty()
        && target
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-:".contains(c))
}
/// The helper's cameras (name, serial, node name) as choices, after the
/// system default. Two cameras of the same name are told apart by serial.
fn choices(entries: &[(String, u64, Option<String>)]) -> Vec<CameraChoice> {
    let mut result = vec![CameraChoice::Default];
    for (i, (name, serial, node)) in entries.iter().enumerate() {
        let duplicate =
            i > 0 && entries[i - 1].0 == *name || entries.get(i + 1).is_some_and(|n| n.0 == *name);
        let node = node.as_ref().filter(|n| plain(n));
        let shared_node =
            node.is_some_and(|n| entries.iter().filter(|e| e.2.as_ref() == Some(n)).count() > 1);
        result.push(CameraChoice::Source {
            target: match node {
                Some(node) if !shared_node => node.clone(),
                _ => serial.to_string(),
            },
            label: if duplicate {
                format!("{name} ({serial})")
            } else {
                name.clone()
            },
        });
    }
    result
}
impl CameraChoice {
    /// The camera the page saved, as a choice before any list is read.
    pub(crate) fn saved(camera: Option<&crate::call_settings::Camera>) -> Self {
        match camera {
            Some(c) if plain(&c.target) => Self::Source {
                target: c.target.clone(),
                label: c.label.clone(),
            },
            _ => Self::Default,
        }
    }
    /// What PipeWire connects to; None for the system default.
    pub(crate) fn target(&self) -> Option<&str> {
        match self {
            Self::Default => None,
            Self::Source { target, .. } => Some(target),
        }
    }
    /// What the page saves for this choice.
    pub(crate) fn to_saved(&self) -> Option<crate::call_settings::Camera> {
        match self {
            Self::Default => None,
            Self::Source { target, label } => Some(crate::call_settings::Camera {
                target: target.clone(),
                label: label.clone(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cameras_are_chosen_by_node_name_and_told_apart_by_serial() {
        let list = choices(&[
            ("Cobalt".into(), 71, Some("qa-second-camera".into())),
            ("Webcam".into(), 80, None),
            ("Webcam".into(), 81, Some("v4l2_input.usb-2".into())),
        ]);
        assert_eq!(list[0], CameraChoice::Default);
        let targets: Vec<_> = list[1..]
            .iter()
            .map(|c| match c {
                CameraChoice::Source { target, label } => (target.as_str(), label.as_str()),
                CameraChoice::Default => unreachable!(),
            })
            .collect();
        assert_eq!(
            targets,
            [
                ("qa-second-camera", "Cobalt"),
                ("80", "Webcam (80)"),
                ("v4l2_input.usb-2", "Webcam (81)")
            ]
        );
        let saved = list[1].to_saved();
        assert_eq!(CameraChoice::saved(saved.as_ref()), list[1]);
        assert_eq!(CameraChoice::saved(None), CameraChoice::Default);
        // A hand-edited file can't smuggle in pipeline syntax.
        let odd = crate::call_settings::Camera {
            target: "x ! filesink location=/tmp/y".into(),
            label: "Odd".into(),
        };
        assert_eq!(CameraChoice::saved(Some(&odd)), CameraChoice::Default);
    }
}
