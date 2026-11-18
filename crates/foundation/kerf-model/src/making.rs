//! How a media's file is made, where Kerf or a model makes it: the steps
//! written under its line in the document.
//!
//! ```text
//! media depth path="shots/depth.mp4" rate=16 dur=65f video=832x480
//!   render s2 {guide.mp4} pass=depth size=832x480 rate=16
//!   generate model=shot prompt="a red car on a wet street" guide={guide.mp4} seconds=4 seed=7
//! ```
//!
//! A step is one of three things, and they run in order:
//!
//! - `render <sequence> {name}` has Kerf draw a sequence of the project
//!   into a file: its picture, a pass of it, or the matte of one clip.
//! - `generate model=<name> …` asks a model the person has set up for a
//!   file: what it should show, what guides it, for how long, with which
//!   seed.
//! - `join {part} {part} … {name}` sets files earlier steps made end to
//!   end into one.
//!
//! `{out}` stands for the media's file, which the last step writes, and any
//! other `{name}` for a file a step makes on the way and a later one reads.
//! A file of the project is named from the project's folder, as a media's
//! path is.
//!
//! No step names a program: Kerf draws, and a model is a program the person
//! names in a file of their own, outside any project. So a document, and a
//! preset made from one, can say what is made and never what runs.
//!
//! This module holds the steps as the document keeps them and reads one
//! from its words. Running them is the engine's.

use crate::time::Rate;
use std::collections::BTreeMap;

/// What stands for the media's own file among the files steps make.
pub const OUT: &str = "out";

/// What of a sequence is drawn: its picture, or something a picture does not say that the document
/// knows, for a model that follows it or for measuring what shows.
///
/// A pass is drawn from the same clips, placed and seen as the picture's are, with no lights, no
/// focus, no adjustment layers, no dips, no captions, and every clip laid over what is behind it
/// whatever its blend. It is drawn on the CPU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Pass {
    /// The picture.
    #[default]
    Picture,
    /// Which clip each pixel is of: every clip that draws in one flat color of its own, over
    /// black where there is none. A nested sequence is one clip.
    Ids,
    /// How near each pixel is: white at the camera, mid grey where a clip at `z=0` stands before a
    /// camera left where it is, and darker the farther, black where nothing is in depth. A clip
    /// out of depth has no distance, and is black.
    Depth,
    /// [`Self::Depth`] with each frame stretched to its own nearest and farthest: the
    /// farthest of what is in depth just above black and the nearest white, as a model that
    /// reads depth from a picture gives it, and as one trained on such depth expects it.
    /// It does not hold still: the same place is another grey as what is in the frame
    /// changes.
    Relative,
    /// Which way each pixel faces, as a color, in the camera's own space: to the right as
    /// red, up as green, and toward the camera as blue, each from -1 to 1 as black to
    /// full, as a normal map has them. A surface seen from behind faces whoever sees it. A
    /// clip in depth is flat, so it is one color; a mesh has the way each place of it
    /// faces. What is not in depth is black.
    Normals,
}

impl Pass {
    /// The pass a step's `pass=` names.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "picture" => Some(Self::Picture),
            "ids" => Some(Self::Ids),
            "depth" => Some(Self::Depth),
            "relative" => Some(Self::Relative),
            "normals" => Some(Self::Normals),
            _ => None,
        }
    }

    /// Its name, as a step writes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Picture => "picture",
            Self::Ids => "ids",
            Self::Depth => "depth",
            Self::Relative => "relative",
            Self::Normals => "normals",
        }
    }
}

/// A file a model is handed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A file of the project, as the step names it.
    File(String),
    /// A file a step before made, by the name it has in braces.
    Made(String),
}

impl Source {
    /// The source as a step writes it.
    pub fn shown(&self) -> String {
        match self {
            Self::File(file) => file.clone(),
            Self::Made(name) => format!("{{{name}}}"),
        }
    }
}

