//! A stamp of the code that draws frames, so that frames kept on disk are never read by a Kerf that
//! would draw them differently: any change to it starts a new folder.

use std::path::{Path, PathBuf};

fn files(folder: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, out);
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| matches!(extension, "rs" | "wgsl" | "tsv" | "cube" | "ttf"))
        {
            out.push(path);
        }
    }
}

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("cargo says where the crate is");
    let manifest = Path::new(&manifest);
    let mut found = Vec::new();
    for folder in [
        "../../foundation",
        "../../render",
        "../../effects",
        "../../color",
        "../../image",
        "../kerf-core/src",
        "src",
    ] {
        let folder = manifest.join(folder);
        println!("cargo:rerun-if-changed={}", folder.display());
        files(&folder, &mut found);
    }
    let lock = manifest.join("../../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock.display());
    found.push(lock);
    found.sort();
    let mut hash = 0xCBF2_9CE4_8422_2325_u64;
    for path in &found {
        for byte in std::fs::read(path).unwrap_or_default() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    println!("cargo:rustc-env=KERF_DRAW_STAMP={hash:016x}");
}
