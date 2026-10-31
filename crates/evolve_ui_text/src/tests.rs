use crate::{fonts, TextAlign, TextStyle, TextSystem};
use evolve_ui_core::{FontId, PathCmd, UiPath};

fn style(size: f32) -> TextStyle {
    TextStyle { size, ..TextStyle::default() }
}

fn families(system: &TextSystem, runs: &[evolve_ui_core::TextRun]) -> Vec<String> {
    runs.iter().map(|r| system.family_of(r.font).expect("a known font")).collect()
}

/// The box of a path's points (control points included, which is enough for a sanity check).
fn bounds(path: &UiPath) -> (f32, f32, f32, f32) {
    let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    let mut add = |p: &evolve_ui_core::Point| {
        b = (b.0.min(p.x), b.1.min(p.y), b.2.max(p.x), b.3.max(p.y));
    };
    for cmd in &path.cmds {
        match cmd {
            PathCmd::MoveTo(p) | PathCmd::LineTo(p) => add(p),
            PathCmd::QuadTo(a, p) => {
                add(a);
                add(p);
            }
            PathCmd::CubicTo(a, c, p) => {
                add(a);
                add(c);
                add(p);
            }
            PathCmd::Close => {}
        }
    }
    b
}

#[test]
fn the_default_fonts_are_the_only_fonts() {
    let system = TextSystem::new();
    for family in [fonts::SANS, fonts::ARABIC, fonts::JAPANESE] {
        assert!(system.has_family(family), "{family} is missing");
    }
    // Nothing from the machine: the fonts of the program, and a missing family stays missing.
    assert!(!system.has_family("Segoe UI") && !system.has_family("Arial"), "a font of the machine was loaded");
}

#[test]
fn italian_text_with_accents_is_one_run_of_the_sans_font() {
    let mut system = TextSystem::new();
    let layout = system.layout("Perché è già così? Città, più, può.", &style(16.0), 1.0);
    assert_eq!(layout.lines, 1);
    assert_eq!(families(&system, &layout.runs), ["Inter"], "accented letters are in Inter itself, no fallback");
    let glyphs: usize = layout.runs.iter().map(|r| r.glyphs.len()).sum();
    assert_eq!(glyphs, "Perché è già così? Città, più, può.".chars().count(), "one glyph per letter, no ligature in this text");
    assert!(layout.width > 100.0 && layout.height > 16.0, "{} x {}", layout.width, layout.height);
    // The pen moves to the right, on one baseline.
    let run = &layout.runs[0];
    assert!(run.glyphs.windows(2).all(|w| w[1].x > w[0].x && w[1].y == w[0].y));
}

#[test]
fn arabic_comes_from_the_arabic_font_and_runs_right_to_left() {
    let mut system = TextSystem::new();
    let layout = system.layout("مرحبا بالعالم", &style(20.0), 1.0);
    // The letters come from the Arabic font; the space between the words is a character of no script, so the default font
    // draws it (it has no outline).
    let names = families(&system, &layout.runs);
    assert_eq!(names, ["Noto Sans Arabic", "Inter", "Noto Sans Arabic"], "{names:?}");
    assert_eq!(layout.runs[1].glyphs.len(), 1, "just the space");
    let glyphs: Vec<_> = layout.runs.iter().flat_map(|r| r.glyphs.iter()).collect();
    assert!(glyphs.len() >= 10, "{} glyphs", glyphs.len());
    // Glyphs come in reading order, and their positions run from the right: the first letter of the text is at the right end
    // of the line, the last one at the left end.
    // Contextual forms: Meem at the start of a word is not the glyph it is when alone.
    let alone = system.layout("م", &style(20.0), 1.0).runs[0].glyphs[0].id;
    assert_ne!(glyphs[0].id, alone, "the joined (initial) form is used");
    let rightmost = glyphs.iter().map(|g| g.x).fold(f32::MIN, f32::max);
    assert_eq!(glyphs[0].x, rightmost, "and it sits at the right end");
    assert!(glyphs.last().expect("glyphs").x < 1.0, "the last letter is at the left end");
    assert!(glyphs.iter().all(|g| g.id != 0), "no missing glyph");
}

#[test]
fn japanese_comes_from_the_japanese_font_and_a_mixed_line_changes_font() {
    let mut system = TextSystem::new();
    let japanese = system.layout("こんにちは、世界", &style(16.0), 1.0);
    assert_eq!(families(&system, &japanese.runs), ["Noto Sans JP"]);
    assert!(japanese.runs[0].glyphs.iter().all(|g| g.id != 0), "no missing glyph");

    let mixed = system.layout("Evolve 日本語 العربية", &style(16.0), 1.0);
    let names = families(&system, &mixed.runs);
    assert!(names.contains(&"Inter".to_string()) && names.contains(&"Noto Sans JP".to_string()) && names.contains(&"Noto Sans Arabic".to_string()), "{names:?}");
    let missing: usize = mixed.runs.iter().flat_map(|r| &r.glyphs).filter(|g| g.id == 0).count();
    assert_eq!(missing, 0, "every character found a font");
}