/// What a `generate` step asks of a model.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    /// The model, by the name the person's models file gives it.
    pub model: String,
    /// What the file should show, in words.
    pub prompt: String,
    /// A picture in motion to follow: a blocking, a rough render, a take.
    pub guide: Option<Source>,
    /// More pictures in motion to follow, each by what it is of: a model that reads a
    /// picture and its depth, or depth and which clip each pixel is of, is handed each
    /// under its name (`guide_depth=`, `guide_ids=`).
    pub guides: Vec<(String, Source)>,
    /// The first and the last frame.
    pub first: Option<Source>,
    pub last: Option<Source>,
    /// Pictures of who and what is in it, and of how it should look.
    pub references: Vec<Source>,
    pub seconds: Option<f64>,
    /// Where the shot starts in its sequences' time, in seconds, where it is not their
    /// start: a part of a longer shot. A signal and where a clip goes are read from there.
    pub from: Option<f64>,
    pub size: Option<(u32, u32)>,
    pub seed: Option<u64>,
    /// Numbers over time that say when what the prompt asks for happens:
    /// signal clips of the project, by their ids.
    pub signals: Vec<String>,
    /// What goes where: clips of the project, by their ids, each handed over as its place
    /// on every frame.
    pub moves: Vec<String>,
    /// Who and what it shows, and how it looks: the project's cast and styles, by their
    /// names. Each one's words follow the prompt and its pictures the references, once
    /// the steps are read for a making.
    pub cast: Vec<String>,
    pub styles: Vec<String>,
    /// A graph of the model's own to run in place of the one its program
    /// would: a file of the project, for a model whose program takes one. A
    /// program that takes none has no use for it, and nothing says so: what
    /// a program reads of a request is its own.
    pub workflow: Option<Source>,
    /// Anything else the step says, which only that model reads.
    pub more: BTreeMap<String, String>,
    /// The file it writes: `out`, or the name of one a later step reads.
    pub out: String,
}

/// The files a `generate` step hands a model, by the names it gives them.
pub const FILES: [&str; 4] = ["guide", "first", "last", "ref"];

/// The name a `generate` step hands a model a signal by.
pub const SIGNAL: &str = "signal";

/// The name a `generate` step hands a model a clip's way across the frame by.
pub const MOVES: &str = "moves";

/// The name a `generate` step hands a model a graph of its own to run by.
pub const WORKFLOW: &str = "workflow";

/// The file a project says its cast and styles in, which `cast=` and `style=` name.
pub const CAST_FILE: &str = "cast.kerfcast";

/// Whether `name` is what a guide beside the first is named by, after `guide_`: lowercase
/// letters and digits, as a workflow's blank is.
pub fn names_a_guide(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|letter| letter.is_ascii_lowercase() || letter.is_ascii_digit())
}

/// Whether `name` can name a cast or a style: letters, digits, `-` and `_`.
pub fn names_cast(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_alphanumeric() || "-_".contains(character))
}

