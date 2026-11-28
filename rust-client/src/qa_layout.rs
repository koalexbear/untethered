//! The QA control map (calls plan §5): in debug builds started with
//! `UNTETHERED_QA_LAYOUT=<file>`, the window's tagged controls are written to
//! that file as JSON, by name, with their bounds, state, tooltip and text.
//! The call fixtures click icon-only controls through it instead of reading
//! tooltips (`scripts/qa_layout.py`).
//!
//! A view tags a control with [`tag`], which always wraps it in a container
//! so the widget tree is the same with and without the map. Every 300 ms a
//! widget operation walks the tree, overlays included, and collects the
//! tagged containers: their bounds in window coordinates (scrolled content
//! included), and the text of the text widgets inside them.
use iced::{
    Element, Rectangle, Vector,
    advanced::widget::{Id, Operation, operation::Outcome, operation::Scrollable},
    widget::container,
};
use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex, OnceLock},
};

/// Where the map goes, in debug builds that asked for it.
fn path() -> Option<&'static std::path::Path> {
    static PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        cfg!(debug_assertions)
            .then(|| std::env::var_os("UNTETHERED_QA_LAYOUT"))
            .flatten()
            .filter(|p| !p.is_empty())
            .map(Into::into)
    })
    .as_deref()
}

/// Whether this run writes the map.
pub fn enabled() -> bool {
    path().is_some()
}

/// What a tagged control says about itself besides its bounds.
#[derive(Clone, Debug, Default, PartialEq)]
struct Info {
    name: String,
    state: &'static str,
    tip: String,
}

/// The tags the last views made, by their container's id. A tag absent from
/// the tree is never reported, so old entries can stay.
static TAGS: LazyLock<Mutex<HashMap<Id, Info>>> = LazyLock::new(Default::default);

/// `content` as the control `name`, in `state` (such as "on", "off" or
/// "selected"; empty when it has none), with the tooltip `tip`.
pub fn tag<'a, M: 'a>(
    name: impl Into<String>,
    state: &'static str,
    tip: impl Into<String>,
    content: impl Into<Element<'a, M>>,
) -> Element<'a, M> {
    let wrapped = container(content);
    if !enabled() {
        return wrapped.into();
    }
    let name = name.into();
    let id = Id::from(format!("qa:{name}"));
    let info = Info {
        name,
        state,
        tip: tip.into(),
    };
    if let Ok(mut tags) = TAGS.lock()
        && tags.get(&id) != Some(&info)
    {
        tags.insert(id.clone(), info);
    }
    wrapped.id(id).into()
}

/// One control found in the tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    info: Info,
    bounds: Rectangle,
    text: Vec<String>,
}

/// Collects the tagged containers and scrollables.
#[derive(Default)]
struct Collect {
    /// How far the content being walked is scrolled: subtracted from the
    /// bounds its widgets report.
    offset: Vector,
    /// What the next traversal adds to `offset`: a scrollable's translation.
    pending: Vector,
    /// The tagged control the next traversal enters.
    entering: Option<usize>,
    /// The tagged controls being walked, innermost last.
    inside: Vec<usize>,
    found: Vec<Found>,
}

impl Collect {
    fn note(&mut self, id: Option<&Id>, bounds: Rectangle) {
        self.pending = Vector::ZERO;
        self.entering = None;
        let Some(info) = id.and_then(|id| TAGS.lock().ok()?.get(id).cloned()) else {
            return;
        };
        self.entering = Some(self.found.len());
        self.found.push(Found {
            info,
            bounds: bounds - self.offset,
            text: vec![],
        });
    }
}

impl Operation<Vec<Found>> for Collect {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Vec<Found>>)) {
        let (offset, entered) = (self.offset, self.entering.take());
        self.offset = offset + std::mem::take(&mut self.pending);
        if let Some(i) = entered {
            self.inside.push(i);
        }
        operate(self);
        if entered.is_some() {
            self.inside.pop();
        }
        self.offset = offset;
        self.pending = Vector::ZERO;
    }
    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        self.note(id, bounds);
    }
    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        _content: Rectangle,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        self.note(id, bounds);
        self.pending = translation;
    }
    fn text(&mut self, _id: Option<&Id>, _bounds: Rectangle, text: &str) {
        for &i in &self.inside {
            self.found[i].text.push(text.to_owned());
        }
    }
    fn finish(&self) -> Outcome<Vec<Found>> {
        Outcome::Some(self.found.clone())
    }
}

