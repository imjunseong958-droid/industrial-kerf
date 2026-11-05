//! The examples in docs/kerf-language.md stay valid.

use kerf_lang::{apply_script, parse_document};
use kerf_model::Catalog;

fn text_blocks(markdown: &str) -> Vec<&str> {
    markdown
        .split("```text\n")
        .skip(1)
        .map(|block| block.split("```").next().unwrap_or_default())
        .collect()
}

#[test]
fn the_language_guide_examples_parse_and_apply() {
    // A Windows checkout can give the guide CRLF line endings.
    let guide = include_str!("../../../../docs/kerf-language.md").replace("\r\n", "\n");
    let blocks = text_blocks(&guide);
    assert_eq!(blocks.len(), 2);
    let document =
        parse_document(blocks[0], Catalog::builtin()).unwrap_or_else(|e| panic!("{e:#?}"));
    assert!(document.warnings.is_empty(), "{:?}", document.warnings);
    let applied = apply_script(&document.project, Catalog::builtin(), blocks[1])
        .unwrap_or_else(|e| panic!("{e:#?}"));
    assert!(applied.project.clip("c9").is_some());
}