impl Ask {
    /// Read the words after `generate`. `made` holds the files the steps
    /// before it make.
    pub fn parse(words: &[String], made: &[String]) -> Result<Self, String> {
        let mut ask = Self {
            model: String::new(),
            prompt: String::new(),
            guide: None,
            guides: Vec::new(),
            first: None,
            last: None,
            references: Vec::new(),
            seconds: None,
            from: None,
            size: None,
            seed: None,
            signals: Vec::new(),
            moves: Vec::new(),
            cast: Vec::new(),
            styles: Vec::new(),
            workflow: None,
            more: BTreeMap::new(),
            out: OUT.to_owned(),
        };
        let source = |value: &str| match whole(value) {
            Some(name) if made.contains(&name) => Ok(Source::Made(name)),
            Some(name) => Err(format!("no step before this one makes `{{{name}}}`")),
            None if value.is_empty() => Err("a file's name is empty".to_owned()),
            None => Ok(Source::File(value.to_owned())),
        };
        // A name that holds one value, given twice, would keep the last and drop the first
        // unsaid: `out={lens.txt} out={out}` made no lens. Those that hold a list may repeat.
        let mut given = std::collections::BTreeSet::new();
        for word in words {
            let (key, value) = word.split_once('=').ok_or_else(|| {
                format!(
                    "`{word}` is not `<name>=<value>`; a value that holds spaces is quoted, as `prompt=\"a boat leaves\"`"
                )
            })?;
            let listed = matches!(key, "ref" | "signal" | "moves" | "cast" | "style")
                || key.starts_with("guide_");
            if !listed && !given.insert(key) {
                return Err(format!(
                    "`{key}=` is given twice; a line takes one{}",
                    if key == "out" {
                        ", the file it makes: a second file of the same model is a `generate` line of its own"
                    } else {
                        ""
                    }
                ));
            }
            match key {
                "model" => ask.model = value.to_owned(),
                "prompt" => ask.prompt = value.to_owned(),
                "guide" => ask.guide = Some(source(value)?),
                named if named.strip_prefix("guide_").is_some_and(names_a_guide) => {
                    let name = named["guide_".len()..].to_owned();
                    if ask.guides.iter().any(|(given, _)| *given == name) {
                        return Err(format!("`{named}=` is given twice"));
                    }
                    ask.guides.push((name, source(value)?));
                }
                "first" => ask.first = Some(source(value)?),
                "last" => ask.last = Some(source(value)?),
                WORKFLOW => ask.workflow = Some(source(value)?),
                "ref" => ask.references.push(source(value)?),
                "from" => {
                    ask.from = Some(
                        value
                            .parse()
                            .ok()
                            .filter(|from: &f64| from.is_finite() && *from >= 0.0)
                            .ok_or_else(|| {
                                format!("`from={value}` is not a number of seconds from the sequence's start, such as 5")
                            })?,
                    );
                }
                "seconds" => {
                    ask.seconds = Some(
                        value
                            .parse()
                            .ok()
                            .filter(|seconds: &f64| seconds.is_finite() && *seconds > 0.0)
                            .ok_or_else(|| {
                                format!("`seconds={value}` is not a length in seconds")
                            })?,
                    );
                }
                "size" => {
                    ask.size = Some(size_of(value).ok_or_else(|| {
                        format!("`size={value}` is not a size such as 1920x1080")
                    })?);
                }
                "seed" => {
                    ask.seed = Some(
                        value
                            .parse()
                            .map_err(|_| format!("`seed={value}` is not a whole number"))?,
                    );
                }
                "signal" if value.is_empty() => {
                    return Err(
                        "`signal=` names a signal clip of the project, such as `signal=swell`"
                            .to_owned(),
                    )
                }
                "signal" => ask.signals.push(value.to_owned()),
                "moves" if value.is_empty() => {
                    return Err(
                        "`moves=` names a clip of the project whose way across the frame a model is handed, such as `moves=car`"
                            .to_owned(),
                    )
                }
                "moves" => ask.moves.push(value.to_owned()),
                "cast" | "style" => {
                    if !names_cast(value) {
                        return Err(format!(
                            "`{key}=` names one of the project's `{CAST_FILE}` by its name there, such as `{key}={}`",
                            if key == "cast" { "Ada" } else { "film" }
                        ));
                    }
                    let named = if key == "cast" {
                        &mut ask.cast
                    } else {
                        &mut ask.styles
                    };
                    if named.iter().any(|given| given == value) {
                        return Err(format!("`{word}` is given twice"));
                    }
                    named.push(value.to_owned());
                }
                "out" => {
                    ask.out = whole(value).ok_or_else(|| {
                        format!("`out={value}` names the file it makes in braces, such as `{{first.png}}`")
                    })?;
                }
                "" => return Err(format!("`{word}` has no name before its `=`")),
                other => {
                    ask.more.insert(other.to_owned(), value.to_owned());
                }
            }
        }
        if ask.model.is_empty() {
            return Err("`generate` names its model: `model=<name>`".to_owned());
        }
        Ok(ask)
    }

    /// Every file it hands the model.
    pub fn sources(&self) -> impl Iterator<Item = &Source> {
        self.guide
            .iter()
            .chain(self.guides.iter().map(|(_, source)| source))
            .chain(&self.first)
            .chain(&self.last)
            .chain(&self.references)
            .chain(&self.workflow)
    }

    /// What it asks, as a person reads it.
    pub fn shown(&self) -> String {
        let mut parts = vec![format!("prompt \"{}\"", self.prompt)];
        let mut named =
            |name: &str, source: &Source| parts.push(format!("{name} {}", source.shown()));
        if let Some(guide) = &self.guide {
            named("guide", guide);
        }
        for (name, guide) in &self.guides {
            named(&format!("guide_{name}"), guide);
        }
        if let Some(first) = &self.first {
            named("first", first);
        }
        if let Some(last) = &self.last {
            named("last", last);
        }
        for reference in &self.references {
            named("ref", reference);
        }
        if let Some(seconds) = self.seconds {
            parts.push(format!("{seconds} s"));
        }
        if let Some(from) = self.from {
            parts.push(format!("from {from} s"));
        }
        if let Some((width, height)) = self.size {
            parts.push(format!("{width}x{height}"));
        }
        if let Some(seed) = self.seed {
            parts.push(format!("seed {seed}"));
        }
        if let Some(workflow) = &self.workflow {
            parts.push(format!("{WORKFLOW} {}", workflow.shown()));
        }
        parts.extend(self.signals.iter().map(|signal| format!("signal {signal}")));
        parts.extend(self.moves.iter().map(|clip| format!("moves {clip}")));
        parts.extend(
            self.more
                .iter()
                .map(|(name, value)| format!("{name} {value}")),
        );
        parts.join(", ")
    }