#[test]
fn a_narrow_width_breaks_lines_at_words() {
    let mut system = TextSystem::new();
    let text = "uno due tre quattro cinque sei sette otto nove dieci";
    let one = system.layout(text, &style(16.0), 1.0);
    let wrapped = system.layout(text, &TextStyle { max_width: Some(120.0), wrap: true, ..style(16.0) }, 1.0);
    assert_eq!(one.lines, 1);
    assert!(wrapped.lines >= 3, "{} lines", wrapped.lines);
    assert!(wrapped.width <= 120.0 + 0.5, "{}", wrapped.width);
    assert!((wrapped.height - wrapped.lines as f32 * 16.0 * 1.3).abs() < 0.5, "height is lines x line height");
    // No word is split: every line starts a word, so the glyph count equals the one-line count plus nothing.
    let glyphs = |l: &crate::TextLayout| l.runs.iter().map(|r| r.glyphs.len()).sum::<usize>();
    assert_eq!(glyphs(&wrapped), glyphs(&one) - (wrapped.lines - 1), "each break swallows exactly one space");
}

#[test]
fn a_text_too_long_for_its_width_ends_with_an_ellipsis() {
    let mut system = TextSystem::new();
    let text = "Questa è una riga molto più lunga dello spazio che ha";
    let free = system.layout(text, &style(16.0), 1.0);
    let cut = system.layout(text, &TextStyle { max_width: Some(150.0), ellipsis: true, ..style(16.0) }, 1.0);
    assert_eq!(cut.lines, 1);
    assert!(cut.width <= 150.5, "{}", cut.width);
    let glyphs = |l: &crate::TextLayout| l.runs.iter().map(|r| r.glyphs.len()).sum::<usize>();
    assert!(glyphs(&cut) < glyphs(&free));
    // The last glyph is the ellipsis: the glyph of U+2026 in Inter.
    let ellipsis = system.layout("\u{2026}", &style(16.0), 1.0).runs[0].glyphs[0].id;
    assert_eq!(cut.runs.last().and_then(|r| r.glyphs.last()).map(|g| g.id), Some(ellipsis));

    // The same text without the option is not cut.
    let plain = system.layout(text, &TextStyle { max_width: Some(150.0), ..style(16.0) }, 1.0);
    assert_eq!(glyphs(&plain), glyphs(&free));
}

#[test]
fn a_limit_of_lines_ends_the_last_line_with_an_ellipsis() {
    let mut system = TextSystem::new();
    let text = "uno due tre quattro cinque sei sette otto nove dieci undici dodici";
    let two = system.layout(text, &TextStyle { max_width: Some(100.0), wrap: true, max_lines: Some(2), ..style(16.0) }, 1.0);
    assert_eq!(two.lines, 2);
    let ellipsis = system.layout("\u{2026}", &style(16.0), 1.0).runs[0].glyphs[0].id;
    assert_eq!(two.runs.last().and_then(|r| r.glyphs.last()).map(|g| g.id), Some(ellipsis));
}

#[test]
fn alignment_moves_a_line_inside_its_width() {
    let mut system = TextSystem::new();
    let mut at = |align| {
        let l = system.layout("ciao", &TextStyle { max_width: Some(200.0), align, ..style(16.0) }, 1.0);
        (l.runs[0].glyphs[0].x, l.width)
    };
    let (left, w) = at(TextAlign::Left);
    let (center, _) = at(TextAlign::Center);
    let (right, _) = at(TextAlign::Right);
    assert!(left.abs() < 0.5);
    assert!((center - (200.0 - w) / 2.0).abs() < 1.0, "{center} {w}");
    assert!((right - (200.0 - w)).abs() < 1.0, "{right} {w}");
}

#[test]
fn bold_is_a_different_face_with_wider_letters() {
    let mut system = TextSystem::new();
    let regular = system.layout("Evolve", &style(16.0), 1.0);
    let bold = system.layout("Evolve", &TextStyle { weight: 700, ..style(16.0) }, 1.0);
    assert_ne!(regular.runs[0].font, bold.runs[0].font);
    assert!(bold.width > regular.width, "{} {}", bold.width, regular.width);
}

#[test]
fn the_size_in_logical_pixels_does_not_depend_on_the_scale() {
    let mut system = TextSystem::new();
    let text = "Evolve Quest, un gioco";
    let one = system.layout(text, &style(14.0), 1.0);
    let two = system.layout(text, &style(14.0), 2.0);
    assert!((one.width - two.width).abs() / one.width < 0.02, "{} {}", one.width, two.width);
    assert!((one.height - two.height).abs() < 0.5);
    assert_eq!(one.runs[0].size, 14.0);
    assert_eq!(two.runs[0].size, 14.0, "runs are in logical pixels at any scale");
    // A scale that is not a number is taken as 1.
    let odd = system.layout(text, &style(14.0), f32::NAN);
    assert_eq!(odd, one);
}

