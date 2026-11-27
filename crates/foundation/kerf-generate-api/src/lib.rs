//! The Kerf generation API, version 1: what a model is asked for a file, what a model says it
//! takes, and how a server that reaches models is spoken to. Kerf and its generation server
//! (`services/kerf-gen`) both hold to what is written here, and nowhere else.
//!
//! A `generate` step is asked as a [`GenerateRequest`]. A program on the person's own computer
//! reads it as a file, every file it names a path ([`REQUEST_FILE`]); a server is sent it with
//! each file named by the SHA-256 of its bytes ([`Sha256`]), which it has been sent before:
//!
//! | Method and path | What it does |
//! |---|---|
//! | `GET /v1/models` | the models it reaches, each with what it takes ([`ModelList`]) |
//! | `HEAD`, `PUT /v1/files/{sha256}` | whether it has a file, and the file sent: one it has is not sent again |
//! | `POST /v1/generations` | a request, its files by their SHA-256; answers [`Created`] |
//! | `GET /v1/generations/{id}` | how it is going ([`GenerationState`]) |
//! | `GET /v1/files/{sha256}` | the file made, by the SHA-256 [`GenerationState::Done`] gives |
//! | `POST /v1/generations/{id}/cancel` | stop it |
//!
//! Every request carries the person's key, `Authorization: Bearer <key>`. A request that is
//! refused answers an [`ApiError`].

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt;

/// The version a request says it is.
pub const REQUEST_VERSION: u32 = 1;

/// The request's file, among what a take's steps made.
pub const REQUEST_FILE: &str = "request.json";

/// The header every request to a server carries the key in, as `Bearer <key>`.
pub const AUTHORIZATION: &str = "Authorization";

/// The paths of a server's API, each under `/v1`.
pub mod path {
    /// `GET`: the models it reaches.
    pub const MODELS: &str = "/v1/models";
    /// `POST`: a request.
    pub const GENERATIONS: &str = "/v1/generations";

    /// `HEAD`, `PUT` and `GET`: the file of this SHA-256.
    pub fn file(sha256: &super::Sha256) -> String {
        format!("/v1/files/{sha256}")
    }

    /// `GET`: how the request `id` is going.
    pub fn generation(id: &str) -> String {
        format!("{GENERATIONS}/{id}")
    }

    /// `POST`: stop the request `id`.
    pub fn cancel(id: &str) -> String {
        format!("{GENERATIONS}/{id}/cancel")
    }
}

/// What a `generate` line hands a model, by the name the line hands it by: the files, and what a
/// model reads beside them. A guide beside the first is `guide_<what it is of>`, as `guide_depth`.
pub mod reads {
    pub const GUIDE: &str = "guide";
    pub const FIRST: &str = "first";
    pub const LAST: &str = "last";
    pub const REF: &str = "ref";
    pub const SIGNAL: &str = "signal";
    pub const MOVES: &str = "moves";
    pub const REGIONS: &str = "regions";

    /// The files a line hands a model.
    pub const FILES: [&str; 4] = [GUIDE, FIRST, LAST, REF];

    /// Whether a model can say it reads `name`: one of the above, or a guide by what it is of.
    pub fn known(name: &str) -> bool {
        FILES.contains(&name)
            || [SIGNAL, MOVES, REGIONS].contains(&name)
            || name.strip_prefix("guide_").is_some_and(names_a_guide)
    }

    /// Whether `name` is what a guide beside the first is named by, after `guide_`: lowercase
    /// letters and digits.
    pub fn names_a_guide(name: &str) -> bool {
        !name.is_empty()
            && name
                .chars()
                .all(|letter| letter.is_ascii_lowercase() || letter.is_ascii_digit())
    }
}

/// The SHA-256 of a file's bytes, in lowercase hex: how a server is told of a file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256(String);

impl Sha256 {
    /// `text`, where it is 64 lowercase hex digits.
    pub fn parse(text: &str) -> Option<Self> {
        (text.len() == 64
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        .then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for Sha256 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Sha256 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text)
            .ok_or_else(|| D::Error::custom(format!("`{text}` is not a SHA-256 in lowercase hex")))
    }
}