    /// The words after `generate` that say it, in the order a document writes them.
    fn words(&self) -> Vec<String> {
        let mut words = vec![format!("model={}", self.model)];
        if !self.prompt.is_empty() {
            words.push(format!("prompt={}", self.prompt));
        }
        let mut named = |name: &str, source: &Source| words.push(format!("{name}={}", source.shown()));
        if let Some(guide) = &self.guide {
            named("guide", guide);
        }
        for (name, guide) in &self.guides {
            named(&format!("guide_{name}"), guide);
        }
        if let Some(first) = &self.first {
            named("first", first);
        }
        if let Some(last) = &self.last {
            named("last", last);
        }
        for reference in &self.references {
            named("ref", reference);
        }
        words.extend(self.cast.iter().map(|name| format!("cast={name}")));
        words.extend(self.styles.iter().map(|name| format!("style={name}")));
        words.extend(self.signals.iter().map(|signal| format!("{SIGNAL}={signal}")));
        words.extend(self.moves.iter().map(|clip| format!("{MOVES}={clip}")));
        if let Some(from) = self.from {
            words.push(format!("from={from}"));
        }
        if let Some(seconds) = self.seconds {
            words.push(format!("seconds={seconds}"));
        }
        if let Some((width, height)) = self.size {
            words.push(format!("size={width}x{height}"));
        }
        if let Some(seed) = self.seed {
            words.push(format!("seed={seed}"));
        }
        if let Some(workflow) = &self.workflow {
            words.push(format!("{WORKFLOW}={}", workflow.shown()));
        }
        words.extend(self.more.iter().map(|(name, value)| format!("{name}={value}")));
        if self.out != OUT {
            words.push(format!("out={{{}}}", self.out));
        }
        words
    }
}

/// One thing done on the way to a media's file.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// A sequence of the project, drawn by Kerf into the file `to` names,
    /// with no sound: at the sequence's size and rate, or at the ones named,
    /// and its picture, or the pass of it named.
    Render {
        sequence: String,
        to: String,
        size: Option<(u32, u32)>,
        rate: Option<Rate>,
        pass: Pass,
        /// A clip of the sequence drawn alone, white on black, in place of a pass.
        matte: Option<String>,
        /// The part of the sequence drawn, in seconds from its start and for
        /// how long: the whole of it where neither is said. A shot longer
        /// than a model makes at once is drawn a part at a time.
        from: Option<f64>,
        seconds: Option<f64>,
    },
    /// A model asked for a file.
    Generate(Box<Ask>),
    /// Pictures in motion steps before it made, set end to end into the
    /// file `to` names: the parts of a shot longer than a model makes at
    /// once. Each part after the first loses its first `lap` frames, the
    /// ones it has of the part before it where it went on from that one's
    /// last frame.
    Join {
        parts: Vec<String>,
        to: String,
        lap: u32,
    },
}

/// What a name a step holds is of: a sequence, a clip, or a file of the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Named {
    Sequence,
    Clip,
    File,
}

/// The words a step starts with.
pub const VERBS: [&str; 3] = ["render", "generate", "join"];

/// How each kind of step is written, for an error that says so.
pub const SHAPES: &str = "`render <sequence> {name} [size=WxH] [rate=<fps>] [pass=ids|depth|relative|normals] [matte=<clip>] [from=<seconds>] [seconds=<seconds>]`, `generate model=<name> prompt=\"…\" …`, or `join {part} {part} … {name} [lap=<frames>]`";

impl Step {
    /// Read a step from its words, the first of which says what it is: `render`,
    /// `generate` or `join`. `made` holds the files the steps before it make. What is
    /// wrong is said as a line of text says it.
    pub fn parse(words: &[String], made: &[String]) -> Result<Self, String> {
        let Some((verb, words)) = words.split_first() else {
            return Err(format!("a step is {SHAPES}"));
        };
        match verb.as_str() {
            "render" if words.len() >= 2 => {
                let to = whole(&words[1]).ok_or_else(|| {
                    format!(
                        "`render <sequence> {{name}}` names the file it makes, such as `{{guide.mp4}}`, not `{}`",
                        words[1]
                    )
                })?;
                let (size, rate, pass, matte, from, seconds) = drawn_at(&words[2..])?;
                Ok(Self::Render {
                    sequence: words[0].clone(),
                    to,
                    size,
                    rate,
                    pass,
                    matte,
                    from,
                    seconds,
                })
            }
            "join" if !words.is_empty() => {
                let (parts, to, lap) = joined(words, made)?;
                Ok(Self::Join { parts, to, lap })
            }
            "generate" if !words.is_empty() => Ok(Self::Generate(Box::new(Ask::parse(words, made)?))),
            "render" | "generate" | "join" => Err(format!("`{verb}` is {SHAPES}")),
            other => Err(format!(
                "`{other}` is not a step; one is `render`, `generate`, or `join`"
            )),
        }
    }

