//! The text system: the fonts, the shaper and the glyph cache.

use crate::fonts;
use crate::outline;
use cosmic_text::{Align, Attrs, Buffer, Ellipsize, EllipsizeHeightLimit, Family, FontSystem, Hinting, Metrics, Shaping, Weight, Wrap};
use cosmic_text::{fontdb, Fallback};
use evolve_ui_core::{FontId, GlyphPos, TextAlign, TextLayout, TextRun, TextStyle, Typesetter, UiPath};
use std::collections::HashMap;
use std::sync::Arc;
use swash::scale::ScaleContext;
use unicode_script::Script;

/// Which of our fonts covers what the family asked for does not. Only fonts of ours are named, so nothing outside the
/// program can answer.
struct Fallbacks;

impl Fallback for Fallbacks {
    fn common_fallback(&self) -> &[&'static str] {
        &[fonts::SANS]
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        &[]
    }

    fn script_fallback(&self, script: Script, _locale: &str) -> &[&'static str] {
        match script {
            Script::Arabic => &[fonts::ARABIC],
            Script::Han | Script::Hiragana | Script::Katakana => &[fonts::JAPANESE],
            _ => &[],
        }
    }
}

pub struct TextSystem {
    fonts: FontSystem,
    /// `FontId(n)` is `faces[n]`: a face of the database at a weight.
    faces: Vec<(fontdb::ID, Weight)>,
    ids: HashMap<(fontdb::ID, Weight), FontId>,
    scale_context: ScaleContext,
    /// `None` is remembered too: a space has no outline.
    outlines: HashMap<(FontId, u16), Option<Arc<UiPath>>>,
}

impl Default for TextSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl Typesetter for TextSystem {
    fn layout(&mut self, text: &str, style: &TextStyle, scale: f32) -> TextLayout {
        TextSystem::layout(self, text, style, scale)
    }
}

impl TextSystem {
    /// A system with the default fonts and nothing else.
    pub fn new() -> Self {
        Self::with_fonts(fonts::default_sources())
    }

    /// A system with exactly these fonts. The machine's own fonts are never loaded. Characters no font here has are drawn
    /// as that font's "missing glyph" box, or not at all.
    pub fn with_fonts(sources: impl IntoIterator<Item = fontdb::Source>) -> Self {
        let mut db = fontdb::Database::new();
        for source in sources {
            db.load_font_source(source);
        }
        db.set_sans_serif_family(fonts::SANS);
        let fonts = FontSystem::new_with_locale_and_db_and_fallback("en-US".to_string(), db, Fallbacks);
        Self { fonts, faces: Vec::new(), ids: HashMap::new(), scale_context: ScaleContext::new(), outlines: HashMap::new() }
    }

    /// Adds a font (TrueType or OpenType data). Its families can then be named in a [`TextStyle`].
    pub fn add_font(&mut self, data: Vec<u8>) {
        self.fonts.db_mut().load_font_data(data);
    }

    /// The family name of a font of this system.
    pub fn family_of(&self, font: FontId) -> Option<String> {
        let (id, _) = *self.faces.get(font.0 as usize)?;
        self.fonts.db().face(id)?.families.first().map(|(name, _)| name.clone())
    }

    /// Whether some font of this system is of the family `name`.
    pub fn has_family(&self, name: &str) -> bool {
        self.fonts.db().faces().any(|face| face.families.iter().any(|(family, _)| family == name))
    }

    fn font_id(&mut self, id: fontdb::ID, weight: Weight) -> FontId {
        *self.ids.entry((id, weight)).or_insert_with(|| {
            self.faces.push((id, weight));
            FontId(self.faces.len() as u32 - 1)
        })
    }

