//! Merges of small documents, each read back to check it prints as the
//! same canonical text.

use super::*;

const BASE: &str = r#"kerf 1
project name="Film"

media m1 path="a.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000
media m2 path="b.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main" rate=24 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00 link=L1
    clip c2 media=m2 src=00:01:00:00 at=01:00:04:00 dur=00:00:03:00
  track A1 audio
    clip a1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00 link=L1
  marker mk1 at=01:00:02:00 name="beat"
"#;

const C1: &str = "clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00";
const C2: &str = "    clip c2 media=m2 src=00:01:00:00 at=01:00:04:00 dur=00:00:03:00\n";
const C2_END: &str = "dur=00:00:03:00\n";
const A1_END: &str = "link=L1\n  marker";

/// `text` with each `(from, to)` replaced once; every `from` must be there.
fn edit(text: &str, edits: &[(&str, &str)]) -> String {
    edits.iter().fold(text.to_owned(), |text, (from, to)| {
        assert!(text.contains(from), "no `{from}` in\n{text}");
        text.replacen(from, to, 1)
    })
}

fn project(text: &str) -> Project {
    parse_document(text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:#?}"))
        .project
}

/// Merge, and check the result reads back to the same canonical text.
fn merge(base: &str, ours: &str, theirs: &str) -> MergedDocument {
    let merged = merge_documents(base, ours, theirs, Catalog::builtin())
        .unwrap_or_else(|error| panic!("{error}"));
    let again = print_document(
        &project(&merged.text),
        Catalog::builtin(),
        PrintOptions::default(),
    );
    assert_eq!(again, merged.text);
    merged
}

fn conflict(path: &str, what: &str, kept: &'static str) -> Conflict {
    Conflict {
        path: path.to_owned(),
        what: what.to_owned(),
        kept,
    }
}

fn renames(pairs: &[(&str, &str)]) -> Vec<(Id, Id)> {
    pairs
        .iter()
        .map(|(old, new)| (Id::new(*old).unwrap(), Id::new(*new).unwrap()))
        .collect()
}

#[test]
fn disjoint_edits_merge_cleanly() {
    let trim = (
        C1,
        "clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:03:00",
    );
    let path = ("path=\"b.mov\"", "path=\"b2.mov\"");
    let name = (C2_END, "dur=00:00:03:00 name=\"Wide\"\n");
    let marker = ("mk1 at=01:00:02:00", "mk1 at=01:00:03:00");
    let merged = merge(
        BASE,
        &edit(BASE, &[trim, path]),
        &edit(BASE, &[name, marker]),
    );
    assert_eq!(merged.text, edit(BASE, &[trim, path, name, marker]));
    assert_eq!(merged.conflicts, vec![]);
    assert_eq!(merged.renamed, vec![]);
}

#[test]
fn clips_appended_on_both_sides_under_one_id_are_both_kept() {
    let ours = edit(
        BASE,
        &[
            (
                C2_END,
                "dur=00:00:03:00\n    \
                 clip c3 media=m1 src=00:00:20:00 at=01:00:07:00 dur=00:00:02:00 link=L2\n    \
                 transition t1 from=c2 to=c3 fx=dissolve@1 dur=00:00:00:12\n",
            ),
            (
                A1_END,
                "link=L1\n    \
                 clip a2 media=m1 src=00:00:20:00 at=01:00:07:00 dur=00:00:02:00 link=L2\n  marker",
            ),
        ],
    );
    let theirs = edit(
        BASE,
        &[
            (
                C2_END,
                "dur=00:00:03:00\n    \
                 clip c3 media=m1 src=00:00:30:00 at=01:00:09:00 dur=00:00:02:00 link=L2\n    \
                 clip c4 media=m2 src=00:02:00:00 at=01:00:11:00 dur=00:00:02:00\n    \
                 transition t1 from=c3 to=c4 fx=dissolve@1 dur=00:00:00:12\n",
            ),
            (
                A1_END,
                "link=L1\n    \
                 clip a2 media=m1 src=00:00:30:00 at=01:00:09:00 dur=00:00:02:00 link=L2\n  marker",
            ),
        ],
    );
    let merged = merge(BASE, &ours, &theirs);
    assert_eq!(
        merged.text,
        r#"kerf 1
project name="Film"

media m1 path="a.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000
media m2 path="b.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main" rate=24 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00 link=L1
    clip c2 media=m2 src=00:01:00:00 at=01:00:04:00 dur=00:00:03:00
    clip c3 media=m1 src=00:00:20:00 at=01:00:07:00 dur=00:00:02:00 link=L2
    clip c5 media=m1 src=00:00:30:00 at=01:00:09:00 dur=00:00:02:00 link=L3
    clip c4 media=m2 src=00:02:00:00 at=01:00:11:00 dur=00:00:02:00
    transition t1 from=c2 to=c3 fx=dissolve@1 dur=00:00:00:12
    transition t2 from=c5 to=c4 fx=dissolve@1 dur=00:00:00:12
  track A1 audio
    clip a1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00 link=L1
    clip a2 media=m1 src=00:00:20:00 at=01:00:07:00 dur=00:00:02:00 link=L2
    clip a3 media=m1 src=00:00:30:00 at=01:00:09:00 dur=00:00:02:00 link=L3
  marker mk1 at=01:00:02:00 name="beat"
"#
    );
    assert_eq!(merged.conflicts, vec![]);
    assert_eq!(
        merged.renamed,
        renames(&[("c3", "c5"), ("a2", "a3"), ("t1", "t2"), ("L2", "L3")])
    );
}

#[test]
fn a_trim_on_one_side_and_a_move_on_the_other_both_apply() {
    let trimmed = |text: &str| edit(text, &[("dur=00:00:03:00", "dur=00:00:02:00")]);
    let moved = (
        C2,
        "  track V2 video\n    clip c2 media=m2 src=00:01:00:00 at=01:00:08:00 dur=00:00:03:00\n",
    );
    let merged = merge(BASE, &trimmed(BASE), &edit(BASE, &[moved]));
    assert_eq!(merged.text, trimmed(&edit(BASE, &[moved])));
    assert_eq!(merged.conflicts, vec![]);
}

#[test]
fn a_field_changed_differently_on_both_sides_keeps_ours() {
    let ours = edit(BASE, &[(C2_END, "dur=00:00:02:00\n")]);
    let theirs = edit(BASE, &[(C2_END, "dur=00:00:02:12 name=\"Wide\"\n")]);
    let merged = merge(BASE, &ours, &theirs);
    assert_eq!(
        merged.text,
        edit(BASE, &[(C2_END, "dur=00:00:02:00 name=\"Wide\"\n")])
    );
    assert_eq!(
        merged.conflicts,
        vec![conflict(
            "s1/V1/c2",
            "duration changed on both sides",
            "ours"
        )]
    );
}

#[test]
fn a_media_file_changed_on_both_sides_keeps_ours_whole() {
    // Ours relinks m1 to a consolidated copy; theirs replaces it.
    let ours = edit(BASE, &[("path=\"a.mov\"", "path=\"media/a-1a2b3c4d.mov\"")]);
    let theirs = edit(
        BASE,
        &[(
            "m1 path=\"a.mov\" rate=24 dur=00:10:00:00 video=1920x1080",
            "m1 path=\"media/new-9f8e7d6c.mov\" rate=25 dur=00:20:00:00 video=3840x2160",
        )],
    );
    let merged = merge(BASE, &ours, &theirs);
    assert_eq!(merged.text, ours);
    assert_eq!(
        merged.conflicts,
        vec![conflict("media m1", "file changed on both sides", "ours")]
    );
    // A clip theirs slipped past the end of ours' file stops the merge.
    let slipped = edit(
        &theirs,
        &[(
            C1,
            "clip c1 media=m1 src=00:15:00:00 at=01:00:00:00 dur=00:00:04:00",
        )],
    );
    let error = merge_documents(BASE, &ours, &slipped, Catalog::builtin()).unwrap_err();
    assert!(
        error.contains("clip.source_range") && error.contains("c1"),
        "{error}"
    );
    // The reel is not the file: it merges on its own.
    let reel = (
        "audio=2ch@48000\nmedia m2",
        "audio=2ch@48000 reel=\"A001\"\nmedia m2",
    );
    let merged = merge(BASE, &ours, &edit(BASE, &[reel]));
    assert_eq!(merged.text, edit(&ours, &[reel]));
    assert_eq!(merged.conflicts, vec![]);
    // Nor is what the file was made from.
    let recipe = (
        "audio=2ch@48000\nmedia m2",
        "audio=2ch@48000 recipe=\"shots/open.json\"\nmedia m2",
    );
    let merged = merge(BASE, &ours, &edit(BASE, &[recipe]));
    assert_eq!(merged.text, edit(&ours, &[recipe]));
    assert_eq!(merged.conflicts, vec![]);
    // Nor what it is lined up with: theirs lines m1 up with m2 while ours relinks it.
    let sync = (
        "audio=2ch@48000\nmedia m2",
        "audio=2ch@48000 sync=m2@1.234s\nmedia m2",
    );
    let merged = merge(BASE, &ours, &edit(BASE, &[sync]));
    assert_eq!(merged.text, edit(&ours, &[sync]));
    assert_eq!(merged.conflicts, vec![]);
    // Two offsets for it are one clash, and ours is kept.
    let other = (
        "audio=2ch@48000\nmedia m2",
        "audio=2ch@48000 sync=m2@1.5s\nmedia m2",
    );
    let merged = merge(BASE, &edit(BASE, &[other]), &edit(BASE, &[sync]));
    assert_eq!(merged.text, edit(BASE, &[other]));
    assert_eq!(
        merged.conflicts,
        vec![conflict("media m1", "sync changed on both sides", "ours")]
    );
}

#[test]
fn removed_against_changed_keeps_ours_either_way() {
    let retire = ("project name=\"Film\"", "project name=\"Film\" retired=c2");
    let removed = edit(BASE, &[retire, (C2, "")]);
    let named = edit(BASE, &[(C2_END, "dur=00:00:03:00 name=\"Wide\"\n")]);

    let merged = merge(BASE, &removed, &named);
    assert_eq!(merged.text, removed);
    assert_eq!(
        merged.conflicts,
        vec![conflict("s1/V1/c2", "removed here, changed there", "ours")]
    );

    let merged = merge(BASE, &named, &removed);
    assert_eq!(merged.text, named);
    assert_eq!(
        merged.conflicts,
        vec![conflict("s1/V1/c2", "changed here, removed there", "ours")]
    );

    // Removed against unchanged is removed, without a conflict.
    let close = (C1, &format!("{C1} name=\"Close\"")[..]);
    let merged = merge(BASE, &removed, &edit(BASE, &[close]));
    assert_eq!(merged.text, edit(&removed, &[close]));
    assert_eq!(merged.conflicts, vec![]);
}

#[test]
fn clips_overlapping_after_the_merge_move_theirs_to_a_free_track() {
    let ours = edit(
        BASE,
        &[(
            C2_END,
            "dur=00:00:03:00\n    clip c3 media=m1 src=00:00:20:00 at=01:00:07:00 dur=00:00:04:00\n",
        )],
    );
    let theirs = edit(
        BASE,
        &[(
            C2_END,
            "dur=00:00:03:00\n    \
             clip c3 media=m2 src=00:02:00:00 at=01:00:07:00 dur=00:00:02:00\n    \
             clip c4 media=m2 src=00:03:00:00 at=01:00:09:00 dur=00:00:02:00\n    \
             transition t1 from=c3 to=c4 fx=dissolve@1 dur=00:00:00:12\n",
        )],
    );
    let merged = merge(BASE, &ours, &theirs);
    assert_eq!(
        merged.text,
        edit(
            BASE,
            &[(
                C2_END,
                "dur=00:00:03:00\n    \
                 clip c3 media=m1 src=00:00:20:00 at=01:00:07:00 dur=00:00:04:00\n  \
                 track V2 video\n    \
                 clip c5 media=m2 src=00:02:00:00 at=01:00:07:00 dur=00:00:02:00\n    \
                 clip c4 media=m2 src=00:03:00:00 at=01:00:09:00 dur=00:00:02:00\n    \
                 transition t1 from=c5 to=c4 fx=dissolve@1 dur=00:00:00:12\n",
            )]
        )
    );
    assert_eq!(
        merged.conflicts,
        vec![
            conflict("s1/V1/c5", "overlapped c3 on V1; moved to V2", "theirs"),
            conflict("s1/V1/c4", "overlapped c3 on V1; moved to V2", "theirs"),
        ]
    );
    assert_eq!(merged.renamed, renames(&[("c3", "c5")]));
}

#[test]
fn tracks_added_on_both_sides_merge_or_get_a_fresh_id() {
    let ours = edit(
        BASE,
        &[
            (
                "  track A1 audio\n",
                "  track V2 video\n    \
                 clip c3 gen=solid@1 at=01:00:00:00 dur=00:00:02:00\n  track A1 audio\n",
            ),
            (
                A1_END,
                "link=L1\n  track A2 audio\n    \
                 clip a2 media=m1 src=00:01:00:00 at=01:00:00:00 dur=00:00:02:00\n  marker",
            ),
        ],
    );
    let theirs = edit(
        BASE,
        &[
            (
                "link=L1\n    clip c2",
                "link=L1\n      fx key.trackmatte@1 matte=V2\n    clip c2",
            ),
            (
                "  track A1 audio\n",
                "  track V2 video name=\"Matte\"\n    \
                 clip c3 gen=solid@1 at=01:00:00:00 dur=00:00:04:00\n  track A1 audio\n",
            ),
            (
                A1_END,
                "link=L1\n  track A2 audio\n    \
                 clip a2 media=m1 src=00:02:00:00 at=01:00:05:00 dur=00:00:02:00\n  marker",
            ),
        ],
    );
    let merged = merge(BASE, &ours, &theirs);
    assert_eq!(
        merged.text,
        r#"kerf 1
project name="Film"

media m1 path="a.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000
media m2 path="b.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main" rate=24 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00 link=L1
      fx key.trackmatte@1 matte=V3
    clip c2 media=m2 src=00:01:00:00 at=01:00:04:00 dur=00:00:03:00
  track V2 video
    clip c3 gen=solid@1 at=01:00:00:00 dur=00:00:02:00
  track V3 video name="Matte"
    clip c4 gen=solid@1 at=01:00:00:00 dur=00:00:04:00
  track A1 audio
    clip a1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00 link=L1
  track A2 audio
    clip a2 media=m1 src=00:01:00:00 at=01:00:00:00 dur=00:00:02:00
    clip a3 media=m1 src=00:02:00:00 at=01:00:05:00 dur=00:00:02:00
  marker mk1 at=01:00:02:00 name="beat"
"#
    );
    assert_eq!(merged.conflicts, vec![]);
    assert_eq!(
        merged.renamed,
        renames(&[("V2", "V3"), ("c3", "c4"), ("a2", "a3")])
    );
}

#[test]
fn a_renamed_marker_takes_the_looks_that_land_on_it_along() {
    let beat = "name=\"beat\"\n";
    let ours = edit(
        BASE,
        &[(
            beat,
            "name=\"beat\"\n  marker mk2 at=01:00:03:00 name=\"ours\"\n",
        )],
    );
    let theirs = edit(
        BASE,
        &[
            (C2_END, "dur=00:00:03:00\n      fx look.flash@1 hit=mk2\n"),
            (
                beat,
                "name=\"beat\"\n  marker mk2 at=01:00:05:00 name=\"theirs\"\n",
            ),
        ],
    );
    let merged = merge(BASE, &ours, &theirs);
    assert_eq!(
        merged.text,
        edit(
            BASE,
            &[
                (C2_END, "dur=00:00:03:00\n      fx look.flash@1 hit=mk3\n"),
                (
                    beat,
                    "name=\"beat\"\n  marker mk2 at=01:00:03:00 name=\"ours\"\n  \
                     marker mk3 at=01:00:05:00 name=\"theirs\"\n",
                ),
            ]
        )
    );
    assert_eq!(merged.conflicts, vec![]);
    assert_eq!(merged.renamed, renames(&[("mk2", "mk3")]));
}

#[test]
fn properties_merge_key_by_key() {
    let set = |props: &str| {
        let line = format!("link=L1\n      set {props}\n    clip c2");
        edit(BASE, &[("link=L1\n    clip c2", &line)])
    };
    let merged = merge(BASE, &set("scale=1.2 opacity=0.5"), &set("x=0.1 scale=0.8"));
    assert_eq!(merged.text, set("x=0.1 scale=1.2 opacity=0.5"));
    assert_eq!(
        merged.conflicts,
        vec![conflict("s1/V1/c1", "scale changed on both sides", "ours")]
    );
}

#[test]
fn track_roles_and_sync_locks_merge_by_track() {
    let role = ("  track V1 video\n", "  track V1 video role=titles\n");
    let sync = (
        "  track A1 audio\n",
        "  track A1 audio role=music sync=off\n",
    );
    let merged = merge(BASE, &edit(BASE, &[role]), &edit(BASE, &[sync]));
    assert_eq!(merged.text, edit(BASE, &[role, sync]));
    assert_eq!(merged.conflicts, vec![]);
    assert_eq!(
        summarize(&project(BASE), &project(&merged.text)),
        [
            "Changed track V1: role",
            "Changed track A1: role, sync lock"
        ]
    );

    // The same field set apart on both sides keeps ours.
    let theirs = (
        "  track V1 video\n",
        "  track V1 video role=\"B roll\" sync=off\n",
    );
    let merged = merge(BASE, &edit(BASE, &[role]), &edit(BASE, &[theirs]));
    assert_eq!(
        merged.text,
        edit(
            BASE,
            &[(
                "  track V1 video\n",
                "  track V1 video role=titles sync=off\n"
            )]
        )
    );
    assert_eq!(
        merged.conflicts,
        vec![conflict("s1/V1", "role changed on both sides", "ours")]
    );
}

#[test]
fn a_file_added_on_both_sides_merges_from_an_empty_base() {
    let ours = r#"kerf 1
project name="Film"

media m1 path="a.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main" rate=24 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00
"#;
    let theirs = r#"kerf 1
project name="Film"

media m1 path="b.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main" rate=24 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=00:00:00:00 at=01:00:04:00 dur=00:00:04:00
"#;
    let merged = merge("", ours, theirs);
    assert_eq!(
        merged.text,
        r#"kerf 1
project name="Film"

media m1 path="a.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000
media m2 path="b.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main" rate=24 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00
    clip c2 media=m2 src=00:00:00:00 at=01:00:04:00 dur=00:00:04:00
"#
    );
    assert_eq!(merged.conflicts, vec![]);
    assert_eq!(merged.renamed, renames(&[("m1", "m2"), ("c1", "c2")]));
    assert_eq!(merge(" \n", ours, ours).text, ours);
}

#[test]
fn merging_identical_sides_changes_nothing() {
    let ours = edit(
        BASE,
        &[
            ("project name=\"Film\"", "project name=\"Film\" retired=c2"),
            (
                C2,
                "    clip c3 media=m1 src=00:00:20:00 at=01:00:05:00 dur=00:00:02:00\n",
            ),
            (C1, &format!("{C1} name=\"Close\"")),
        ],
    );
    for theirs in [ours.as_str(), BASE] {
        let merged = merge(BASE, &ours, theirs);
        assert_eq!(merged.text, ours);
        assert_eq!(merged.conflicts, vec![]);
        assert_eq!(merged.renamed, vec![]);
    }
    assert_eq!(merge(BASE, BASE, &ours).text, ours);
    assert_eq!(merge(BASE, BASE, BASE).text, BASE);
    let project = project(&ours);
    let merged = merge_projects(&project, &project, &project, Catalog::builtin()).unwrap();
    assert_eq!(merged.project, project);
}

#[test]
fn a_track_one_side_removed_comes_back_for_clips_the_other_put_on_it() {
    let base = edit(
        BASE,
        &[("  track A1 audio\n", "  track V2 video\n  track A1 audio\n")],
    );
    let theirs = edit(
        &base,
        &[(
            "  track V2 video\n",
            "  track V2 video\n    clip c3 gen=solid@1 at=01:00:00:00 dur=00:00:01:00\n",
        )],
    );
    let merged = merge(&base, BASE, &theirs);
    assert_eq!(merged.text, theirs);
    assert_eq!(
        merged.conflicts,
        vec![conflict(
            "s1/V2",
            "removed here, used there; restored",
            "theirs"
        )]
    );
}

#[test]
fn a_sequence_one_side_removed_comes_back_whole_for_a_clip_the_other_nests_it_in() {
    // s2 nests s3, which comes back too once s2's clips do.
    let reel = |id: &str, clip: &str| {
        format!(
            "\nsequence {id} name=\"Reel\" rate=24 size=1920x1080 audio=48000 start=00:00:00:00\n  \
             track V1 video\n    \
             clip {clip} src=00:00:00:00 at=00:00:00:00 dur=00:00:05:00\n"
        )
    };
    let base = format!(
        "{BASE}{}{}",
        reel("s2", "c7 seq=s3"),
        reel("s3", "c9 media=m2")
    );
    let theirs = edit(
        &base,
        &[(
            C2_END,
            "dur=00:00:03:00\n    clip c8 seq=s2 src=00:00:01:00 at=01:00:07:00 dur=00:00:02:00\n",
        )],
    );
    let restored = |what: &str, kept| {
        ["sequence s2", "sequence s3"]
            .map(|path| conflict(path, what, kept))
            .to_vec()
    };
    let merged = merge(&base, BASE, &theirs);
    assert_eq!(merged.text, theirs);
    assert_eq!(
        merged.conflicts,
        restored("removed here, used there; restored", "theirs")
    );
    // From the other side, ours' clip brings them back from ours.
    let merged = merge(&base, &theirs, BASE);
    assert_eq!(merged.text, theirs);
    assert_eq!(
        merged.conflicts,
        restored("removed there, used here; restored", "ours")
    );
}

#[test]
fn a_track_one_side_removed_comes_back_whole_for_an_effect_the_other_reads_it_with() {
    let base = edit(
        BASE,
        &[(
            "  track A1 audio\n",
            "  track V2 video\n    \
             clip c3 gen=solid@1 at=01:00:00:00 dur=00:00:04:00\n  track A1 audio\n",
        )],
    );
    let matte = (
        "link=L1\n    clip c2",
        "link=L1\n      fx key.trackmatte@1 matte=V2\n    clip c2",
    );
    let theirs = edit(&base, &[matte]);
    let merged = merge(&base, BASE, &theirs);
    assert_eq!(merged.text, theirs);
    assert_eq!(
        merged.conflicts,
        vec![conflict(
            "s1/V2",
            "removed here, used there; restored",
            "theirs"
        )]
    );
    // A sound track an audio spectrum draws comes back the same way.
    let base = edit(
        BASE,
        &[(
            A1_END,
            "link=L1\n  track A2 audio\n    \
             clip a2 media=m2 src=00:00:00:00 at=01:00:00:00 dur=00:00:04:00\n  marker",
        )],
    );
    let spectrum = (
        "link=L1\n    clip c2",
        "link=L1\n      fx generate.audiospectrum@1 audioLayer=A2\n    clip c2",
    );
    let theirs = edit(&base, &[spectrum]);
    let merged = merge(&base, BASE, &theirs);
    assert_eq!(merged.text, theirs);
    assert_eq!(
        merged.conflicts,
        vec![conflict(
            "s1/A2",
            "removed here, used there; restored",
            "theirs"
        )]
    );
}

#[test]
fn a_clip_both_sides_added_alike_keeps_its_link_group() {
    let a2 = "    clip a2 media=m1 src=00:00:20:00 at=01:00:07:00 dur=00:00:02:00 link=L2\n";
    let added = |audio: &str| {
        edit(
            BASE,
            &[
                (
                    C2_END,
                    "dur=00:00:03:00\n    \
                     clip c3 media=m1 src=00:00:20:00 at=01:00:07:00 dur=00:00:02:00 link=L2\n",
                ),
                (A1_END, &format!("link=L1\n{audio}  marker")),
            ],
        )
    };
    let ours = added(a2);
    let theirs = added(&format!("{a2}      set gain=-3\n"));
    let merged = merge(BASE, &ours, &theirs);
    let a3 = a2.replace("a2", "a3");
    assert_eq!(
        merged.text,
        added(&format!("{a2}  track A2 audio\n{a3}      set gain=-3\n"))
    );
    assert_eq!(
        merged.conflicts,
        vec![conflict(
            "s1/A1/a3",
            "overlapped a2 on A1; moved to A2",
            "theirs"
        )]
    );
    assert_eq!(merged.renamed, renames(&[("a2", "a3")]));
}

#[test]
fn a_second_transition_on_one_cut_is_dropped() {
    let transition = |id: &str, frames: &str| {
        let line = format!(
            "dur=00:00:03:00\n    transition {id} from=c1 to=c2 fx=dissolve@1 dur=00:00:00:{frames}\n"
        );
        edit(BASE, &[(C2_END, &line)])
    };
    let merged = merge(BASE, &transition("t1", "12"), &transition("t1", "06"));
    assert_eq!(merged.text, transition("t1", "12"));
    assert_eq!(
        merged.conflicts,
        vec![conflict(
            "s1/V1/t2",
            "the cut between c1 and c2 already has a transition; removed",
            "ours"
        )]
    );
    assert_eq!(merged.renamed, renames(&[("t1", "t2")]));
}

#[test]
fn a_merge_that_cannot_be_valid_says_why() {
    let ours = edit(BASE, &[("src=00:01:00:00", "src=00:09:57:00")]);
    let theirs = edit(BASE, &[(C2_END, "dur=00:00:04:00\n")]);
    let error = merge_documents(BASE, &ours, &theirs, Catalog::builtin()).unwrap_err();
    assert!(
        error.contains("clip.source_range") && error.contains("c2"),
        "{error}"
    );
    let error = merge_documents(BASE, "kerf 1\nclip\n", BASE, Catalog::builtin()).unwrap_err();
    assert!(error.starts_with("ours does not parse"), "{error}");
}