    /// The step as the words a document writes it in: what it is first, and each word
    /// after it as it reads with no quotes around it, a `name=value` or a word alone.
    /// [`Step::parse`] of them is the step again.
    pub fn words(&self) -> Vec<String> {
        match self {
            Self::Render {
                sequence,
                to,
                size,
                rate,
                pass,
                matte,
                from,
                seconds,
            } => {
                let mut words = vec!["render".to_owned(), sequence.clone(), format!("{{{to}}}")];
                if *pass != Pass::Picture {
                    words.push(format!("pass={}", pass.name()));
                }
                if let Some(clip) = matte {
                    words.push(format!("matte={clip}"));
                }
                if let Some((width, height)) = size {
                    words.push(format!("size={width}x{height}"));
                }
                if let Some(rate) = rate {
                    words.push(format!("rate={rate}"));
                }
                if let Some(from) = from {
                    words.push(format!("from={from}"));
                }
                if let Some(seconds) = seconds {
                    words.push(format!("seconds={seconds}"));
                }
                words
            }
            Self::Generate(ask) => {
                let mut words = vec!["generate".to_owned()];
                words.extend(ask.words());
                words
            }
            Self::Join { parts, to, lap } => {
                let mut words = vec!["join".to_owned()];
                words.extend(parts.iter().map(|part| format!("{{{part}}}")));
                words.push(format!("{{{to}}}"));
                if *lap > 0 {
                    words.push(format!("lap={lap}"));
                }
                words
            }
        }
    }

    /// The file it makes, by the name it has in braces: `out` for the media's own.
    pub fn makes(&self) -> &str {
        match self {
            Self::Render { to, .. } | Self::Join { to, .. } => to,
            Self::Generate(ask) => &ask.out,
        }
    }

    /// The things of the project it names, each with what it is: the sequence a `render`
    /// draws and the clip of its matte, the clips a model is handed (`moves=`, `signal=`),
    /// and every file of the project, as the step writes it. A `{name}`, the cast and
    /// styles, and the model are not of the project's sequences or files, and are left out.
    pub fn names(&self) -> Vec<(Named, &str)> {
        match self {
            Self::Render {
                sequence, matte, ..
            } => std::iter::once((Named::Sequence, sequence.as_str()))
                .chain(matte.iter().map(|clip| (Named::Clip, clip.as_str())))
                .collect(),
            Self::Join { .. } => Vec::new(),
            Self::Generate(ask) => ask
                .sources()
                .filter_map(|source| match source {
                    Source::File(file) => Some((Named::File, file.as_str())),
                    Source::Made(_) => None,
                })
                .chain(
                    ask.signals
                        .iter()
                        .chain(&ask.moves)
                        .map(|clip| (Named::Clip, clip.as_str())),
                )
                .collect(),
        }
    }

    /// Each thing of the project it names ([`Self::names`]) given the name `rename` gives
    /// it, where it gives one: what a step says, said of another project.
    pub fn rename(&mut self, mut rename: impl FnMut(Named, &str) -> Option<String>) {
        let mut one = |named: Named, name: &mut String| {
            if let Some(new) = rename(named, name) {
                *name = new;
            }
        };
        match self {
            Self::Render {
                sequence, matte, ..
            } => {
                one(Named::Sequence, sequence);
                if let Some(clip) = matte {
                    one(Named::Clip, clip);
                }
            }
            Self::Join { .. } => {}
            Self::Generate(ask) => {
                let Ask {
                    guide,
                    guides,
                    first,
                    last,
                    references,
                    workflow,
                    signals,
                    moves,
                    ..
                } = &mut **ask;
                let sources = guide
                    .iter_mut()
                    .chain(guides.iter_mut().map(|(_, source)| source))
                    .chain(first.iter_mut())
                    .chain(last.iter_mut())
                    .chain(references.iter_mut())
                    .chain(workflow.iter_mut());
                for source in sources {
                    if let Source::File(file) = source {
                        one(Named::File, file);
                    }
                }
                for clip in signals.iter_mut().chain(moves.iter_mut()) {
                    one(Named::Clip, clip);
                }
            }
        }
    }

