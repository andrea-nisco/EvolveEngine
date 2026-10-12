//! Packaging a project for distribution.
//!
//! - **Desktop:** the player executable with the project appended to it, followed by a small
//!   trailer (`length`, then a magic marker). The player looks for that trailer in its own file.
//! - **Web:** a folder with `index.html` (the project embedded as JSON), the player's WebAssembly
//!   module and its JavaScript glue.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const MAGIC: &[u8; 8] = b"EVOLVEGA";
const TRAILER_LEN: u64 = 16; // u64 length + magic

/// Files the web player needs next to `index.html`.
pub const WEB_PLAYER_FILES: [&str; 2] = ["evolve_player.js", "evolve_player_bg.wasm"];
const WEB_TEMPLATE: &str = include_str!("../web/index.html.template");
const PROJECT_PLACEHOLDER: &str = "{{PROJECT_JSON}}";

/// The project JSON appended to the executable at `exe`, if there is one.
pub fn read_embedded(exe: &Path) -> Option<String> {
    let mut file = File::open(exe).ok()?;
    let total = file.metadata().ok()?.len();
    if total < TRAILER_LEN {
        return None;
    }
    file.seek(SeekFrom::Start(total - TRAILER_LEN)).ok()?;
    let mut trailer = [0u8; TRAILER_LEN as usize];
    file.read_exact(&mut trailer).ok()?;
    if &trailer[8..] != MAGIC {
        return None;
    }
    let length = u64::from_le_bytes(trailer[..8].try_into().ok()?);
    if length > total - TRAILER_LEN {
        return None;
    }
    file.seek(SeekFrom::Start(total - TRAILER_LEN - length)).ok()?;
    let mut json = vec![0u8; length as usize];
    file.read_exact(&mut json).ok()?;
    String::from_utf8(json).ok()
}

/// Writes `player` plus the project to `output`, a self-contained game executable.
pub fn export_desktop(player: &Path, project_json: &str, output: &Path) -> Result<()> {
    if read_embedded(player).is_some() {
        return Err("the base player already contains a project".into());
    }
    if fs::canonicalize(player).ok() == fs::canonicalize(output).ok() && output.exists() {
        return Err("the output must not overwrite the player itself".into());
    }
    fs::copy(player, output)?;
    let mut file = OpenOptions::new().append(true).open(output)?;
    file.write_all(project_json.as_bytes())?;
    file.write_all(&(project_json.len() as u64).to_le_bytes())?;
    file.write_all(MAGIC)?;
    Ok(())
}

/// Writes a playable web folder into `output_dir`, taking the compiled player from `player_dir`.
pub fn export_web(player_dir: &Path, project_json: &str, output_dir: &Path) -> Result<()> {
    for name in WEB_PLAYER_FILES {
        if !player_dir.join(name).is_file() {
            return Err(format!("{} is missing; run tools/build-web-player.ps1 first", player_dir.join(name).display()).into());
        }
    }
    fs::create_dir_all(output_dir)?;
    for name in WEB_PLAYER_FILES {
        fs::copy(player_dir.join(name), output_dir.join(name))?;
    }
    // "</" must not appear inside a script element; "<\/" is the same string in JSON.
    let embedded = project_json.replace("</", "<\\/");
    fs::write(output_dir.join("index.html"), WEB_TEMPLATE.replace(PROJECT_PLACEHOLDER, &embedded))?;
    Ok(())
}

// --- Finding the player files next to the editor ----------------------------------------------

/// The directories to search for player files: the running program's folder and its ancestors.
fn search_dirs() -> Vec<PathBuf> {
    let exe = std::env::current_exe().ok();
    exe.as_deref().and_then(Path::parent).map(|dir| dir.ancestors().map(Path::to_path_buf).collect()).unwrap_or_default()
}

/// The desktop player executable that belongs to this build of the editor.
pub fn find_player_exe() -> Option<PathBuf> {
    let name = if cfg!(windows) { "evolve_player.exe" } else { "evolve_player" };
    let own = std::env::current_exe().ok()?;
    let sibling = own.with_file_name(name);
    (sibling.is_file() && sibling != own).then_some(sibling)
}

/// The folder with the compiled web player (`tools/build-web-player.ps1` creates it).
pub fn find_web_player_dir() -> Option<PathBuf> {
    search_dirs().into_iter().flat_map(|dir| [dir.join("web-player"), dir.join("dist").join("web-player")]).find(|d| d.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("evolve_bundle_{}_{name}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_desktop_export_carries_its_project_and_still_starts_like_the_player() {
        let dir = temp("desktop");
        let player = dir.join("player.exe");
        fs::write(&player, b"MZ pretend this is an executable").unwrap();
        assert!(read_embedded(&player).is_none());

        let output = dir.join("game.exe");
        export_desktop(&player, "{\"hello\": \"wörld\"}", &output).unwrap();
        assert_eq!(read_embedded(&output).as_deref(), Some("{\"hello\": \"wörld\"}"));
        let bytes = fs::read(&output).unwrap();
        assert!(bytes.starts_with(b"MZ pretend"), "the player program comes first");
        assert!(export_desktop(&output, "{}", &dir.join("again.exe")).is_err(), "no double bundling");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn files_without_a_trailer_or_with_a_bad_length_are_not_projects() {
        let dir = temp("garbage");
        let file = dir.join("x.exe");
        fs::write(&file, b"short").unwrap();
        assert!(read_embedded(&file).is_none());
        let mut bogus = b"payload".to_vec();
        bogus.extend_from_slice(&u64::MAX.to_le_bytes());
        bogus.extend_from_slice(MAGIC);
        fs::write(&file, bogus).unwrap();
        assert!(read_embedded(&file).is_none());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_web_export_embeds_the_project_safely() {
        let dir = temp("web");
        let player = dir.join("player");
        fs::create_dir_all(&player).unwrap();
        for name in WEB_PLAYER_FILES {
            fs::write(player.join(name), b"x").unwrap();
        }
        let output = dir.join("out");
        export_web(&player, "{\"script\": \"</script><b>\"}", &output).unwrap();
        let html = fs::read_to_string(output.join("index.html")).unwrap();
        assert!(html.contains("<\\/script><b>"), "the closing tag is escaped");
        assert!(!html.contains("{{PROJECT_JSON}}"));
        assert!(output.join("evolve_player_bg.wasm").is_file());
        assert!(export_web(&dir.join("nowhere"), "{}", &dir.join("out2")).is_err());
        fs::remove_dir_all(dir).ok();
    }
}
