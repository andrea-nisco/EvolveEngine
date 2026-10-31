//! The default fonts, inside the program. All are under the SIL Open Font License 1.1 (texts next to the files, in `fonts/`).
//! They are subsets, made by `tools/build-ui-fonts.py`; see the notes there.

use cosmic_text::fontdb::Source;
use std::sync::Arc;

/// Latin, Greek, Cyrillic and common symbols. The default family of every text.
pub const SANS: &str = "Inter";
pub const ARABIC: &str = "Noto Sans Arabic";
/// Japanese kana, punctuation and the kanji of JIS X 0208 (also used for Chinese characters of any language).
pub const JAPANESE: &str = "Noto Sans JP";

const INTER_REGULAR: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");
const INTER_BOLD: &[u8] = include_bytes!("../fonts/Inter-Bold.ttf");
const NOTO_ARABIC: &[u8] = include_bytes!("../fonts/NotoSansArabic-Regular.ttf");
const NOTO_JAPANESE: &[u8] = include_bytes!("../fonts/NotoSansJP-Regular.ttf");

/// The licence texts, for an "about" page of a game that ships these fonts.
pub const LICENSES: [(&str, &str); 3] = [
    ("Inter", include_str!("../fonts/OFL-Inter.txt")),
    ("Noto Sans Arabic", include_str!("../fonts/OFL-NotoSansArabic.txt")),
    ("Noto Sans JP", include_str!("../fonts/OFL-NotoSansJP.txt")),
];

/// The font data of the default families, ready for a font database.
pub fn default_sources() -> Vec<Source> {
    [INTER_REGULAR, INTER_BOLD, NOTO_ARABIC, NOTO_JAPANESE].into_iter().map(|data| Source::Binary(Arc::new(data))).collect()
}