    /// The files earlier steps made that it reads, by the names they have in braces.
    pub fn reads(&self) -> Vec<&str> {
        match self {
            Self::Render { .. } => Vec::new(),
            Self::Join { parts, .. } => parts.iter().map(String::as_str).collect(),
            Self::Generate(ask) => ask
                .sources()
                .filter_map(|source| match source {
                    Source::Made(name) => Some(name.as_str()),
                    Source::File(_) => None,
                })
                .collect(),
        }
    }
}

/// Whether `steps` hold together in the order they are in: each file a step reads in
/// braces is made by one before it, and none follows the step that writes the media's
/// own file. What does not is said by the step's number, from 1.
///
/// Steps that stop short of the media's file are not refused here: a making's steps are
/// written a line at a time, and the making that would run them says what is missing.
pub fn check(steps: &[Step]) -> Result<(), String> {
    let mut made: Vec<&str> = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let number = index + 1;
        if let Some(last) = made.iter().position(|name| *name == OUT) {
            return Err(format!(
                "step {} writes `{{{OUT}}}`, the media's file, so it is the last: step {number} goes before it",
                last + 1
            ));
        }
        for read in step.reads() {
            if !made.contains(&read) {
                return Err(format!(
                    "step {number} reads `{{{read}}}`, which no step before it makes"
                ));
            }
        }
        if let Step::Join { parts, to, .. } = step {
            if parts.contains(to) {
                return Err(format!(
                    "step {number} writes `{{{to}}}`, which is one of the parts it joins"
                ));
            }
        }
        made.push(step.makes());
    }
    Ok(())
}

/// A picture's width and height, in pixels.
type Size = (u32, u32);

/// A size as a line writes it, `1920x1080`: two whole numbers, neither 0.
pub fn size_of(value: &str) -> Option<Size> {
    let (width, height) = value.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
        .filter(|(width, height): &Size| *width > 0 && *height > 0)
}

/// What the words after `render <sequence> {name}` name: the size, the rate, the pass, the
/// clip a matte is of, and the part of the sequence.
type Drawn = (Option<Size>, Option<Rate>, Pass, Option<String>, Option<f64>, Option<f64>);

/// A number of seconds a line gives as `name=value`: `least` or more.
fn seconds_of(name: &str, value: &str, least: f64) -> Result<f64, String> {
    value
        .parse()
        .ok()
        .filter(|seconds: &f64| seconds.is_finite() && *seconds >= least && (least == 0.0 || *seconds > 0.0))
        .ok_or_else(|| format!("`{name}={value}` is not a number of seconds, such as 5 or 5.0625"))
}

/// What the words after `join` name: the parts, the file they make, and how many frames
/// each part after the first has of the one before it.
fn joined(words: &[String], made: &[String]) -> Result<(Vec<String>, String, u32), String> {
    let mut names = Vec::new();
    let mut lap = 0;
    for word in words {
        match (whole(word), word.split_once('=')) {
            (Some(name), _) => names.push(name),
            (None, Some(("lap", value))) => {
                lap = value.parse().map_err(|_| {
                    format!("`lap={value}` is not a whole number of frames, such as 1")
                })?;
            }
            _ => {
                return Err(format!(
                    "`{word}` is not something `join` takes: it is `join {{part}} {{part}} … {{name}} [lap=<frames>]`, each part a file a step before made"
                ))
            }
        }
    }
    let to = names.pop().filter(|_| names.len() >= 2).ok_or_else(|| {
        "`join` sets two or more parts end to end into the file it names last: `join {a.mp4} {b.mp4} {out}`".to_owned()
    })?;
    for part in &names {
        if !made.contains(part) {
            return Err(format!("no step before this one makes `{{{part}}}`"));
        }
    }
    if names.contains(&to) {
        return Err(format!("`join` writes `{{{to}}}`, which is one of its parts"));
    }
    Ok((names, to, lap))
}

