// What the stamp of the code that draws frames is made of (`build.rs`), named once for the
// stamp and for the test that holds it to them (`tests/compose/frame_cache.rs`): both take
// this file in as it is.

/// The folders whose files are that code, from this crate's.
const STAMPED_FOLDERS: [&str; 7] = [
    "../../foundation",
    "../../render",
    "../../effects",
    "../../color",
    "../../image",
    "../kerf-core/src",
    "src",
];

/// The kinds of file in them that are that code: Rust, the kernels and what they are written
/// out with for OpenCL, the catalogs, the tables of a look, and the fonts text is drawn in.
const STAMPED_KINDS: [&str; 6] = ["rs", "wgsl", "cl", "tsv", "cube", "ttf"];
