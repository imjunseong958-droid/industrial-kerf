//! The editor's version is the repository's.

#[test]
fn the_editor_has_the_repositorys_version() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let manifest = std::fs::read_to_string(root).expect("the repository's Cargo.toml");
    let package = manifest
        .split("[workspace.package]")
        .nth(1)
        .expect("a [workspace.package] table");
    let version = package
        .lines()
        .find_map(|line| line.trim().strip_prefix("version = "))
        .map(|value| value.trim().trim_matches('"'))
        .expect("its version");
    assert_eq!(env!("CARGO_PKG_VERSION"), version);
}