#[test]
fn empty_text_has_no_runs_and_no_width() {
    let mut system = TextSystem::new();
    let layout = system.layout("", &style(16.0), 1.0);
    assert!(layout.runs.is_empty() && layout.width == 0.0);
}

#[test]
fn a_glyph_outline_is_in_em_units_with_y_down_on_the_baseline() {
    let mut system = TextSystem::new();
    let layout = system.layout("H", &style(16.0), 1.0);
    let (font, glyph) = (layout.runs[0].font, layout.runs[0].glyphs[0].id);
    let path = system.outline(font, glyph).expect("H has an outline");
    let (min_x, min_y, max_x, max_y) = bounds(&path);
    // Inter's capital H: about 0.73 em tall, standing on the baseline (y = 0), growing upwards (negative y).
    assert!((-0.80..-0.65).contains(&min_y), "top {min_y}");
    assert!(max_y.abs() < 0.01, "bottom {max_y}");
    assert!(min_x >= 0.0 && max_x > 0.4 && max_x < 0.9, "x {min_x}..{max_x}");
    assert!(matches!(path.cmds.first(), Some(PathCmd::MoveTo(_))) && path.cmds.contains(&PathCmd::Close));
}

#[test]
fn outlines_are_made_once_and_shared() {
    let mut system = TextSystem::new();
    let layout = system.layout("ee", &style(16.0), 1.0);
    let (font, glyph) = (layout.runs[0].font, layout.runs[0].glyphs[0].id);
    let a = system.outline(font, glyph).expect("e");
    let b = system.outline(font, glyph).expect("e again");
    assert!(std::sync::Arc::ptr_eq(&a, &b), "the second request is the cached one");
    assert_eq!(system.cached_outlines(), 1);
}

#[test]
fn a_space_has_no_outline_and_an_unknown_font_gives_none() {
    let mut system = TextSystem::new();
    let layout = system.layout("a b", &style(16.0), 1.0);
    let space = layout.runs[0].glyphs[1].id;
    assert!(system.outline(layout.runs[0].font, space).is_none());
    assert!(system.outline(FontId(999), 5).is_none());
}

#[test]
fn a_font_added_later_can_be_asked_for_by_name() {
    let mut system = TextSystem::new();
    assert!(!system.has_family("Noto Sans JP2"));
    // A broken font is ignored, not a crash.
    system.add_font(vec![1, 2, 3]);
    let layout = system.layout("ciao", &TextStyle { family: Some("Does Not Exist".into()), ..style(16.0) }, 1.0);
    assert!(!layout.runs.is_empty(), "an unknown family falls back to the fonts we have");
}

#[test]
fn snapping_puts_letters_and_baseline_on_whole_physical_pixels() {
    let mut system = TextSystem::new();
    for scale in [1.0f32, 2.0, 1.5] {
        let layout = system.layout("Perché hello, world", &style(13.0), scale);
        for glyph in layout.runs.iter().flat_map(|r| r.glyphs.iter()) {
            for value in [glyph.x * scale, glyph.y * scale] {
                assert!((value - value.round()).abs() < 1e-3, "{value} is not on a pixel at scale {scale}");
            }
        }
    }
    // Without it, positions fall between pixels.
    let loose = system.layout("Perché hello, world", &TextStyle { snap: false, ..style(13.0) }, 1.0);
    assert!(loose.runs.iter().flat_map(|r| r.glyphs.iter()).any(|g| (g.x - g.x.round()).abs() > 0.05), "unsnapped text has fractional positions");
}

#[test]
fn combining_marks_are_placed_over_their_base_letter() {
    let mut system = TextSystem::new();
    // Arabic Beh with a Fatha above: the mark is a glyph of its own, moved by the font's mark positioning.
    let layout = system.layout("بَ", &style(20.0), 1.0);
    let glyphs: Vec<_> = layout.runs.iter().flat_map(|r| r.glyphs.iter()).collect();
    assert!(glyphs.len() >= 2, "{} glyphs", glyphs.len());
    assert!(glyphs.iter().any(|g| g.x > 1.0), "a mark is moved sideways from the start of the letter: {:?}", glyphs.iter().map(|g| g.x).collect::<Vec<_>>());
    let mut rows: Vec<i32> = glyphs.iter().map(|g| g.y.round() as i32).collect();
    rows.sort();
    rows.dedup();
    assert!(rows.len() >= 2, "and up or down from the baseline: {rows:?}");
}
