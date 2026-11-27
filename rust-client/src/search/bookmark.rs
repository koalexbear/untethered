//! Measure a visible message before replacing the timeline. On return, use its
//! new content coordinate, even if messages arrived or rows changed height.
use iced::{
    Rectangle, Task, Vector,
    advanced::widget::{
        Id, Operation, operate,
        operation::{Outcome, Scrollable},
    },
    widget::operation,
};
#[derive(Clone, Default)]
pub struct Bookmark {
    row: Option<(Id, f32)>,
    top: f32,
    tail: bool,
}
impl Bookmark {
    /// A message's row, `dy` below the top of the view (places.rs).
    pub fn at(event: &str, dy: f32) -> Self {
        Self {
            row: Some((Id::from(format!("post-{event}")), dy)),
            top: 0.,
            tail: false,
        }
    }
}
struct Measure {
    timeline: Id,
    requested: Option<Id>,
    /// Only these rows count, and name the one found.
    rows: Option<std::collections::HashMap<Id, String>>,
    name: Option<String>,
    geometry: Option<(Rectangle, f32, f32)>,
    row: Option<(Id, f32)>,
    enter: bool,
    inside: bool,
    visible: bool,
}
impl Measure {
    fn new(timeline: Id, requested: Option<Id>) -> Self {
        Self {
            timeline,
            requested,
            rows: None,
            name: None,
            geometry: None,
            row: None,
            enter: false,
            inside: false,
            visible: false,
        }
    }
}
#[derive(Default)]
struct Reading {
    name: Option<String>,
    row: Option<(Id, f32)>,
    top: f32,
    max: f32,
    visible: bool,
}
impl Operation<Reading> for Measure {
    fn traverse(&mut self, op: &mut dyn FnMut(&mut dyn Operation<Reading>)) {
        let was = self.inside;
        self.inside |= std::mem::take(&mut self.enter);
        op(self);
        self.inside = was;
    }
    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content: Rectangle,
        translation: Vector,
        _: &mut dyn Scrollable,
    ) {
        if id == Some(&self.timeline) {
            self.geometry = Some((
                bounds,
                translation.y,
                (content.height - bounds.height).max(0.),
            ));
            self.enter = true;
        }
    }
    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if self.inside
            && self.row.is_none()
            && let Some(id) = id
            && let Some((view, top, _)) = self.geometry
            && self.rows.as_ref().is_none_or(|rows| rows.contains_key(id))
        {
            let y = bounds.y - view.y;
            if self
                .requested
                .as_ref()
                .map_or(y + bounds.height > top && y < top + view.height, |wanted| {
                    wanted == id
                })
            {
                self.row = Some((id.clone(), y));
                self.name = self.rows.as_ref().and_then(|rows| rows.get(id).cloned());
                self.visible = y + bounds.height > top && y < top + view.height;
            }
        }
    }
    fn finish(&self) -> Outcome<Reading> {
        let (_, top, max) = self.geometry.unwrap_or_default();
        Outcome::Some(Reading {
            name: self.name.clone(),
            row: self.row.clone(),
            top,
            max,
            visible: self.visible,
        })
    }
}
pub fn capture(timeline: Id) -> Task<Bookmark> {
    operate(Measure::new(timeline, None)).map(|m| Bookmark {
        row: m.row.map(|(id, y)| (id, y - m.top)),
        top: m.top,
        tail: m.max - m.top < 0.5,
    })
}
/// The first of `rows` showing in the timeline, named as `rows` names it,
/// and how far below the top of the view it starts.
pub fn first_visible(
    timeline: Id,
    rows: std::collections::HashMap<Id, String>,
) -> Task<Option<(String, f32)>> {
    let mut measure = Measure::new(timeline, None);
    measure.rows = Some(rows);
    operate(measure).map(|m| Some((m.name?, m.row?.1 - m.top)))
}
pub fn visible_at_tail(timeline: Id, event: &str) -> Task<bool> {
    operate(Measure::new(
        timeline,
        Some(Id::from(format!("post-{event}"))),
    ))
    .map(|m| m.visible && (m.max - m.top).abs() <= 1.)
}
pub fn restore_row(timeline: Id, mut b: Bookmark) -> Task<crate::Msg> {
    b.tail = false;
    restore(timeline, b)
}

pub fn restore(timeline: Id, b: Bookmark) -> Task<crate::Msg> {
    operate(Measure::new(
        timeline.clone(),
        b.row.as_ref().map(|(id, _)| id.clone()),
    ))
    .then(move |m| {
        let top = b
            .row
            .as_ref()
            .zip(m.row)
            .map_or(b.top, |((_, dy), (_, y))| y - dy);
        operation::scroll_to(
            timeline.clone(),
            operation::AbsoluteOffset {
                x: None,
                y: Some(if b.tail {
                    0.
                } else {
                    (m.max - top).clamp(0., m.max)
                }),
            },
        )
    })
}
