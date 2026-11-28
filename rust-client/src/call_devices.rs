//! Linux call-local PulseAudio routes. A device name, not its transient index
//! or display label, is passed to pacat. None leaves the session default intact.
use serde_json::Value;
use std::{collections::BTreeMap, fmt, time::Duration};
use tokio::{process::Command, time::timeout};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Port {
    Mic,
    Speaker,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Route {
    pub name: Option<String>,
    // Selecting the same endpoint again retries it after a device failure.
    pub revision: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Choice {
    pub name: Option<String>,
    label: String,
}
impl fmt::Display for Choice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}
impl Choice {
    fn system() -> Self {
        Self {
            name: None,
            label: "System default".into(),
        }
    }
    #[cfg(test)]
    pub fn for_test(name: &str, label: &str) -> Self {
        Self {
            name: Some(name.into()),
            label: label.into(),
        }
    }
}

#[derive(Clone)]
pub struct Catalog {
    pub mics: Vec<Choice>,
    pub speakers: Vec<Choice>,
    pub streams: Vec<AppStream>,
}
#[derive(Clone, PartialEq, Eq)]
pub struct AppStream {
    pub index: u32,
    pub monitor: String,
    sink: u32,
    client: u32,
    serial: Option<String>,
    label: String,
}
impl fmt::Display for AppStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}
impl Default for Catalog {
    fn default() -> Self {
        Self {
            mics: vec![Choice::system()],
            speakers: vec![Choice::system()],
            streams: vec![],
        }
    }
}
impl Catalog {
    pub fn list(&self, port: Port) -> &[Choice] {
        match port {
            Port::Mic => &self.mics,
            Port::Speaker => &self.speakers,
        }
    }
    pub fn selected(&self, port: Port, route: &Route) -> Choice {
        self.list(port)
            .iter()
            .find(|c| c.name == route.name)
            .cloned()
            .unwrap_or_else(|| Choice {
                name: route.name.clone(),
                label: format!(
                    "Unavailable: {}",
                    route.name.as_deref().unwrap_or("default")
                ),
            })
    }
    pub async fn read() -> Result<Self, &'static str> {
        let (sources, sinks, inputs) =
            tokio::try_join!(rows("sources"), rows("sinks"), rows("sink-inputs"))?;
        Ok(Self {
            mics: devices(&sources),
            speakers: devices(&sinks),
            streams: app_streams(&inputs, &sinks),
        })
    }
}
async fn rows(kind: &str) -> Result<Vec<Value>, &'static str> {
    let output = timeout(
        Duration::from_secs(4),
        Command::new("pactl")
            .args(["--format=json", "list", kind])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "Audio device query timed out.")?
    .map_err(|_| "Cannot list audio devices. Is pactl installed?")?;
    if !output.status.success() {
        return Err("Audio server unavailable. Refresh to try again.");
    }
    serde_json::from_slice(&output.stdout).map_err(|_| "Invalid audio device list.")
}
fn devices(rows: &[Value]) -> Vec<Choice> {
    let mut choices: Vec<_> = rows
        .iter()
        .filter_map(|v| {
            let name = v["name"].as_str().filter(|s| !s.is_empty())?;
            let label = v["description"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(name);
            Some(Choice {
                name: Some(name.to_owned()),
                label: label.to_owned(),
            })
        })
        .collect();
    let mut counts = BTreeMap::new();
    for c in &choices {
        *counts.entry(c.label.clone()).or_insert(0) += 1;
    }
    for c in &mut choices {
        if counts[&c.label] > 1 {
            c.label = format!("{} ({})", c.label, c.name.as_deref().unwrap());
        }
    }
    choices.sort_by(|a, b| a.label.cmp(&b.label));
    choices.insert(0, Choice::system());
    choices
}
fn number(v: &Value) -> Option<u32> {
    v.as_u64()
        .and_then(|x| u32::try_from(x).ok())
        .or_else(|| v.as_str()?.parse().ok())
}
fn app_streams(inputs: &[Value], sinks: &[Value]) -> Vec<AppStream> {
    let mut streams: Vec<_> = inputs
        .iter()
        .filter_map(|r| {
            let p = &r["properties"];
            let app = p["application.name"].as_str()?.trim();
            if app.is_empty() || app == "Untethered" || app == "Untethered ringtone" {
                return None;
            }
            let index = number(&r["index"]).filter(|n| *n != u32::MAX)?;
            let client = number(&r["client"])?;
            let sink = number(&r["sink"])?;
            let monitor =
                sinks.iter().find(|s| number(&s["index"]) == Some(sink))?["monitor_source"]
                    .as_str()?
                    .to_owned();
            if monitor.is_empty() {
                return None;
            }
            let clean = |s: &str| {
                s.chars()
                    .filter(|c| !c.is_control())
                    .take(60)
                    .collect::<String>()
            };
            let name = p["media.name"]
                .as_str()
                .filter(|s| !s.is_empty() && *s != app);
            let label = match name {
                Some(n) => format!("{} · {} (#{index})", clean(app), clean(n)),
                None => format!("{} (#{index})", clean(app)),
            };
            Some(AppStream {
                index,
                sink,
                client,
                serial: p["object.serial"].as_str().map(str::to_owned),
                monitor,
                label,
            })
        })
        .collect();
    streams.sort_by(|a, b| a.label.cmp(&b.label));
    streams
}
impl AppStream {
    pub async fn present(&self) -> bool {
        let Ok(inputs) = rows("sink-inputs").await else {
            return false;
        };
        inputs.iter().any(|r| {
            number(&r["index"]) == Some(self.index)
                && number(&r["client"]) == Some(self.client)
                && number(&r["sink"]) == Some(self.sink)
                && self.serial.as_deref() == r["properties"]["object.serial"].as_str()
        })
    }
}
