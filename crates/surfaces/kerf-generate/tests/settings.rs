//! The generation file as Settings ▸ Generation changes it: what the person wrote there that is
//! not changed stays as they wrote it.

use kerf_generate::settings::{ProviderSettings, Settings};

#[test]
fn a_server_set_up_or_taken_away_leaves_the_rest_of_the_file_as_it_was() {
    let written = r#"{
  "providers": {
    "previs": { "run": ["vork", "render", "{ref}", "--out", "{out}"], "takes": { "reads": ["ref"] } }
  },
  "note": "mine"
}"#;
    let mut settings = Settings::parse(written).unwrap();
    settings
        .set_endpoint("pod", " https://a.trycloudflare.com/ ")
        .unwrap();
    let read = Settings::parse(&settings.text()).unwrap();
    let providers: Vec<(&str, &ProviderSettings)> = read.providers().collect();
    assert_eq!(providers.len(), 2);
    assert!(matches!(
        providers.iter().find(|(name, _)| *name == "pod").unwrap().1,
        ProviderSettings::Endpoint { url } if url == "https://a.trycloudflare.com"
    ));
    assert!(matches!(
        providers.iter().find(|(name, _)| *name == "previs").unwrap().1,
        ProviderSettings::Command { run, takes: Some(_) } if run[0] == "vork"
    ));
    assert!(settings.text().contains("\"note\": \"mine\""));

    // Set again, it is changed where it is; taken away, the program stays.
    settings
        .set_endpoint("pod", "http://127.0.0.1:8190")
        .unwrap();
    settings.remove("pod");
    let read = Settings::parse(&settings.text()).unwrap();
    assert_eq!(
        read.providers().map(|(name, _)| name).collect::<Vec<_>>(),
        ["previs"]
    );

    // What cannot be a provider is refused, and leaves it as it was.
    let before = settings.text();
    for (name, url) in [
        ("a/b", "https://x.dev"),
        ("pod", "ftp://x.dev"),
        ("pod", "x.dev"),
    ] {
        assert!(settings.set_endpoint(name, url).is_err(), "{name} {url}");
    }
    assert_eq!(settings.text(), before);
}

#[test]
fn a_provider_that_is_neither_a_server_nor_a_program_is_refused_by_its_name() {
    for (written, why) in [
        (r#"{"providers": {"pod": {}}}"#, "neither a `url`"),
        (
            r#"{"providers": {"pod": {"url": "https://x.dev", "run": ["a", "{out}"]}}}"#,
            "both a `url` and a `run`",
        ),
        (
            r#"{"providers": {"cli": {"run": ["a", "b"]}}}"#,
            "names neither `{request}` nor `{out}`",
        ),
        (
            r#"{"providers": {"cli": {"run": ["a", "{out}"], "takes": {"rate": 0}}}}"#,
            "`rate` in `takes`",
        ),
        (r#"{"providers": ["pod"]}"#, "`providers` is not an object"),
    ] {
        let refused = Settings::parse(written).unwrap_err();
        assert!(refused.contains(why), "{written}: {refused}");
    }
}