/// What a `generate` step asks a model for, with each file it names as `F`: a path on this
/// computer ([`String`]) in the request a program reads, or the file's [`Sha256`] in the one a
/// server is sent.
///
/// ```json
/// {
///   "kerf-request": 1,
///   "model": "dawn",
///   "prompt": "a boat leaves the harbour at dawn",
///   "guide": "/project/shots/open.takes/003.work/guide.mp4",
///   "guides": {},
///   "first": "/project/refs/boat.png",
///   "last": null,
///   "references": [],
///   "seconds": 4.0,
///   "size": [1920, 1080],
///   "seed": 7,
///   "signals": [{ "name": "swell", "rate": 24.0, "values": [0.0, 0.25, 0.5] }],
///   "moves": [],
///   "more": { "strength": "0.6" },
///   "out": "/project/shots/open.takes/.making-003.mp4"
/// }
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenerateRequest<F = String> {
    #[serde(rename = "kerf-request")]
    pub version: u32,
    /// The model, by the name the step gives it, or, sent to a server, the server's own.
    pub model: String,
    pub prompt: String,
    pub guide: Option<F>,
    /// The other guides, by what each is of, in the order the step names them. As a JSON
    /// object holds them, a name said twice is there once, with the file it was last given.
    #[serde(with = "in_order")]
    pub guides: Vec<(String, F)>,
    /// The first and the last frame.
    pub first: Option<F>,
    pub last: Option<F>,
    pub references: Vec<F>,
    pub seconds: Option<f64>,
    pub size: Option<[u32; 2]>,
    pub seed: Option<u64>,
    /// Each signal the step names: a number for each frame of the clip's sequence.
    pub signals: Vec<Signal>,
    /// Where each clip the step names goes, frame by frame.
    pub moves: Vec<Moves>,
    /// What clips of the guide say they are, each with where the guide shows them: said only
    /// where some do, so a request of none reads as it did.
    #[serde(default = "Vec::new", skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<Region<F>>,
    /// Whatever else the step sets, by name, as it is written.
    pub more: BTreeMap<String, String>,
    /// Where the file goes: its path, in a program's request, or, sent to a server, a name of
    /// it alone, whose extension says what file is wanted (`out.mp4`, `still.png`).
    pub out: String,
    /// Where in a longer shot this part starts, in seconds. Said only of a part: a request for
    /// a whole one reads as it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<f64>,
}

impl<F> GenerateRequest<F> {
    /// The guide of what `name` is, the last one the step gives it.
    pub fn guide_of(&self, name: &str) -> Option<&F> {
        self.guides
            .iter()
            .rev()
            .find(|(said, _)| said == name)
            .map(|(_, file)| file)
    }

    /// Every file it names, each where it is named.
    pub fn files(&self) -> Vec<&F> {
        let mut files: Vec<&F> = Vec::new();
        files.extend(&self.guide);
        files.extend(self.guides.iter().map(|(_, file)| file));
        files.extend(&self.first);
        files.extend(&self.last);
        files.extend(&self.references);
        files.extend(self.regions.iter().map(|region| &region.mask));
        files
    }

    /// The same request with each file it names made another way by `file`: the first that
    /// cannot be is the error.
    pub fn try_map<G, E>(
        self,
        mut file: impl FnMut(F) -> Result<G, E>,
    ) -> Result<GenerateRequest<G>, E> {
        let optional = |said: Option<F>, file: &mut dyn FnMut(F) -> Result<G, E>| {
            said.map(|said| file(said)).transpose()
        };
        Ok(GenerateRequest {
            version: self.version,
            model: self.model,
            prompt: self.prompt,
            guide: optional(self.guide, &mut file)?,
            guides: self
                .guides
                .into_iter()
                .map(|(name, said)| Ok((name, file(said)?)))
                .collect::<Result<_, E>>()?,
            first: optional(self.first, &mut file)?,
            last: optional(self.last, &mut file)?,
            references: self
                .references
                .into_iter()
                .map(&mut file)
                .collect::<Result<_, E>>()?,
            seconds: self.seconds,
            size: self.size,
            seed: self.seed,
            signals: self.signals,
            moves: self.moves,
            regions: self
                .regions
                .into_iter()
                .map(|region| {
                    Ok(Region {
                        prompt: region.prompt,
                        mask: file(region.mask)?,
                    })
                })
                .collect::<Result<_, E>>()?,
            more: self.more,
            out: self.out,
            from: self.from,
        })
    }
}

