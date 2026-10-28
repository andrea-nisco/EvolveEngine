//! Crash recovery: while there are unsaved changes the editor writes a recovery copy every
//! few seconds. A clean exit (or a deliberate "don't save") deletes it, so finding one at
//! startup means the last session ended unexpectedly.

use evolve_engine::io;
use evolve_engine::project::Project;
use std::path::{Path, PathBuf};

/// How often the recovery copy is refreshed.
pub const INTERVAL_SECONDS: u64 = 30;

pub struct Recovery {
    pub project: Project,
    /// The file the work belonged to, if it had one.
    pub original: Option<PathBuf>,
}

/// Where recovery files live: the user's local data folder, or the temp folder.
pub fn default_dir() -> PathBuf {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    data_root(var("LOCALAPPDATA"), var("XDG_DATA_HOME"), var("HOME")).unwrap_or_else(std::env::temp_dir).join("EvolveEngine")
}

/// The per-user data folder: `%LOCALAPPDATA%` on Windows, `~/Library/Application Support` on macOS,
/// `$XDG_DATA_HOME` or `~/.local/share` on Linux.
fn data_root(local_app_data: Option<PathBuf>, xdg_data_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    local_app_data.or(xdg_data_home).or_else(|| {
        let home = home?;
        Some(if cfg!(target_os = "macos") { home.join("Library/Application Support") } else { home.join(".local/share") })
    })
}

pub fn write_in(dir: &Path, project: &Project, original: Option<&Path>) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    io::save_project(project, &dir.join("recovery.evolve"))?;
    let original_text = original.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    std::fs::write(dir.join("recovery.path"), original_text)?;
    Ok(())
}

pub fn read_in(dir: &Path) -> Option<Recovery> {
    let project = io::load_project(&dir.join("recovery.evolve")).ok()?;
    let original = std::fs::read_to_string(dir.join("recovery.path")).ok().filter(|t| !t.is_empty()).map(PathBuf::from);
    Some(Recovery { project, original })
}

pub fn clear_in(dir: &Path) {
    for name in ["recovery.evolve", "recovery.path"] {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

pub fn write(project: &Project, original: Option<&Path>) -> io::Result<()> {
    write_in(&default_dir(), project, original)
}

pub fn read() -> Option<Recovery> {
    read_in(&default_dir())
}

pub fn clear() {
    clear_in(&default_dir());
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_engine::geometry::Vec2;
    use evolve_engine::scene::Scene;

    #[test]
    fn the_data_folder_follows_each_platform_convention() {
        let p = |s: &str| Some(PathBuf::from(s));
        assert_eq!(data_root(p("/win"), p("/xdg"), p("/home/a")), p("/win"));
        assert_eq!(data_root(None, p("/xdg"), p("/home/a")), p("/xdg"));
        let from_home = data_root(None, None, p("/home/a")).unwrap();
        let expected = if cfg!(target_os = "macos") { "/home/a/Library/Application Support" } else { "/home/a/.local/share" };
        assert_eq!(from_home, PathBuf::from(expected));
        assert_eq!(data_root(None, None, None), None, "nothing known: the caller falls back to the temp folder");
    }

    #[test]
    fn a_recovery_copy_round_trips_and_can_be_cleared() {
        let dir = std::env::temp_dir().join(format!("evolve_recovery_test_{}", std::process::id()));
        assert!(read_in(&dir).is_none());

        let mut project = Project::single(Scene::with_camera(Vec2::new(321.0, 123.0)));
        project.add_scene();
        write_in(&dir, &project, Some(Path::new("C:/work/game.evolve"))).unwrap();
        let recovered = read_in(&dir).unwrap();
        assert_eq!(recovered.project, project);
        assert_eq!(recovered.original, Some(PathBuf::from("C:/work/game.evolve")));

        write_in(&dir, &project, None).unwrap();
        assert_eq!(read_in(&dir).unwrap().original, None);

        clear_in(&dir);
        assert!(read_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
