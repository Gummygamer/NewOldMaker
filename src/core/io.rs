//! Project save/load. One pretty-printed JSON file per project.

use super::data::{ProjectData, FORMAT_VERSION};
use anyhow::{bail, Context, Result};
use std::path::Path;

pub fn save_project(project: &ProjectData, path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(project)?;
    std::fs::write(path, json).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

pub fn load_project(path: &Path) -> Result<ProjectData> {
    let json = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let project: ProjectData = serde_json::from_str(&json).context("parsing project JSON")?;
    if project.format_version > FORMAT_VERSION {
        bail!(
            "project format v{} is newer than this engine supports (v{})",
            project.format_version,
            FORMAT_VERSION
        );
    }
    Ok(project)
}

#[cfg(test)]
mod tests {
    use crate::core::defaults::default_project;

    #[test]
    fn project_roundtrips_through_json() {
        let p = default_project();
        let json = serde_json::to_string(&p).unwrap();
        let p2: crate::core::data::ProjectData = serde_json::from_str(&json).unwrap();
        assert_eq!(p.maps.len(), p2.maps.len());
        assert_eq!(p.maps[0].tiles.len(), p2.maps[0].tiles.len());
        assert_eq!(p.actors.len(), p2.actors.len());
        assert_eq!(p.skills.len(), p2.skills.len());
        let json2 = serde_json::to_string(&p2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn save_and_load_from_disk() {
        let p = default_project();
        let dir = std::env::temp_dir().join("nom_io_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.nom.json");
        super::save_project(&p, &path).unwrap();
        let loaded = super::load_project(&path).unwrap();
        assert_eq!(loaded.name, p.name);
        assert_eq!(loaded.maps[1].events.len(), p.maps[1].events.len());
        std::fs::remove_file(&path).ok();
    }
}
