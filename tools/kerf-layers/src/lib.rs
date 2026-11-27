//! The rule that keeps Kerf's areas apart, read from where a crate lives:
//! `crates/<folder>/<crate>`.
//!
//! A crate may depend on crates in its own folder and in any lower layer.
//! Domains share a layer and may not depend on each other, so one can be
//! rewritten without touching the rest; what several domains need moves
//! down to `render` or `foundation`. Only `engine` and `surfaces` see them
//! all. The rule holds for every kind of dependency, tests' included.
//!
//! `surfaces` is the one folder whose crates keep an order among themselves
//! as well ([`SURFACES`]): it holds kerfd, and the programs built on kerfd.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use toml::{Table, Value};

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

/// The crates of `surfaces`, in rows: one may depend on those in the rows before its own, and
/// on none in its row or after.
///
/// kerfd and the services come first and know none of the others, so that none of them comes
/// to need what is built on it. Then what is built on one or two of them, then the MCP server,
/// which gathers those, and last the editor's bridge and the `kerf` command, which gather
/// them all: no other crate of `surfaces` is built on those two.
pub const SURFACES: [&[&str]; 4] = [
    &["kerf-cloud", "kerf-generate", "kerf-runtime", "kerf-skills"],
    &["kerf-agent", "kerf-commands", "kerf-shots"],
    &["kerf-mcp"],
    &["kerf-bridge", "kerf-cli"],
];

/// The row of [`SURFACES`] a crate is in.
pub fn surface_row(name: &str) -> Option<usize> {
    SURFACES.iter().position(|row| row.contains(&name))
}

/// The paths a manifest names its dependencies by, as written: under `[dependencies]`,
/// `[dev-dependencies]` and `[build-dependencies]`, for every target or for one, each on a
/// line or as a table of its own.
pub fn path_dependencies(manifest: &str) -> Result<Vec<String>, toml::de::Error> {
    let manifest: Table = manifest.parse()?;
    let for_one_target = manifest
        .get("target")
        .and_then(Value::as_table)
        .into_iter()
        .flat_map(|targets| targets.values().filter_map(Value::as_table));
    let mut found = Vec::new();
    for table in std::iter::once(&manifest).chain(for_one_target) {
        for kind in ["dependencies", "dev-dependencies", "build-dependencies"] {
            let named = table.get(kind).and_then(Value::as_table);
            for dependency in named.into_iter().flat_map(Table::values) {
                if let Some(path) = dependency.get("path").and_then(Value::as_str) {
                    found.push(path.to_owned());
                }
            }
        }
    }
    Ok(found)
}

/// The dependencies the workspace's own manifest gives by path, which a crate would take
/// with `x.workspace = true` and so name no path of its own for [`path_dependencies`] to
/// find. That table is for what comes from outside; a crate of Kerf is named where it is
/// needed.
pub fn inherited_paths(root: &str) -> Result<Vec<String>, toml::de::Error> {
    let root: Table = root.parse()?;
    let given = root
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(Value::as_table);
    Ok(given
        .into_iter()
        .flatten()
        .filter(|(_, dependency)| dependency.get("path").is_some())
        .map(|(name, _)| name.clone())
        .collect())
}

/// Where a path written in the manifest of `crates/<folder>/<name>` leads.
#[derive(Debug, PartialEq, Eq)]
enum Leads {
    /// Into the crate's own folder.
    Within,
    /// To `crates/<folder>/<name>`.
    Crate { folder: String, name: String },
    /// Anywhere else: out of `crates`, or to no crate's own folder.
    Elsewhere,
}