    /// Shapes `text`. `scale` is the number of physical pixels per logical pixel (1 on an ordinary screen, 2 on a dense
    /// one): the text is laid out at its physical size, so that spacing is decided where pixels are, and the result is given
    /// back in logical pixels.
    pub fn layout(&mut self, text: &str, style: &TextStyle, scale: f32) -> TextLayout {
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let size = style.size.max(1.0) * scale;
        let line_height = (size * style.line_height.max(0.5)).max(1.0);
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, line_height));

        let limited = style.max_width.filter(|w| w.is_finite() && *w > 0.0).map(|w| w * scale);
        buffer.set_size(limited, None);
        let (wrap, ellipsize) = match (limited, style.wrap) {
            (None, _) => (Wrap::None, Ellipsize::None),
            (Some(_), true) => match style.max_lines.filter(|n| *n > 0) {
                Some(lines) => (Wrap::Word, Ellipsize::End(EllipsizeHeightLimit::Lines(lines))),
                None => (Wrap::Word, Ellipsize::None),
            },
            (Some(_), false) => (Wrap::None, if style.ellipsis { Ellipsize::End(EllipsizeHeightLimit::Lines(1)) } else { Ellipsize::None }),
        };
        buffer.set_hinting(if style.snap { Hinting::Enabled } else { Hinting::Disabled });
        buffer.set_wrap(wrap);
        buffer.set_ellipsize(ellipsize);

        let family = match &style.family {
            Some(name) => Family::Name(name),
            None => Family::Name(fonts::SANS),
        };
        let attrs = Attrs::new().family(family).weight(Weight(style.weight));
        let align = match style.align {
            TextAlign::Left => None,
            TextAlign::Center => Some(Align::Center),
            TextAlign::Right => Some(Align::Right),
        };
        buffer.set_text(text, &attrs, Shaping::Advanced, align);
        buffer.shape_until_scroll(&mut self.fonts, false);

        let mut runs: Vec<TextRun> = Vec::new();
        let (mut width, mut lines) = (0.0f32, 0usize);
        let mut bottom = 0.0f32;
        for line in buffer.layout_runs() {
            lines += 1;
            width = width.max(line.line_w);
            bottom = bottom.max(line.line_top + line.line_height);
            // Consecutive glyphs of one font make one run (the font changes where the fallback takes over).
            let mut current: Option<(fontdb::ID, Weight)> = None;
            for glyph in line.glyphs {
                let key = (glyph.font_id, glyph.font_weight);
                if current != Some(key) {
                    current = Some(key);
                    let font = self.font_id(key.0, key.1);
                    runs.push(TextRun { font, size: glyph.font_size / scale, color: style.color, glyphs: Vec::new() });
                }
                let x = glyph.x + glyph.x_offset * glyph.font_size;
                let y = line.line_y - glyph.y_offset * glyph.font_size;
                let y = if style.snap { y.round() } else { y };
                runs.last_mut().expect("a run was pushed above").glyphs.push(GlyphPos { id: glyph.glyph_id, x: x / scale, y: y / scale });
            }
        }
        TextLayout { runs, width: width / scale, height: bottom / scale, lines }
    }

    /// The outline of a glyph of a run: em units, y down, origin at the pen position on the baseline. Made on the first
    /// request and kept for good. `None` for a glyph that draws nothing.
    pub fn outline(&mut self, font: FontId, glyph: u16) -> Option<Arc<UiPath>> {
        if let Some(found) = self.outlines.get(&(font, glyph)) {
            return found.clone();
        }
        let made = self.make_outline(font, glyph).map(Arc::new);
        self.outlines.insert((font, glyph), made.clone());
        made
    }

    fn make_outline(&mut self, font: FontId, glyph: u16) -> Option<UiPath> {
        let (id, weight) = *self.faces.get(font.0 as usize)?;
        let face = self.fonts.get_font(id, weight)?;
        outline::glyph_path(&mut self.scale_context, face.as_swash(), glyph)
    }

    /// How many outlines are kept (for tests and the debug panel).
    pub fn cached_outlines(&self) -> usize {
        self.outlines.len()
    }
}