impl<F: Serialize> GenerateRequest<F> {
    /// The request as its file holds it. Made a JSON object first, so that its keys are in
    /// the order every object the build writes is: sorted, or, where serde_json keeps order
    /// (the editor's build), the order of the fields above.
    pub fn file(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec_pretty(&serde_json::to_value(self)?)
    }
}

/// Names and their files as an object's entries, in the order they are said.
mod in_order {
    use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};
    use serde::{Serialize, Serializer};
    use std::fmt;
    use std::marker::PhantomData;

    pub fn serialize<F: Serialize, S: Serializer>(
        guides: &[(String, F)],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_map(guides.iter().map(|(name, file)| (name, file)))
    }

    pub fn deserialize<'de, F: Deserialize<'de>, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<(String, F)>, D::Error> {
        struct Entries<F>(PhantomData<F>);
        impl<'de, F: Deserialize<'de>> Visitor<'de> for Entries<F> {
            type Value = Vec<(String, F)>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an object of guides by what each is of")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(entries)
            }
        }
        deserializer.deserialize_map(Entries(PhantomData))
    }
}

/// A signal as a model is handed it: a clip of the project whose value says when something
/// happens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    /// The clip it is, by its id.
    pub name: String,
    /// The frames a second its values are at, which is its sequence's rate.
    pub rate: f64,
    /// What it is on each frame of its sequence, from the first: 0 to 1.
    pub values: Vec<f64>,
}

/// Where a clip goes, as a model is handed it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Moves {
    /// The clip it is, by its id.
    pub name: String,
    /// The frames a second its places are at, which is its sequence's rate.
    pub rate: f64,
    /// How wide and tall its sequence is, in pixels.
    pub frame: (u32, u32),
    /// Where it is on each frame of its sequence, from the first: the middle of its box
    /// across and down, and the box's width and height, in shares of the frame; `null` on a
    /// frame it does not show on.
    pub places: Vec<Option<[f64; 4]>>,
}

/// Words clips of a guide say they are (`prompt=`), and a grey video of where the guide shows
/// those clips, frame by frame: white where they are.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region<F = String> {
    pub prompt: String,
    pub mask: F,
}

/// What a model takes, as whoever reaches it says: the picture it makes, its rate, the longest
/// shot, what of a line it reads, and what else a line can set. Each part is there only where
/// it is said.
///
/// ```json
/// { "size": [832, 480], "rate": 16, "seconds": 5, "reads": ["guide", "ref"] }
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelSpec {
    /// What it makes, in a line.
    pub about: Option<String>,
    /// The picture it makes: the size a guide for it is drawn at, and the most a `size=` asks
    /// of it.
    pub size: Option<(u32, u32)>,
    /// The frames a second it makes.
    pub rate: Option<f64>,
    /// The longest shot it makes at once, in seconds.
    pub seconds: Option<f64>,
    /// What it has a use for of what a line hands it ([`reads`]): none is a model that takes
    /// a prompt alone.
    pub reads: Option<Vec<String>>,
    /// The other names a `generate` line can set for it, each with the value it has when the
    /// line does not.
    pub more: Map<String, Value>,
    /// Whatever else is said of it, as it is written.
    pub other: Map<String, Value>,
}

