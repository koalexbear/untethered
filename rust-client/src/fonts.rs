//! Font matching for names, room headings, and avatar initials.
use iced::advanced::graphics::text::{
    cosmic_text::fontdb::{Database, Family, Query, Weight},
    font_system,
};

// cosmic-text 0.15 only tries named fallbacks at the exact requested weight.
// System emoji usually have just a regular face. For a bold nickname that
// means shaping the word against potentially thousands of unrelated fonts.
// Make the same emoji outlines available to that fast named-fallback pass.
fn register_bold_emoji(db: &mut Database, family: &str) {
    let Some(id) = db.query(&Query {
        families: &[Family::Name(family)],
        weight: Weight::BOLD,
        ..Query::default()
    }) else {
        return;
    };
    let mut face = db.face(id).unwrap().clone();
    if face.weight != Weight::BOLD {
        face.weight = Weight::BOLD;
        db.push_face_info(face);
    }
}

pub fn configure() {
    let mut system = font_system().write().expect("font system lock");
    let db = system.raw().db_mut();
    for family in ["Noto Color Emoji", "Segoe UI Emoji", "Apple Color Emoji"] {
        register_bold_emoji(db, family);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use iced::advanced::graphics::text::cosmic_text::{
        Attrs, FontSystem, fallback::FontFallbackIter,
    };

    #[test]
    fn bold_name_uses_named_emoji_fallback_before_the_system_wide_scan() {
        let mut db = Database::new();
        db.load_font_data(include_bytes!("../assets/LiberationSans-Bold.ttf").to_vec());
        db.load_font_data(include_bytes!("../assets/LiberationSans-Regular.ttf").to_vec());
        // Only the family/weight lookup is under test. Model a single-weight
        // emoji font using a fixture, independent of what fonts CI has installed.
        let mut emoji = db
            .faces()
            .find(|f| f.weight == Weight::NORMAL)
            .unwrap()
            .clone();
        emoji.families[0].0 = "Noto Color Emoji".into();
        db.push_face_info(emoji);
        register_bold_emoji(&mut db, "Noto Color Emoji");

        let mut fs = FontSystem::new_with_locale_and_db("en-US".into(), db);
        let family = Family::Name("Liberation Sans");
        let matches = fs.get_font_matches(&Attrs::new().family(family).weight(Weight::BOLD));
        let families = [&family];
        let mut fonts =
            FontFallbackIter::new(&mut fs, &matches, &families, &[], "Alex🙂", Weight::BOLD);
        let primary = fonts.next().unwrap().id();
        let fallback = fonts.next().unwrap().id();
        assert_eq!(
            fs.db().face(primary).unwrap().families[0].0,
            "Liberation Sans"
        );
        assert_eq!(
            fs.db().face(fallback).unwrap().families[0].0,
            "Noto Color Emoji"
        );
    }
}
