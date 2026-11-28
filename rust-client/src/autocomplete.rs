//! Composer autocomplete, as on Discord: `@` lists the room's people, `#`
//! its channels and `:` emoji once two letters follow, in a popup above the
//! field. It works the same in the composer and while editing a message.
//!
//! The popup is worked out from the editor's text and cursor whenever it is
//! needed, so it can't go stale: it closes as soon as the word at the cursor
//! stops being a token. Only the selection, an Escape and the picks (pills,
//! which become mentions and links when the message is sent) are kept.
use crate::{
    App, Msg,
    emoji::{self, Found},
    moderation::{self, Member},
    pills::{self, Pill, Target},
};
use iced::{
    Task,
    widget::{
        operation,
        text_editor::{self, Action, Content, Cursor, Edit, Position},
    },
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

/// Rows the popup shows at most.
pub const ROWS: usize = 10;

/// The editor a popup belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    Composer,
    /// The inline editor of one of our messages.
    Edit,
}
impl Field {
    /// The editor's widget ID, to give it focus back after a click.
    pub fn id(self) -> &'static str {
        match self {
            Self::Composer => "composer",
            Self::Edit => "message-edit",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// `@`: people in the room.
    Person,
    /// `#`: channels.
    Room,
    /// `:`: emoji by shortcode.
    Emoji,
}

/// The word being completed: from its trigger character to the end of the
/// word around the cursor, in byte offsets of one line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub trigger: Trigger,
    pub line: usize,
    pub start: usize,
    pub end: usize,
    /// What follows the trigger, up to the cursor.
    pub query: String,
}

/// The token being typed at byte `cursor` of `text`, line `line`: a trigger
/// at the start of a word (so `a@b`, `12:30` and URLs never open a popup)
/// and what follows it. Emoji need two letters of a shortcode first.
pub fn token(text: &str, line: usize, cursor: usize) -> Option<Token> {
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    let before = &text[..cursor];
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8());
    let mut word = before[start..].chars();
    let trigger = match word.next()? {
        '@' => Trigger::Person,
        '#' => Trigger::Room,
        ':' => Trigger::Emoji,
        _ => return None,
    };
    let query = word.as_str();
    if trigger == Trigger::Emoji
        && (query.len() < 2
            || !query
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')))
    {
        return None;
    }
    let end = text[cursor..]
        .find(char::is_whitespace)
        .map_or(text.len(), |i| cursor + i);
    Some(Token {
        trigger,
        line,
        start,
        end,
        query: query.to_owned(),
    })
}

/// A shortcode just closed before byte `cursor`, as typing `:tada:` does:
/// where it starts, and its emoji.
pub fn closed_shortcode(text: &str, cursor: usize) -> Option<(usize, &'static str)> {
    let before = text.get(..cursor)?;
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8());
    let code = before[start..].strip_prefix(':')?.strip_suffix(':')?;
    if code.is_empty() || code.contains(':') {
        return None;
    }
    emoji::by_shortcode(code).map(|e| (start, e))
}