fn drawn_at(words: &[String]) -> Result<Drawn, String> {
    let (mut size, mut rate, mut pass, mut matte) = (None, None, Pass::Picture, None);
    let (mut from, mut seconds) = (None, None);
    for word in words {
        match word.split_once('=') {
            Some(("size", value)) => {
                let (width, height) = size_of(value)
                    .ok_or_else(|| format!("`size={value}` is not a size such as 832x480"))?;
                if width % 2 == 1 || height % 2 == 1 {
                    return Err(format!(
                        "`size={value}` has an odd side, which the file cannot have; `size={}x{}` has none",
                        width + width % 2,
                        height + height % 2
                    ));
                }
                size = Some((width, height));
            }
            Some(("rate", value)) => {
                rate = Some(Rate::parse(value).map_err(|_| {
                    format!("`rate={value}` is not a rate in frames a second, such as 16 or 23.976")
                })?);
            }
            Some(("pass", value)) => {
                pass = Pass::parse(value).ok_or_else(|| {
                    format!(
                        "`pass={value}` is not a pass Kerf draws: `ids`, which clip each pixel is of, `depth`, how near it is, `relative`, the same stretched to each frame's own nearest and farthest, or `normals`, which way it faces"
                    )
                })?;
            }
            Some(("matte", value)) if !value.is_empty() => matte = Some(value.to_owned()),
            Some(("from", value)) => from = Some(seconds_of("from", value, 0.0)?),
            Some(("seconds", value)) => seconds = Some(seconds_of("seconds", value, f64::MIN_POSITIVE)?),
            _ => {
                return Err(format!(
                    "`{word}` is not something `render` takes; after the file it makes come `size=WxH`, `rate=<fps>`, `pass=ids|depth|relative|normals`, `matte=<clip>`, and the part of the sequence, `from=<seconds>` and `seconds=<seconds>`"
                ))
            }
        }
    }
    if matte.is_some() && pass != Pass::Picture {
        return Err("`matte=` is one clip in white on black, in place of a pass: give one of the two".to_owned());
    }
    Ok((size, rate, pass, matte, from, seconds))
}

/// The name an argument that is nothing but `{name}` holds.
pub fn whole(argument: &str) -> Option<String> {
    let name = argument.strip_prefix('{')?.strip_suffix('}')?;
    file_name(name).then(|| name.to_owned())
}

