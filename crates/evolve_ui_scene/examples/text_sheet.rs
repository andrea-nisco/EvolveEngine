//! Phase 1 test bed: draws the text sheets of the acceptance criteria with the CPU renderer and writes them as PNG files.
//!
//! `cargo run -p evolve_ui_scene --release --example text_sheet -- <folder>`
//!
//! - `languages_1x.png`, `languages_2x.png` (and `_nosnap`: without pixel snapping, see `TextStyle::snap`): Italian with accents, Arabic and Japanese (fallback fonts), at 1x and 2x.
//! - `small_1x.png`, `small_2x.png`: the same sentence at 12, 14 and 16 px, regular and bold, dark on light and light on dark.
//! - `*_zoom.png`: a crop of the sheet enlarged 4x without smoothing, to judge single pixels.

use evolve_engine::tiny_skia::{Color, Pixmap};
use evolve_ui_core::{Color as UiColor, DrawList, Point, Rect};
use evolve_ui_scene::render_cpu;
use evolve_ui_text::{TextStyle, TextSystem};
use std::path::PathBuf;

const INK: UiColor = UiColor::rgb(24, 28, 36);
const PAPER: UiColor = UiColor::rgb(246, 247, 249);

struct Line {
    text: &'static str,
    size: f32,
    weight: u16,
    dark: bool,
}

const fn line(text: &'static str, size: f32) -> Line {
    Line { text, size, weight: 400, dark: false }
}

/// Lays out `lines` one under the other on a `width` x `height` logical sheet and renders it at `scale`.
fn sheet(system: &mut TextSystem, lines: &[Line], width: f32, height: f32, scale: f32, snap: bool) -> Pixmap {
    let mut list = DrawList::new();
    list.rect(Rect::new(0.0, 0.0, width, height), [0.0; 4], PAPER);
    let mut y = 10.0;
    for l in lines {
        let (ink, paper) = if l.dark { (UiColor::rgb(236, 238, 242), UiColor::rgb(30, 34, 42)) } else { (INK, PAPER) };
        let style = TextStyle { size: l.size, weight: l.weight, color: ink, snap, ..TextStyle::default() };
        let layout = system.layout(l.text, &style, scale);
        if l.dark {
            list.rect(Rect::new(0.0, y - 3.0, width, layout.height + 6.0), [0.0; 4], paper);
        }
        for run in layout.runs {
            list.text(Point::new(12.0, y), run);
        }
        y += layout.height + 8.0;
    }
    let mut pixmap = Pixmap::new((width * scale) as u32, (height * scale) as u32).expect("a sheet");
    pixmap.fill(Color::TRANSPARENT);
    render_cpu(&list, system, scale, &mut pixmap);
    pixmap
}

/// A crop of `pixmap`, enlarged `factor` times with the nearest pixel (no smoothing).
fn zoom(pixmap: &Pixmap, x: u32, y: u32, w: u32, h: u32, factor: u32) -> Pixmap {
    let (w, h) = (w.min(pixmap.width() - x), h.min(pixmap.height() - y));
    let mut out = Pixmap::new(w * factor, h * factor).expect("a crop");
    let (src, dst) = (pixmap.data(), out.data_mut());
    for oy in 0..h * factor {
        for ox in 0..w * factor {
            let s = (((y + oy / factor) * pixmap.width() + x + ox / factor) * 4) as usize;
            let d = ((oy * w * factor + ox) * 4) as usize;
            dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
    out
}

fn main() {
    let folder = PathBuf::from(std::env::args().nth(1).expect("usage: text_sheet <folder>"));
    std::fs::create_dir_all(&folder).expect("the folder");
    let mut system = TextSystem::new();

    let languages = [
        line("Perché è già così? Città, più, può. Àèéìòù ÀÈÉÌÒÙ", 18.0),
        line("Evolve Quest: la sfida dell'università è finita!", 14.0),
        line("مرحبا بالعالم، هذا اختبار للنص العربي", 18.0),
        line("こんにちは、世界。日本語のテキストです。", 18.0),
        line("Mixed: Evolve 日本語 العربية Ciao", 16.0),
    ];
    let small = [
        line("12 px  Perché è già così? The quick brown fox jumps over the lazy dog 0123456789", 12.0),
        line("14 px  Perché è già così? The quick brown fox jumps over the lazy dog 0123456789", 14.0),
        line("16 px  Perché è già così? The quick brown fox jumps over the lazy dog 0123456789", 16.0),
        Line { weight: 700, ..line("12 px bold  Perché è già così? The quick brown fox jumps over", 12.0) },
        Line { weight: 700, ..line("14 px bold  Perché è già così? The quick brown fox jumps over", 14.0) },
        Line { dark: true, ..line("12 px  light on dark: Perché è già così? The quick brown fox 0123456789", 12.0) },
        Line { dark: true, ..line("14 px  light on dark: Perché è già così? The quick brown fox 0123456789", 14.0) },
        Line { dark: true, ..line("16 px  light on dark: Perché è già così? The quick brown fox 0123456789", 16.0) },
        line("12 px  日本語のテキスト  مرحبا بالعالم", 12.0),
    ];
    for (name, lines, width, height) in [("languages", &languages[..], 520.0, 170.0), ("small", &small[..], 620.0, 250.0)] {
        for (scale, snap) in [(1.0f32, true), (1.0, false), (2.0, true), (2.0, false)] {
            let pixmap = sheet(&mut system, lines, width, height, scale, snap);
            let tag = format!("{}x{}", scale as u32, if snap { "" } else { "_nosnap" });
            pixmap.save_png(folder.join(format!("{name}_{tag}.png"))).expect("write the sheet");
            let crop = zoom(&pixmap, (10.0 * scale) as u32, (8.0 * scale) as u32, (190.0 * scale) as u32, (60.0 * scale) as u32, 4 / scale as u32);
            crop.save_png(folder.join(format!("{name}_{tag}_zoom.png"))).expect("write the crop");
        }
    }
    println!("written to {}", folder.display());
}
