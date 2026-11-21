//! On Windows, gives kerf-editor.exe Kerf's icon (assets/icon/kerf-editor.rc).

fn main() {
    println!("cargo:rerun-if-changed=assets/icon/kerf-editor.rc");
    println!("cargo:rerun-if-changed=assets/icon/industrial-kerf.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/icon/kerf-editor.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("the icon resource compiles");
    }
}
