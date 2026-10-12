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
    ["LOCALAPPDATA", "XDG_DATA_HOME"]
        .into_iter()
        .find_map(std::env::var_os)
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("EvolveEngine")
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