/// How well `query` matches a person or room: the name's start, a word's
/// start, the ID's start, then anywhere in either. `None` if not at all.
pub fn rank(query: &str, name: &str, id: &str) -> Option<u8> {
    if query.is_empty() {
        return Some(0);
    }
    let (q, name, id) = (query.to_lowercase(), name.to_lowercase(), id.to_lowercase());
    let bare = id.trim_start_matches(['@', '!', '#']);
    if name.starts_with(&q) {
        Some(0)
    } else if name
        .split(|c: char| c.is_whitespace() || matches!(c, '-' | '_' | '.'))
        .any(|w| w.starts_with(&q))
    {
        Some(1)
    } else if bare.starts_with(&q) || id.starts_with(&q) {
        Some(2)
    } else if name.contains(&q) || bare.contains(&q) {
        Some(3)
    } else {
        None
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Candidate {
    Person {
        id: String,
        name: String,
        avatar: Option<String>,
        /// What goes into the composer; names two members share get their
        /// Matrix ID, as Element disambiguates them.
        label: String,
    },
    Room {
        id: String,
        name: String,
        avatar: Option<String>,
        voice: bool,
    },
    Emoji(Found),
    /// A custom emoji from an image pack (emotes.rs).
    Emote {
        shortcode: String,
        url: String,
        pack: String,
    },
}

/// An open popup: its token, matches and selected row.
pub struct Popup {
    pub field: Field,
    pub token: Token,
    pub candidates: Vec<Candidate>,
    pub selected: usize,
}

/// Popup keys and clicks, and the composer's emoji picker.
#[derive(Clone, Debug)]
pub enum Input {
    Up,
    Down,
    /// Tab or Enter: insert the selected row.
    Accept,
    /// A click on a row.
    Pick(usize),
    /// Escape: close this token's popup.
    Close,
    /// The composer's emoji button.
    TogglePicker,
    PickerSearch(String),
    /// Insert an emoji from the picker at the composer's cursor.
    PickEmoji(String),
}

#[derive(Default)]
pub struct State {
    /// The editor typed in last; the popup follows its cursor.
    field: Option<Field>,
    selected: usize,
    /// A token closed with Escape (field, line, start) stays closed until
    /// the cursor leaves it.
    dismissed: Option<(Field, usize, usize)>,
    /// Picks per editor: the composer's by room, an edit's by event.
    pills: BTreeMap<(Field, String), Vec<Pill>>,
    /// Each room's membership from its state when it opened.
    rosters: BTreeMap<String, BTreeMap<String, Member>>,
    /// The composer's emoji picker and its search text, while open.
    pub picker: Option<String>,
}

impl State {
    /// The picks to send with an editor's text: the composer's for `key` =
    /// room, an edit's for `key` = event.
    pub fn pills(&self, field: Field, key: &str) -> Vec<Pill> {
        self.pills
            .get(&(field, key.to_owned()))
            .cloned()
            .unwrap_or_default()
    }
    /// A room's members, read when it opened.
    pub fn learn_roster(&mut self, room: &str, members: &BTreeMap<String, Member>) {
        self.rosters.insert(room.to_owned(), members.clone());
    }
}

/// Select `start..end` of `line` and type `with` over it.
fn replace(content: &mut Content, line: usize, start: usize, end: usize, with: &str) {
    content.move_to(Cursor {
        position: Position { line, column: end },
        selection: Some(Position {
            line,
            column: start,
        }),
    });
    content.perform(Action::Edit(Edit::Paste(Arc::new(with.to_owned()))));
}

impl App {
    /// The editor of `field` if it's on screen, and the key its picks are
    /// kept under.
    fn completion_editor(&self, field: Field) -> Option<(&Content, String)> {
        match field {
            Field::Composer => (!self.account && !self.selected.is_empty())
                .then(|| (&self.editor, self.selected.clone())),
            Field::Edit => self
                .editing
                .as_ref()
                .filter(|d| d.room == self.selected && d.pending.is_none())
                .map(|d| (&d.input, d.event.clone())),
        }
    }
    fn completion_editor_mut(&mut self, field: Field) -> Option<&mut Content> {
        match field {
            Field::Composer => Some(&mut self.editor),
            Field::Edit => self
                .editing
                .as_mut()
                .filter(|d| d.pending.is_none())
                .map(|d| &mut d.input),
        }
    }

    /// The token at the cursor of `field`'s editor, Escape or not.
    fn raw_token(&self, field: Field) -> Option<Token> {
        let (content, _) = self.completion_editor(field)?;
        let cursor = content.cursor();
        if cursor.selection.is_some_and(|s| s != cursor.position) {
            return None;
        }
        let line = content.line(cursor.position.line)?;
        token(&line.text, cursor.position.line, cursor.position.column)
    }

    /// The popup to show, if any.
    pub fn completion(&self) -> Option<Popup> {
        let field = self.completion.field?;
        let token = self.raw_token(field)?;
        if self.completion.dismissed == Some((field, token.line, token.start)) {
            return None;
        }
        let candidates = self.candidates(&token);
        if candidates.is_empty() {
            return None;
        }
        Some(Popup {
            field,
            selected: self.completion.selected.min(candidates.len() - 1),
            token,
            candidates,
        })
    }

    fn candidates(&self, token: &Token) -> Vec<Candidate> {
        match token.trigger {
            Trigger::Person => self.people(&token.query),
            Trigger::Room => self.channels(&token.query),
            // The room's custom emoji first, as on Discord, then Unicode's.
            Trigger::Emoji => {
                let mut out: Vec<Candidate> = self
                    .emote_search(&self.selected, &token.query, ROWS)
                    .into_iter()
                    .map(|(pack, image)| Candidate::Emote {
                        shortcode: image.shortcode.clone(),
                        url: image.url.clone(),
                        pack: pack.name.clone(),
                    })
                    .collect();
                let left = ROWS.saturating_sub(out.len()).max(ROWS / 2);
                out.extend(
                    emoji::search(&token.query, left)
                        .into_iter()
                        .map(Candidate::Emoji),
                );
                out.truncate(ROWS);
                out
            }
        }
    }

    /// The open room's members matching `query`, best match first, then
    /// whoever wrote most recently. Everyone joined or invited by the room's
    /// state, and anyone who wrote since it was read; not ourselves.
    fn people(&self, query: &str) -> Vec<Candidate> {
        let Some(room) = self.rooms().list.get(&self.selected) else {
            return Vec::new();
        };
        let roster = self.completion.rosters.get(&self.selected);
        let mut people: BTreeMap<&str, (&str, Option<&str>)> = BTreeMap::new();
        for (id, m) in roster.into_iter().flatten() {
            if matches!(m.kind, moderation::Kind::Joined | moderation::Kind::Invited) {
                people.insert(id, (&m.name, m.avatar.as_deref()));
            }
        }
        for (id, name) in &room.members {
            if roster.is_none_or(|r| !r.contains_key(id)) {
                people.insert(id, (name, None));
            }
        }
        people.remove(self.matrix_id.as_str());
        let recent: HashMap<&str, usize> = room
            .posts()
            .enumerate()
            .map(|(i, p)| (p.sender.as_str(), i))
            .collect();
        let named = |id: &str, name: &str| {
            if name.trim().is_empty() {
                crate::rooms::short(id).to_owned()
            } else {
                name.to_owned()
            }
        };
        let mut seen: HashMap<String, usize> = HashMap::new();
        for (id, (name, _)) in &people {
            *seen.entry(named(id, name)).or_default() += 1;
        }
        let mut found: Vec<(u8, std::cmp::Reverse<Option<usize>>, String, Candidate)> = people
            .into_iter()
            .filter_map(|(id, (name, avatar))| {
                let name = named(id, name);
                let r = rank(query, &name, id)?;
                let label_name = if seen.get(&name).is_some_and(|n| *n > 1) {
                    format!("{name} ({id})")
                } else {
                    name.clone()
                };
                Some((
                    r,
                    std::cmp::Reverse(recent.get(id).copied()),
                    name.to_lowercase(),
                    Candidate::Person {
                        id: id.to_owned(),
                        avatar: avatar
                            .map(str::to_owned)
                            .or_else(|| room.avatars.get(id).cloned()),
                        label: format!("@{label_name}"),
                        name,
                    },
                ))
            })
            .collect();
        found.sort_by(|a, b| (a.0, &a.1, &a.2).cmp(&(b.0, &b.1, &b.2)));
        found.into_iter().take(ROWS).map(|(.., c)| c).collect()
    }

    /// Joined channels matching `query`: rooms, not spaces or DMs.
    fn channels(&self, query: &str) -> Vec<Candidate> {
        let mut found: Vec<(u8, String, Candidate)> = self
            .rooms()
            .list
            .iter()
            .filter(|(_, r)| !r.space && !r.dm)
            .filter_map(|(id, r)| {
                let name = if r.name.is_empty() { id } else { &r.name };
                Some((
                    rank(query, name, id)?,
                    name.to_lowercase(),
                    Candidate::Room {
                        id: id.clone(),
                        name: name.clone(),
                        avatar: r.avatar.clone(),
                        voice: r.voice,
                    },
                ))
            })
            .collect();
        found.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        found.into_iter().take(ROWS).map(|(.., c)| c).collect()
    }

    /// The pictures of the people in an open popup, to fetch.
    pub fn completion_avatars(&self) -> Vec<&str> {
        let Some(p) = self.completion() else {
            return Vec::new();
        };
        let room = self.rooms().list.get(&self.selected);
        let roster = self.completion.rosters.get(&self.selected);
        p.candidates
            .iter()
            .filter_map(|c| match c {
                Candidate::Person { id, .. } => roster
                    .and_then(|r| r.get(id)?.avatar.as_deref())
                    .or_else(|| room?.avatars.get(id).map(String::as_str)),
                _ => None,
            })
            .collect()
    }

    /// The composer's editor action: perform it, then follow the cursor.
    pub fn draft(&mut self, action: text_editor::Action) {
        let typed = typed(&action);
        self.editor.perform(action);
        self.edited(Field::Composer, typed);
    }

    /// After any action in an editor: the popup follows that editor, a new
    /// query starts at the top, and a closed shortcode becomes its emoji.
    pub fn edited(&mut self, field: Field, typed: Option<char>) {
        self.completion.field = Some(field);
        self.completion.selected = 0;
        let token = self.raw_token(field);
        if self
            .completion
            .dismissed
            .is_some_and(|d| token.as_ref().is_none_or(|t| d != (field, t.line, t.start)))
        {
            self.completion.dismissed = None;
        }
        if typed == Some(':') {
            self.close_shortcode(field);
        }
        // Forget picks whose label is gone, so typing it later is just text.
        if let Some((content, key)) = self.completion_editor(field) {
            let text = content.text();
            if let Some(pills) = self.completion.pills.get_mut(&(field, key)) {
                pills.retain(|p| text.contains(&p.label));
            }
        }
    }

    fn close_shortcode(&mut self, field: Field) {
        let Some(content) = self.completion_editor_mut(field) else {
            return;
        };
        let cursor = content.cursor().position;
        let Some(line) = content.line(cursor.line) else {
            return;
        };
        if let Some((start, emoji)) = closed_shortcode(&line.text, cursor.column) {
            replace(content, cursor.line, start, cursor.column, emoji);
        }
    }

    pub fn autocomplete(&mut self, input: Input) -> Task<Msg> {
        match input {
            Input::Up | Input::Down => {
                if let Some(p) = self.completion() {
                    let n = p.candidates.len();
                    self.completion.selected = if matches!(input, Input::Up) {
                        (p.selected + n - 1) % n
                    } else {
                        (p.selected + 1) % n
                    };
                }
            }
            Input::Accept => return self.insert(None),
            Input::Pick(i) => return self.insert(Some(i)),
            Input::Close => {
                if let Some(p) = self.completion() {
                    self.completion.dismissed = Some((p.field, p.token.line, p.token.start));
                }
            }
            Input::TogglePicker => {
                if self.completion.picker.take().is_none() {
                    self.completion.picker = Some(String::new());
                    return operation::focus("emoji-search");
                }
            }
            Input::PickerSearch(s) => {
                if let Some(search) = &mut self.completion.picker {
                    *search = s;
                }
            }
            Input::PickEmoji(e) => {
                self.completion.picker = None;
                self.editor.perform(Action::Edit(Edit::Paste(Arc::new(e))));
                self.edited(Field::Composer, None);
                return operation::focus(Field::Composer.id());
            }
        }
        Task::none()
    }

    /// Put row `index` (or the selected one) in place of the token: a
    /// person's or room's label, remembered as a pill, or an emoji.
    fn insert(&mut self, index: Option<usize>) -> Task<Msg> {
        let Some(p) = self.completion() else {
            return Task::none();
        };
        let Some(choice) = p.candidates.get(index.unwrap_or(p.selected)).cloned() else {
            return Task::none();
        };
        let Some((_, key)) = self.completion_editor(p.field) else {
            return Task::none();
        };
        let (text, pill) = match choice {
            Candidate::Person { id, label, .. } => {
                let name = label.trim_start_matches('@').to_owned();
                (
                    label.clone(),
                    Some(Pill {
                        label,
                        target: Target::User { id, name },
                    }),
                )
            }
            Candidate::Room { id, name, .. } => {
                let label = format!("#{name}");
                let via = self.via(&id);
                (
                    label.clone(),
                    Some(Pill {
                        label,
                        target: Target::Room { id, via },
                    }),
                )
            }
            Candidate::Emoji(f) => (f.emoji.to_owned(), None),
            Candidate::Emote { shortcode, url, .. } => {
                let label = format!(":{shortcode}:");
                (
                    label.clone(),
                    Some(Pill {
                        label,
                        target: Target::Emote { url },
                    }),
                )
            }
        };
        let Some(content) = self.completion_editor_mut(p.field) else {
            return Task::none();
        };
        let t = &p.token;
        let spaced = content
            .line(t.line)
            .is_some_and(|l| l.text[t.end..].starts_with(char::is_whitespace));
        let with = if spaced { text } else { format!("{text} ") };
        replace(content, t.line, t.start, t.end, &with);
        if spaced {
            content.perform(Action::Move(text_editor::Motion::Right));
        }
        if let Some(pill) = pill {
            let pills = self.completion.pills.entry((p.field, key)).or_default();
            pills.retain(|old| old.label != pill.label);
            pills.push(pill);
        }
        self.completion.selected = 0;
        operation::focus(p.field.id())
    }

    /// Mention `user` at the composer's cursor, as a menu's or profile's
    /// Mention action does: the same pill a pick from the popup makes.
    pub fn mention(&mut self, user: &str) -> Task<Msg> {
        let room = self.selected.clone();
        let Some(r) = self.rooms().list.get(&room) else {
            return Task::none();
        };
        let name = self
            .completion
            .rosters
            .get(&room)
            .and_then(|m| m.get(user))
            .map(|m| m.name.clone())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| r.display(user).to_owned());
        let label = format!("@{name}");
        self.editor
            .perform(Action::Edit(Edit::Paste(Arc::new(format!("{label} ")))));
        let pills = self
            .completion
            .pills
            .entry((Field::Composer, room))
            .or_default();
        pills.retain(|old| old.label != label);
        pills.push(Pill {
            label,
            target: Target::User {
                id: user.to_owned(),
                name,
            },
        });
        self.edited(Field::Composer, None);
        operation::focus(Field::Composer.id())
    }

    /// Servers that can route a visitor to `room`: ours, and the room's
    /// own if its ID names one.
    fn via(&self, room: &str) -> Vec<String> {
        let mut via: Vec<String> = pills::server(&self.matrix_id)
            .into_iter()
            .map(str::to_owned)
            .collect();
        if let Some(s) = pills::server(room)
            && !via.iter().any(|v| v == s)
        {
            via.push(s.to_owned());
        }
        via
    }
}

/// The character an action types, if it types one.
pub fn typed(action: &text_editor::Action) -> Option<char> {
    match action {
        Action::Edit(Edit::Insert(c)) => Some(*c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::moderation::Kind;

    #[test]
    fn a_trigger_at_a_word_start_opens_a_token() {
        let t = token("hi @mi", 0, 6).unwrap();
        assert_eq!(
            (t.trigger, t.start, t.end, t.query.as_str()),
            (Trigger::Person, 3, 6, "mi")
        );
        let t = token("#", 2, 1).unwrap();
        assert_eq!(
            (t.trigger, t.line, t.query.as_str()),
            (Trigger::Room, 2, "")
        );
        assert_eq!(
            token("@", 0, 1).unwrap().query,
            "",
            "a bare @ lists everyone"
        );
        // The word runs on past the cursor, and all of it is replaced.
        let t = token("@mira! ok", 0, 3).unwrap();
        assert_eq!((t.start, t.end, t.query.as_str()), (0, 6, "mi"));
    }

    #[test]
    fn mid_word_triggers_times_and_urls_are_not_tokens() {
        assert_eq!(token("mail a@b", 0, 8), None);
        assert_eq!(token("at 12:30", 0, 8), None);
        assert_eq!(token("https://x.org/#top", 0, 18), None);
        assert_eq!(token("@mi ", 0, 4), None, "a space ends the token");
        assert_eq!(token("@mi", 0, 9), None, "a cursor past the end");
    }

    #[test]
    fn emoji_need_two_shortcode_letters() {
        assert_eq!(token(":s", 0, 2), None);
        assert_eq!(token("ok :)", 0, 5), None);
        let t = token("ok :sm", 0, 6).unwrap();
        assert_eq!((t.trigger, t.query.as_str()), (Trigger::Emoji, "sm"));
        assert_eq!(token(":+1", 0, 3).unwrap().query, "+1");
        assert_eq!(token(":tada:", 0, 6), None, "a closed shortcode");
    }

    #[test]
    fn offsets_are_bytes_and_survive_other_scripts() {
        let line = "héllo @mí";
        let t = token(line, 0, line.len()).unwrap();
        assert_eq!(&line[t.start..t.end], "@mí");
        assert_eq!(token(line, 0, 2), None, "not a character boundary");
    }

    #[test]
    fn closing_a_shortcode_gives_its_emoji() {
        assert_eq!(closed_shortcode("yay :tada:", 10), Some((4, "🎉")));
        assert_eq!(closed_shortcode(":nope_nope:", 11), None);
        assert_eq!(closed_shortcode("a::", 3), None);
        assert_eq!(closed_shortcode("12:30:", 6), None);
    }

    #[test]
    fn ranking_prefers_name_starts_then_words_then_ids() {
        assert_eq!(rank("mi", "Mira", "@mira:x"), Some(0));
        assert_eq!(rank("lo", "Mira Lopez", "@ml:x"), Some(1));
        assert_eq!(rank("ml", "Mira Lopez", "@ml:x"), Some(2));
        assert_eq!(rank("ope", "Mira Lopez", "@ml:x"), Some(3));
        assert_eq!(rank("zz", "Mira", "@mira:x"), None);
        assert_eq!(rank("", "Anyone", "@a:x"), Some(0));
        assert_eq!(rank("MI", "mira", "@m:x"), Some(0), "case-insensitive");
    }

    fn signed_in() -> App {
        let (tx, _rx) = async_channel::unbounded();
        let mut app = App::new(tx);
        app.logged_in = true;
        app.matrix_id = "@me:example.org".into();
        app.selected = "!room:example.org".into();
        let mut room = crate::rooms::Room {
            name: "general".into(),
            ..Default::default()
        };
        room.members
            .insert("@mira:example.org".into(), "Mira".into());
        room.members.insert("@me:example.org".into(), "Me".into());
        app.live.list.insert(app.selected.clone(), room);
        app.live.list.insert(
            "!design:example.org".into(),
            crate::rooms::Room {
                name: "design-lab".into(),
                ..Default::default()
            },
        );
        app.live.list.insert(
            "!dm:example.org".into(),
            crate::rooms::Room {
                name: "Mike".into(),
                dm: true,
                ..Default::default()
            },
        );
        app
    }

    fn typing(app: &mut App, text: &str) {
        for c in text.chars() {
            app.draft(Action::Edit(Edit::Insert(c)));
        }
    }

    #[test]
    fn people_come_from_the_roster_without_ourselves_or_leavers() {
        let mut app = signed_in();
        let member = |kind, name: &str| Member {
            kind,
            name: name.into(),
            avatar: Some(format!("mxc://x/{name}")),
            event: "$m".into(),
        };
        let roster = BTreeMap::from([
            ("@mira:example.org".to_owned(), member(Kind::Joined, "Mira")),
            (
                "@mike:example.org".to_owned(),
                member(Kind::Invited, "Mike"),
            ),
            ("@milo:example.org".to_owned(), member(Kind::Left, "Milo")),
            ("@me:example.org".to_owned(), member(Kind::Joined, "Me")),
        ]);
        app.completion.learn_roster(&app.selected.clone(), &roster);
        typing(&mut app, "@mi");
        let p = app.completion().expect("a popup");
        let ids: Vec<_> = p
            .candidates
            .iter()
            .map(|c| match c {
                Candidate::Person { id, .. } => id.as_str(),
                _ => panic!("people only"),
            })
            .collect();
        assert_eq!(ids, ["@mike:example.org", "@mira:example.org"]);
        assert_eq!(app.completion_avatars(), ["mxc://x/Mike", "mxc://x/Mira"]);
    }

    #[test]
    fn tab_inserts_the_mention_and_it_is_sent_as_a_pill() {
        let mut app = signed_in();
        typing(&mut app, "hey @mi");
        let p = app.completion().expect("a popup");
        assert_eq!(p.token.trigger, Trigger::Person);
        let _ = app.autocomplete(Input::Accept);
        assert_eq!(app.editor.text(), "hey @Mira ");
        assert!(app.completion().is_none(), "the token ended");
        typing(&mut app, "look");
        let pills = app.completion.pills(Field::Composer, &app.selected);
        let text = app.editor.text();
        let parts = pills::split(&text, &pills);
        assert_eq!(pills::plain(&parts), "hey Mira look");
        assert_eq!(pills::user_ids(&parts), ["@mira:example.org"]);
    }

    #[test]
    fn arrows_move_the_selection_and_wrap() {
        let mut app = signed_in();
        typing(&mut app, "#");
        let p = app.completion().unwrap();
        assert_eq!(p.candidates.len(), 2, "channels, not DMs");
        assert_eq!(p.selected, 0);
        let _ = app.autocomplete(Input::Up);
        assert_eq!(app.completion().unwrap().selected, 1);
        let _ = app.autocomplete(Input::Down);
        assert_eq!(app.completion().unwrap().selected, 0);
        typing(&mut app, "de");
        let p = app.completion().unwrap();
        assert!(
            matches!(&p.candidates[..], [Candidate::Room { name, .. }] if name == "design-lab"),
            "typing narrows the list"
        );
        let _ = app.autocomplete(Input::Pick(0));
        assert_eq!(app.editor.text(), "#design-lab ");
        let pills = app.completion.pills(Field::Composer, &app.selected);
        assert_eq!(
            pills[0].target,
            Target::Room {
                id: "!design:example.org".into(),
                via: vec!["example.org".into()],
            }
        );
    }

    #[test]
    fn escape_closes_until_the_token_changes() {
        let mut app = signed_in();
        typing(&mut app, "@m");
        let _ = app.autocomplete(Input::Close);
        assert!(app.completion().is_none());
        typing(&mut app, "i");
        assert!(app.completion().is_none(), "still the same token");
        typing(&mut app, " @");
        assert!(app.completion().is_some(), "a new token opens again");
    }

    #[test]
    fn emoji_insert_their_character_and_shortcodes_close_into_emoji() {
        let mut app = signed_in();
        typing(&mut app, "yay :tad");
        let p = app.completion().unwrap();
        assert!(matches!(p.candidates[0], Candidate::Emoji(f) if f.emoji == "🎉"));
        let _ = app.autocomplete(Input::Accept);
        assert_eq!(app.editor.text(), "yay 🎉 ");
        typing(&mut app, ":thumbsup:");
        assert_eq!(app.editor.text(), "yay 🎉 👍");
    }

    #[test]
    fn deleted_mentions_are_forgotten() {
        let mut app = signed_in();
        typing(&mut app, "@mi");
        let _ = app.autocomplete(Input::Accept);
        assert_eq!(
            app.completion.pills(Field::Composer, &app.selected).len(),
            1
        );
        for _ in 0..3 {
            app.draft(Action::Edit(Edit::Backspace));
        }
        assert!(
            app.completion
                .pills(Field::Composer, &app.selected)
                .is_empty()
        );
    }

    #[test]
    fn a_mention_action_inserts_the_same_pill() {
        let mut app = signed_in();
        typing(&mut app, "hi ");
        let _ = app.mention("@mira:example.org");
        typing(&mut app, "there");
        let text = app.editor.text();
        let pills = app.completion.pills(Field::Composer, &app.selected);
        let parts = pills::split(&text, &pills);
        assert_eq!(text, "hi @Mira there");
        assert_eq!(pills::user_ids(&parts), ["@mira:example.org"]);
    }

    #[test]
    fn the_picker_inserts_at_the_cursor() {
        let mut app = signed_in();
        typing(&mut app, "ab");
        app.draft(Action::Move(text_editor::Motion::Left));
        let _ = app.autocomplete(Input::TogglePicker);
        let _ = app.autocomplete(Input::PickEmoji("🎉".into()));
        assert_eq!(app.editor.text(), "a🎉b");
        assert!(app.completion.picker.is_none());
    }
}
