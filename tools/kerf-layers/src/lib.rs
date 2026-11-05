//! The rule that keeps Kerf's areas apart, read from where a crate lives:
//! `crates/<folder>/<crate>`. See `docs/architecture.md`.
//!
//! A crate may depend on crates in its own folder and in any lower layer.
//! Domains share a layer and may not depend on each other, so one can be
//! rewritten without touching the rest; what several domains need moves
//! down to `render` or `foundation`. Only `engine` and `surfaces` see them
//! all. The rule holds for every kind of dependency, tests' included.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// A folder's layer, or `None` for a folder this table does not know yet.
pub fn layer(folder: &str) -> Option<u8> {
    match folder {
        "foundation" => Some(0),
        "render" => Some(1),
        "edit" | "effects" | "color" | "motion" | "vfx" | "image" => Some(2),
        "engine" => Some(3),
        "surfaces" => Some(4),
        _ => None,
    }
}

/// Whether a crate in `from` may depend on a crate in `to`.
pub fn allowed(from: &str, to: &str) -> bool {
    from == to || matches!((layer(from), layer(to)), (Some(f), Some(t)) if t < f)
}

/// The crates a manifest names by path.
pub fn path_dependencies(manifest: &str) -> Vec<String> {
    manifest
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| {
            let (name, value) = line.split_once('=')?;
            let path = value.split("path = \"").nth(1)?.split('"').next()?;
            path.starts_with("..").then(|| name.trim().to_owned())
        })
        .collect()
}

/// Every crate under `crates`, by name, with its folder and manifest.
pub fn workspace(crates: &Path) -> std::io::Result<BTreeMap<String, (String, String)>> {
    let mut found = BTreeMap::new();
    for folder in fs::read_dir(crates)? {
        let folder = folder?;
        if !folder.file_type()?.is_dir() {
            continue;
        }
        let area = folder.file_name().to_string_lossy().into_owned();
        for package in fs::read_dir(folder.path())? {
            let package = package?;
            let manifest = package.path().join("Cargo.toml");
            if manifest.is_file() {
                found.insert(
                    package.file_name().to_string_lossy().into_owned(),
                    (area.clone(), fs::read_to_string(manifest)?),
                );
            }
        }
    }
    Ok(found)
}

/// Every break of the rule, in words.
pub fn violations(crates: &BTreeMap<String, (String, String)>) -> Vec<String> {
    let mut found = Vec::new();
    for (name, (folder, manifest)) in crates {
        if layer(folder).is_none() {
            found.push(format!(
                "crates/{folder}/{name}: `{folder}` is not a known folder; give it a layer in kerf-layers"
            ));
        }
        for dependency in path_dependencies(manifest) {
            match crates.get(&dependency) {
                Some((to, _)) if !allowed(folder, to) => found.push(format!(
                    "{name} ({folder}) may not depend on {dependency} ({to})"
                )),
                Some(_) => {}
                None => found.push(format!("{name} names {dependency}, which is not a crate")),
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_workspace_keeps_its_layers() {
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates");
        let crates = workspace(&crates).expect("the crates folder reads");
        assert!(crates.len() > 20, "found {} crates", crates.len());
        let found = violations(&crates);
        assert!(found.is_empty(), "{}", found.join("\n"));
    }

    #[test]
    fn domains_are_apart_and_lower_layers_look_only_down() {
        assert!(allowed("motion", "render"));
        assert!(allowed("motion", "motion"));
        assert!(!allowed("motion", "vfx"));
        assert!(!allowed("render", "effects"));
        assert!(!allowed("foundation", "render"));
        assert!(allowed("engine", "image"));
        assert!(!allowed("image", "engine"));
        assert!(allowed("surfaces", "engine"));
        assert!(!allowed("somewhere", "foundation"));
    }

    #[test]
    fn a_break_is_named() {
        let crates = BTreeMap::from([
            (
                "kerf-a".to_owned(),
                (
                    "motion".to_owned(),
                    "[dependencies]\nkerf-b = { path = \"../../vfx/kerf-b\" }\n# kerf-c = { path = \"../c\" }\nserde = \"1\"\n".to_owned(),
                ),
            ),
            ("kerf-b".to_owned(), ("vfx".to_owned(), String::new())),
        ]);
        assert_eq!(
            violations(&crates),
            ["kerf-a (motion) may not depend on kerf-b (vfx)"]
        );
    }
}