/// Whether `name` can be a file a step makes: letters, digits, and `. - _`.
pub fn file_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &[&str]) -> Vec<String> {
        line.iter().map(|word| (*word).to_owned()).collect()
    }

    /// Each step in turn, read with what the ones before it make.
    fn read(lines: &[&[&str]]) -> Result<Vec<Step>, String> {
        let mut made = Vec::new();
        let mut steps = Vec::new();
        for line in lines {
            let step = Step::parse(&words(line), &made)?;
            made.push(step.makes().to_owned());
            steps.push(step);
        }
        Ok(steps)
    }

    #[test]
    fn a_step_is_read_from_its_words_and_written_as_them_again() {
        let steps = read(&[
            &["render", "s2", "{guide.mp4}", "size=832x480", "rate=16", "pass=depth"],
            &["render", "s2", "{mask.mp4}", "matte=crate", "from=5", "seconds=5.0625"],
            &["generate", "model=still", "prompt=an orange \"game\" ball", "seed=3", "out={ball.png}"],
            &[
                "generate",
                "seed=7",
                "strength=0.6",
                "guide={guide.mp4}",
                "guide_mask={mask.mp4}",
                "ref={ball.png}",
                "ref=refs/court.png",
                "model=video",
                "prompt=it bounces",
                "signal=bounce",
                "moves=ball",
                "cast=Ada",
                "style=film",
                "seconds=4",
                "size=832x480",
                "from=5",
                "out={a.mp4}",
            ],
            &["generate", "model=video", "first={a.mp4}", "out={b.mp4}"],
            &["join", "{a.mp4}", "{b.mp4}", "{out}", "lap=1"],
        ])
        .unwrap();
        // Written in one order whatever order they were given in, and with what goes
        // without saying left out.
        assert_eq!(
            steps.iter().map(Step::words).collect::<Vec<_>>(),
            [
                words(&["render", "s2", "{guide.mp4}", "pass=depth", "size=832x480", "rate=16"]),
                words(&["render", "s2", "{mask.mp4}", "matte=crate", "from=5", "seconds=5.0625"]),
                words(&["generate", "model=still", "prompt=an orange \"game\" ball", "seed=3", "out={ball.png}"]),
                words(&[
                    "generate",
                    "model=video",
                    "prompt=it bounces",
                    "guide={guide.mp4}",
                    "guide_mask={mask.mp4}",
                    "ref={ball.png}",
                    "ref=refs/court.png",
                    "cast=Ada",
                    "style=film",
                    "signal=bounce",
                    "moves=ball",
                    "from=5",
                    "seconds=4",
                    "size=832x480",
                    "seed=7",
                    "strength=0.6",
                    "out={a.mp4}",
                ]),
                words(&["generate", "model=video", "first={a.mp4}", "out={b.mp4}"]),
                words(&["join", "{a.mp4}", "{b.mp4}", "{out}", "lap=1"]),
            ]
        );
        // And what is written reads as the step it was.
        let mut made = Vec::new();
        for step in &steps {
            assert_eq!(Step::parse(&step.words(), &made).as_ref(), Ok(step));
            made.push(step.makes().to_owned());
        }
        assert_eq!(check(&steps), Ok(()));
        assert_eq!(steps[3].reads(), ["guide.mp4", "mask.mp4", "ball.png"]);
    }

    #[test]
    fn what_a_step_cannot_be_is_said() {
        let wrong = |line: &[&str]| Step::parse(&words(line), &["a.mp4".to_owned()]).unwrap_err();
        assert!(wrong(&["run", "vork", "render"]).contains("`run` is not a step"));
        assert!(wrong(&["render", "s2"]).contains("`render` is `render <sequence> {name}"));
        assert!(wrong(&["render", "s2", "guide.mp4"]).contains("names the file it makes"));
        assert!(wrong(&["render", "s2", "{g.mp4}", "size=831x480"]).contains("odd side"));
        assert!(wrong(&["render", "s2", "{g.mp4}", "pass=depth", "matte=c"]).contains("give one of the two"));
        assert!(wrong(&["generate", "prompt=a boat"]).contains("names its model"));
        assert!(wrong(&["generate", "model=m", "guide={b.mp4}"]).contains("no step before this one makes `{b.mp4}`"));
        assert!(wrong(&["generate", "model=m", "seed=7", "seed=8"]).contains("given twice"));
        assert!(wrong(&["join", "{a.mp4}", "{out}"]).contains("two or more parts"));
    }

    #[test]
    fn what_a_step_names_of_the_project_is_renamed_and_nothing_else() {
        let mut steps = read(&[
            &["render", "s2", "{mask.mp4}", "matte=crate"],
            &[
                "generate",
                "model=video",
                "guide=footage/walk.mp4",
                "guide_mask={mask.mp4}",
                "ref=refs/a.png",
                "cast=Ada",
                "signal=bounce",
                "moves=ball",
            ],
        ])
        .unwrap();
        assert_eq!(steps[0].names(), [(Named::Sequence, "s2"), (Named::Clip, "crate")]);
        assert_eq!(
            steps[1].names(),
            [
                (Named::File, "footage/walk.mp4"),
                (Named::File, "refs/a.png"),
                (Named::Clip, "bounce"),
                (Named::Clip, "ball"),
            ]
        );
        for step in &mut steps {
            step.rename(|named, name| match named {
                Named::Sequence => Some(format!("{name}x")),
                Named::Clip => Some(format!("{name}2")),
                Named::File => (name == "footage/walk.mp4").then(|| "footage/run.mp4".to_owned()),
            });
        }
        assert_eq!(steps[0].words(), words(&["render", "s2x", "{mask.mp4}", "matte=crate2"]));
        assert_eq!(
            steps[1].words(),
            words(&[
                "generate",
                "model=video",
                "guide=footage/run.mp4",
                "guide_mask={mask.mp4}",
                "ref=refs/a.png",
                "cast=Ada",
                "signal=bounce2",
                "moves=ball2",
            ])
        );
    }

    #[test]
    fn steps_are_held_to_their_order() {
        let steps = read(&[
            &["generate", "model=matte", "guide=footage/walk.mp4", "prompt=man", "out={man.mp4}"],
            &["generate", "model=plate", "guide=footage/walk.mp4", "guide_mask={man.mp4}"],
        ])
        .unwrap();
        assert_eq!(check(&steps), Ok(()));
        // The matte's step gone, the one that read it reads nothing.
        assert_eq!(
            check(&steps[1..]),
            Err("step 1 reads `{man.mp4}`, which no step before it makes".to_owned())
        );
        // Nothing follows the step that writes the media's file.
        let both = [steps[0].clone(), steps[1].clone(), steps[0].clone()];
        assert_eq!(
            check(&both),
            Err("step 2 writes `{out}`, the media's file, so it is the last: step 3 goes before it".to_owned())
        );
        // Steps that stop short of the media's file are a making's to refuse.
        assert_eq!(check(&steps[..1]), Ok(()));
    }
}