/// Walk the window's tree for its tagged controls.
pub fn collect() -> iced::Task<Vec<Found>> {
    iced::advanced::widget::operate(Collect::default())
}

/// The map's JSON: each control by name. A name shown twice keeps its
/// first (outermost) bounds.
fn render(found: &[Found]) -> Value {
    let mut controls = Map::new();
    for f in found {
        let b = f.bounds;
        controls.entry(f.info.name.clone()).or_insert_with(|| {
            json!({
                "x": b.x.round(), "y": b.y.round(), "w": b.width.round(), "h": b.height.round(),
                "state": f.info.state, "tip": f.info.tip, "text": f.text.join(" "),
            })
        });
    }
    Value::Object(controls)
}

/// The last map written, so an unchanged one isn't written again.
static WRITTEN: Mutex<Option<Value>> = Mutex::new(None);

/// Write the map if it changed: to a temporary file renamed over it, so a
/// fixture never reads half of one.
pub fn write(found: &[Found]) {
    let Some(path) = path() else { return };
    let controls = render(found);
    let Ok(mut written) = WRITTEN.lock() else {
        return;
    };
    if written.as_ref() == Some(&controls) {
        return;
    }
    let body = json!({ "controls": controls }).to_string();
    let temporary = path.with_extension("tmp");
    if std::fs::write(&temporary, body).is_ok() && std::fs::rename(&temporary, path).is_ok() {
        *written = Some(controls);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::widget::operation::scrollable;

    fn found(name: &str, x: f32, text: &[&str]) -> Found {
        Found {
            info: Info {
                name: name.into(),
                state: "on",
                tip: "Mute".into(),
            },
            bounds: Rectangle::new(iced::Point::new(x, 2.4), iced::Size::new(32., 32.)),
            text: text.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn controls_by_name_keep_the_first() {
        let map = render(&[
            found("mute", 10.6, &[]),
            found("mute", 90., &[]),
            found("menu", 0., &["A", "B"]),
        ]);
        assert_eq!(map["mute"]["x"], 11.0);
        assert_eq!(map["mute"]["y"], 2.0);
        assert_eq!(map["mute"]["state"], "on");
        assert_eq!(map["mute"]["tip"], "Mute");
        assert_eq!(map["menu"]["text"], "A B");
    }

    #[test]
    fn scrolled_content_and_nested_text() {
        // A tagged scrollable scrolled 40 px, holding a tagged control
        // with text, and a tagged control after it outside.
        let tags = [
            ("qa:list", "list"),
            ("qa:item", "item"),
            ("qa:after", "after"),
        ];
        for (id, name) in tags {
            TAGS.lock().unwrap().insert(
                Id::from(id.to_owned()),
                Info {
                    name: name.into(),
                    ..Info::default()
                },
            );
        }
        let rect = |y| Rectangle::new(iced::Point::new(0., y), iced::Size::new(100., 20.));
        let mut c = Collect::default();
        struct Fixed;
        impl Scrollable for Fixed {
            fn snap_to(&mut self, _: scrollable::RelativeOffset<Option<f32>>) {}
            fn scroll_to(&mut self, _: scrollable::AbsoluteOffset<Option<f32>>) {}
            fn scroll_by(&mut self, _: scrollable::AbsoluteOffset, _: Rectangle, _: Rectangle) {}
        }
        c.scrollable(
            Some(&Id::from("qa:list".to_owned())),
            rect(100.),
            rect(100.),
            Vector::new(0., 40.),
            &mut Fixed,
        );
        c.traverse(&mut |op| {
            op.container(Some(&Id::from("qa:item".to_owned())), rect(200.));
            op.traverse(&mut |op| {
                op.container(None, rect(200.));
                op.traverse(&mut |op| op.text(None, rect(200.), "Cobalt"));
            });
            op.text(None, rect(230.), "outside the item");
        });
        c.container(Some(&Id::from("qa:after".to_owned())), rect(300.));
        let Outcome::Some(found) = c.finish() else {
            panic!("no outcome")
        };
        let by = |n: &str| found.iter().find(|f| f.info.name == n).unwrap();
        assert_eq!(by("list").bounds.y, 100.);
        assert_eq!(by("list").text, ["Cobalt", "outside the item"]);
        assert_eq!(by("item").bounds.y, 160.);
        assert_eq!(by("item").text, ["Cobalt"]);
        assert_eq!(by("after").bounds.y, 300.);
    }
}