/// Follow `path` from `crates/<folder>/<name>` by its parts alone, as Cargo does, however it
/// is spelled (`./../x`, `src/../../x`, `../x/`).
fn leads(folder: &str, name: &str, path: &str) -> Leads {
    if Path::new(path).has_root() {
        return Leads::Elsewhere;
    }
    let mut at = vec!["crates", folder, name];
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                if at.pop().is_none() {
                    return Leads::Elsewhere;
                }
            }
            part => at.push(part),
        }
    }
    match at.as_slice() {
        ["crates", in_folder, in_name, ..] if *in_folder == folder && *in_name == name => {
            Leads::Within
        }
        ["crates", folder, name] => Leads::Crate {
            folder: (*folder).to_owned(),
            name: (*name).to_owned(),
        },
        _ => Leads::Elsewhere,
    }
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
        if folder == "surfaces" && surface_row(name).is_none() {
            found.push(format!(
                "{name} (surfaces) is in no row of SURFACES; give it one in kerf-layers"
            ));
        }
        let paths = match path_dependencies(manifest) {
            Ok(paths) => paths,
            Err(error) => {
                found.push(format!("{name}'s manifest does not read: {error}"));
                continue;
            }
        };
        for path in paths {
            let (to, dependency) = match leads(folder, name, &path) {
                Leads::Within => continue,
                Leads::Crate { folder, name }
                    if crates.get(&name).is_some_and(|c| c.0 == folder) =>
                {
                    (folder, name)
                }
                _ => {
                    found.push(format!("{name} names {path}, which is not a crate"));
                    continue;
                }
            };
            if !allowed(folder, &to) {
                found.push(format!(
                    "{name} ({folder}) may not depend on {dependency} ({to})"
                ));
            } else if folder == "surfaces"
                && to == "surfaces"
                && let (Some(from), Some(to)) = (surface_row(name), surface_row(&dependency))
                && to >= from
            {
                found.push(format!(
                    "{name} may not depend on {dependency}, which is not before it in SURFACES"
                ));
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
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let crates = workspace(&root.join("crates")).expect("the crates folder reads");
        assert!(crates.len() > 20, "found {} crates", crates.len());
        let mut found = violations(&crates);
        let rows = SURFACES.iter().copied().flatten();
        for name in rows.clone() {
            if !matches!(crates.get(*name), Some((folder, _)) if folder == "surfaces") {
                found.push(format!(
                    "SURFACES names {name}, which is not a crate of surfaces"
                ));
            }
            if rows.clone().filter(|other| *other == name).count() > 1 {
                found.push(format!("SURFACES names {name} in two places"));
            }
        }
        let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("the workspace reads");
        for name in inherited_paths(&manifest).expect("the workspace's manifest reads") {
            found.push(format!(
                "the workspace gives {name} by path; a crate that needs it names the path itself"
            ));
        }
        assert!(found.is_empty(), "{}", found.join("\n"));
    }

    #[test]
    fn a_path_is_found_however_its_manifest_writes_it() {
        let manifest = r#"
            [package]
            name = "kerf-x"

            [dependencies]
            serde = "1"
            kerf-model = { path = "../../foundation/kerf-model" }
            # kerf-lang = { path = "../../foundation/kerf-lang" }
            inside = { path = "vendor/inside" }

            [dependencies.kerf-engine]
            path = "../../engine/kerf-engine"

            [build-dependencies.drawing]
            package = "kerf-draw"
            path = "../../render/kerf-draw"

            [target.'cfg(windows)'.dev-dependencies]
            kerf-gpu = { path = "../../render/kerf-gpu" }

            [target.'cfg(unix)'.dependencies.kerf-fx]
            path = "../../effects/kerf-fx"
        "#;
        let mut found = path_dependencies(manifest).expect("the manifest reads");
        found.sort();
        assert_eq!(
            found,
            [
                "../../effects/kerf-fx",
                "../../engine/kerf-engine",
                "../../foundation/kerf-model",
                "../../render/kerf-draw",
                "../../render/kerf-gpu",
                "vendor/inside",
            ]
        );
    }

    #[test]
    fn a_path_the_workspace_gives_is_found() {
        let root = r#"
            [workspace]
            members = ["crates/foundation/kerf-model"]

            [workspace.dependencies]
            serde = { version = "1", features = ["derive"] }
            kerf-engine = { path = "crates/engine/kerf-engine" }

            [workspace.dependencies.kerf-gpu]
            path = "crates/render/kerf-gpu"
        "#;
        assert_eq!(
            inherited_paths(root).expect("the manifest reads"),
            ["kerf-engine", "kerf-gpu"]
        );
    }

    fn krate(name: &str, folder: &str, paths: &[&str]) -> (String, (String, String)) {
        let needs: String = paths
            .iter()
            .enumerate()
            .map(|(at, path)| format!("d{at} = {{ path = \"{path}\" }}\n"))
            .collect();
        (
            name.to_owned(),
            (folder.to_owned(), format!("[dependencies]\n{needs}")),
        )
    }

    #[test]
    fn a_surface_may_depend_only_on_those_before_it() {
        let crates = BTreeMap::from([
            krate("kerf-skills", "surfaces", &[]),
            krate("kerf-agent", "surfaces", &["../kerf-skills"]),
            // kerfd comes to need what is built on it.
            krate("kerf-runtime", "surfaces", &["../kerf-agent"]),
            krate(
                "kerf-bridge",
                "surfaces",
                &["../kerf-runtime", "../kerf-agent"],
            ),
            // One of the last row is built on the other.
            krate("kerf-cli", "surfaces", &["../kerf-bridge"]),
            krate("kerf-new", "surfaces", &["../kerf-skills"]),
        ]);
        assert_eq!(
            violations(&crates),
            [
                "kerf-cli may not depend on kerf-bridge, which is not before it in SURFACES",
                "kerf-new (surfaces) is in no row of SURFACES; give it one in kerf-layers",
                "kerf-runtime may not depend on kerf-agent, which is not before it in SURFACES",
            ]
        );
    }

    #[test]
    fn a_path_is_followed_however_it_is_spelled() {
        let crates = BTreeMap::from([
            krate("kerf-engine", "engine", &[]),
            krate(
                "kerf-model",
                "foundation",
                &[
                    // Its own folder, which the rule is not about.
                    "vendor/inside",
                    "../kerf-model/vendor/inside",
                    // The engine, four ways.
                    "../../engine/kerf-engine",
                    "./../../engine/kerf-engine/",
                    "src/../../../engine/kerf-engine",
                    "../../engine/kerf-engine/src/..",
                    // No crate: a folder of the same name elsewhere, a crate's inside, and
                    // paths that leave the workspace.
                    "../../../vendor/kerf-engine",
                    "../../engine/kerf-engine/src",
                    "../../../../elsewhere",
                    "/opt/kerf-engine",
                ],
            ),
        ]);
        assert_eq!(
            violations(&crates),
            [
                "kerf-model (foundation) may not depend on kerf-engine (engine)",
                "kerf-model (foundation) may not depend on kerf-engine (engine)",
                "kerf-model (foundation) may not depend on kerf-engine (engine)",
                "kerf-model (foundation) may not depend on kerf-engine (engine)",
                "kerf-model names ../../../vendor/kerf-engine, which is not a crate",
                "kerf-model names ../../engine/kerf-engine/src, which is not a crate",
                "kerf-model names ../../../../elsewhere, which is not a crate",
                "kerf-model names /opt/kerf-engine, which is not a crate",
            ]
        );
    }

    #[test]
    fn a_manifest_that_does_not_read_is_said() {
        let crates = BTreeMap::from([(
            "kerf-model".to_owned(),
            ("foundation".to_owned(), "[dependencies".to_owned()),
        )]);
        let found = violations(&crates);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with("kerf-model's manifest does not read"),
            "{found:?}"
        );
    }
}