impl ModelSpec {
    /// Read what a model takes. What is wrong is said by its name, in words a person who wrote
    /// it follows, not serde's: so it is read part by part, as JSON.
    pub fn parse(value: &Value) -> Result<Self, String> {
        let fields = value
            .as_object()
            .ok_or("`takes` is not an object, such as {\"size\": [832, 480], \"rate\": 16}")?;
        let mut spec = Self::default();
        let positive = |name: &str, value: &Value| {
            value
                .as_f64()
                .filter(|number| number.is_finite() && *number > 0.0)
                .ok_or_else(|| format!("`{name}` in `takes` is not a number above 0"))
        };
        for (name, value) in fields {
            match name.as_str() {
                "about" => {
                    spec.about = Some(
                        value
                            .as_str()
                            .ok_or("`about` in `takes` is not text")?
                            .to_owned(),
                    );
                }
                "size" => {
                    spec.size = Some(
                        serde_json::from_value::<(u32, u32)>(value.clone())
                            .ok()
                            .filter(|(width, height)| *width > 0 && *height > 0)
                            .ok_or(
                                "`size` in `takes` is not a width and a height, such as [832, 480]",
                            )?,
                    );
                }
                "rate" => spec.rate = Some(positive(name, value)?),
                "seconds" => spec.seconds = Some(positive(name, value)?),
                "reads" => {
                    let said: Vec<String> = serde_json::from_value(value.clone())
                        .ok()
                        .filter(|said: &Vec<String>| said.iter().all(|read| reads::known(read)))
                        .ok_or_else(|| {
                            format!(
                                "`reads` in `takes` is not a list of what a `generate` line hands a model: {}, {}, {}, {}, and a guide by what it is of, as `guide_depth`",
                                reads::FILES.join(", "),
                                reads::SIGNAL,
                                reads::MOVES,
                                reads::REGIONS
                            )
                        })?;
                    spec.reads = Some(said);
                }
                "more" => {
                    spec.more = value
                        .as_object()
                        .ok_or("`more` in `takes` is not an object of names and their values")?
                        .clone();
                }
                _ => {
                    spec.other.insert(name.clone(), value.clone());
                }
            }
        }
        Ok(spec)
    }

    /// As [`ModelSpec::parse`] reads it: each part said under its name, and the rest after.
    pub fn to_map(&self) -> Map<String, Value> {
        let mut map = Map::new();
        if let Some(about) = &self.about {
            map.insert("about".into(), about.clone().into());
        }
        if let Some((width, height)) = self.size {
            map.insert("size".into(), serde_json::json!([width, height]));
        }
        if let Some(rate) = self.rate {
            map.insert("rate".into(), rate.into());
        }
        if let Some(seconds) = self.seconds {
            map.insert("seconds".into(), seconds.into());
        }
        if let Some(reads) = &self.reads {
            map.insert("reads".into(), reads.clone().into());
        }
        if !self.more.is_empty() {
            map.insert("more".into(), Value::Object(self.more.clone()));
        }
        for (name, value) in &self.other {
            map.entry(name.clone()).or_insert_with(|| value.clone());
        }
        map
    }

    /// Whether it follows a guide, where it says what it reads.
    pub fn follows_a_guide(&self) -> Option<bool> {
        self.reads
            .as_ref()
            .map(|said| said.iter().any(|read| read == reads::GUIDE))
    }
}

impl Serialize for ModelSpec {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_map().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ModelSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&Value::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// A model a server reaches: its name, and what it takes.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedModel {
    pub name: String,
    pub spec: ModelSpec,
}

impl Serialize for NamedModel {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = Map::new();
        map.insert("name".into(), self.name.clone().into());
        map.extend(self.spec.to_map());
        map.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for NamedModel {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut map = Map::<String, Value>::deserialize(deserializer)?;
        let name = match map.remove("name") {
            Some(Value::String(name)) if !name.is_empty() => name,
            _ => return Err(D::Error::custom("a model has no `name`")),
        };
        let spec = ModelSpec::parse(&Value::Object(map))
            .map_err(|what| D::Error::custom(format!("model `{name}`: {what}")))?;
        Ok(Self { name, spec })
    }
}

/// What `GET /v1/models` answers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelList {
    pub models: Vec<NamedModel>,
}

/// What `POST /v1/generations` answers: the request's id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Created {
    pub id: String,
}

/// How a request is going, as `GET /v1/generations/{id}` answers it. Once it is done, has
/// failed or was cancelled ([`GenerationState::finished`]) it is so from then on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum GenerationState {
    /// Waiting for the requests before it.
    Queued,
    /// Under way: how far, from 0 to 1, and the last thing it said, where it says them.
    Running {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        progress: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        line: Option<String>,
    },
    /// Made: the file, by its SHA-256.
    Done { out: Sha256 },
    /// It could not be made, and why.
    Failed { message: String },
    /// Stopped when asked to.
    Cancelled,
}

impl GenerationState {
    /// Whether it is over: done, failed or cancelled, and so from then on.
    pub fn finished(&self) -> bool {
        matches!(
            self,
            Self::Done { .. } | Self::Failed { .. } | Self::Cancelled
        )
    }
}

/// What a request that is refused answers, with any status that is not a success.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}
