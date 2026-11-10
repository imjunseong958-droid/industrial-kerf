//! Grades stacked on a clip, as a document says them: plain `fx` lines, in order, each with its own
//! `amount`, which may animate.

use kerf_lang::{apply_script, parse_document, print_document, PrintOptions};
use kerf_model::{Catalog, Param, Project, Value};

fn document(fx: &str) -> String {
    format!(
        "kerf 1\nproject name=\"t\"\nmedia m1 path=\"a.mov\" rate=24 dur=00:10:00:00 video=1920x1080\nsequence s1 rate=24 size=1920x1080 audio=48000\n  track V1 video\n    clip c1 media=m1 src=0f at=0f dur=48f\n      {fx}\n"
    )
}

fn read(fx: &str) -> Project {
    parse_document(&document(fx), Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{fx}: {errors:?}"))
        .project
}

/// What a document with `fx` is refused for.
fn refused(fx: &str) -> String {
    let errors = parse_document(&document(fx), Catalog::builtin())
        .map(|_| ())
        .expect_err(fx);
    errors
        .iter()
        .map(|error| error.message.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}

fn printed(project: &Project) -> String {
    print_document(project, Catalog::builtin(), PrintOptions::default())
}

fn run(project: &Project, script: &str) -> Project {
    apply_script(project, Catalog::builtin(), script)
        .unwrap_or_else(|errors| panic!("{script}: {errors:?}"))
        .project
}

const STACK: &str = "fx look.grade.teal-warm@1 amount=0.6\n      fx look.grade.sepia@1 amount={0f:0 24f:1:ease 48f:0.5:0.25,0.1,0.25,1}\n      fx look.grade.legal-range@1";

fn ids(project: &Project) -> Vec<String> {
    project
        .clip("c1")
        .unwrap()
        .effects
        .iter()
        .map(|effect| effect.fx.to_string())
        .collect()
}

#[test]
fn scripts_build_and_reorder_a_stack() {
    let plain = read("set opacity=1");
    let built = run(
        &plain,
        "fx c1 add look.grade.teal-warm@1 amount=0.6\nfx c1 add look.grade.sepia@1\nfx c1 set 2 amount={0f:0 24f:1:ease}",
    );
    assert_eq!(ids(&built), ["look.grade.teal-warm", "look.grade.sepia"]);
    assert!(
        printed(&built).contains("fx look.grade.sepia@1 amount={0f:0 24f:1:ease}\n"),
        "{}",
        printed(&built)
    );
    let moved = run(&built, "fx c1 move 2 to=1");
    assert_eq!(ids(&moved), ["look.grade.sepia", "look.grade.teal-warm"]);
}

#[test]
fn flatten_opens_one_grade_in_a_stack_where_it_stands() {
    let flat = run(&read(STACK), "fx c1 flatten 2");
    assert_eq!(
        ids(&flat),
        [
            "look.grade.teal-warm",
            "color.lumetri",
            "look.grade.legal-range"
        ]
    );
    let clip = flat.clip("c1").unwrap();
    // What was keyed on the amount is keyed on what the amount scales, at the same times.
    let Param::Animated(keys) = &clip.effects[1].params["look_intensity"] else {
        panic!("look_intensity animates");
    };
    let values: Vec<_> = keys.iter().map(|key| &key.value).collect();
    assert_eq!(
        values,
        [
            &Value::Number(0.0),
            &Value::Number(100.0),
            &Value::Number(50.0)
        ]
    );
    let source = read(STACK);
    let Param::Animated(amounts) = &source.clip("c1").unwrap().effects[1].params["amount"] else {
        panic!("amount animates");
    };
    for (key, amount) in keys.iter().zip(amounts) {
        assert_eq!((key.at, key.interp), (amount.at, amount.interp));
    }
    // A flat stack round trips too, and the first and last grades flatten just the same.
    let text = printed(&flat);
    let back = parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{text}: {errors:?}"))
        .project;
    assert_eq!(flat, back);
    let all = run(&flat, "fx c1 flatten 3\nfx c1 flatten 1");
    assert_eq!(
        ids(&all),
        ["color.lumetri", "color.lumetri", "color.levels"]
    );
}

#[test]
fn a_grade_s_amount_is_refused_outside_its_range() {
    for fx in [
        "fx look.grade.teal-warm@1 amount=2.5",
        "fx look.grade.teal-warm@1 amount=-0.5",
        // Mono grades stop at 1.
        "fx look.grade.sepia@1 amount=1.5",
        "fx look.grade.sepia@1 amount={0f:0 24f:1.5}",
        "fx look.grade.sepia@1 amount={0f:-0.1 24f:1}",
    ] {
        assert!(refused(fx).contains("amount"), "{fx}: {}", refused(fx));
    }
    for (fx, why) in [
        ("fx look.grade.sepia@1 amount=strong", "expects a number"),
        ("fx look.grade.sepia@1 amount=#ffffff", "expects a number"),
        (
            "fx look.grade.sepia@1 amount={24f:0 0f:1}",
            "increasing time order",
        ),
        ("fx look.grade.sepia@1 strength=1", "strength"),
    ] {
        assert!(refused(fx).contains(why), "{fx}: {}", refused(fx));
    }
    // In a stack, the one that is wrong is refused.
    let stack = "fx look.grade.teal-warm@1 amount=1\n      fx look.grade.sepia@1 amount=2";
    assert!(refused(stack).contains("amount"), "{}", refused(stack));
}

#[test]
fn only_the_numbers_a_look_scales_animate() {
    // A time places the look's keyframes, and a number inside them would have to curve.
    for (fx, name) in [
        ("fx look.shake@1 hit={0f:0 24f:1}", "hit"),
        ("fx look.shake@1 intensity={0f:0 24f:1}", "intensity"),
    ] {
        let why = refused(fx);
        assert!(
            why.contains(&format!("`{name}` cannot be animated")),
            "{why}"
        );
    }
    let catalog = Catalog::builtin();
    let animates = |look: &str, param: &str| {
        catalog
            .fx(look)
            .and_then(|def| def.find(param))
            .is_some_and(|param| param.animatable)
    };
    for look in catalog
        .looks()
        .filter(|l| l.id.as_str().starts_with("look.grade."))
    {
        assert!(animates(look.id.as_str(), "amount"), "{}", look.id);
    }
    assert!(!animates("look.shake", "intensity"));
    // A transition has no clip for its keys to count from.
    let transition = catalog
        .looks()
        .find(|look| look.on == kerf_model::look::On::Transition)
        .expect("a transition look");
    for param in &transition.params {
        assert!(
            !animates(transition.id.as_str(), &param.def.name),
            "{}",
            param.def.name
        );
    }
}

#[test]
fn every_number_said_to_animate_expands_with_its_keys() {
    let catalog = Catalog::builtin();
    for look in catalog
        .looks()
        .filter(|look| look.on == kerf_model::look::On::Clip)
    {
        let def = catalog.fx(look.id.as_str()).expect("listed");
        for param in def.params.iter().filter(|param| param.animatable) {
            let (low, high) = (param.min.unwrap_or(0.0), param.max.unwrap_or(1.0));
            let fx = format!(
                "fx {}@{} {}={{0f:{low} 24f:{high}}}",
                look.id, look.version, param.name
            );
            let project = read(&fx);
            kerf_model::look::expand(&project, catalog).unwrap_or_else(|why| panic!("{fx}: {why}"));
        }
    }
}
