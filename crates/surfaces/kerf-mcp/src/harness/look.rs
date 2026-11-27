//! `look`: which reading its arguments ask for, refused where they do not go together, and
//! what it reads beside the project: the catalog's effects and clip properties, and fonts.

use super::tools::{input, CutsWith, EyeWith, LookInput, Toggle};
use super::{catalog, program, MAX_LOOK_FRAMES, MAX_SHEET_FRAMES};
use crate::{engine_error, KerfMcpRouter, McpCode, McpError};
use kerf_lang::{print_document, PrintOptions};
use kerf_model::Project;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const DEFAULT_FRAME_WIDTH: u32 = 960;
const DEFAULT_SHEET_TILE_WIDTH: u32 = 320;

/// The most catalog entries `look` with `effects` returns.
const MAX_FOUND_FX: usize = 20;

/// How many clip properties `effects` gives.
const MAX_FOUND_PROPERTIES: usize = 6;

impl KerfMcpRouter {
    pub(crate) fn look(&self, arguments: Value) -> Result<Value, McpError> {
        let preset_only = arguments.as_object().is_some_and(|o| o.len() == 1);
        let beside_eye = arguments.as_object().is_some_and(|given| {
            given.keys().all(|name| {
                matches!(
                    name.as_str(),
                    "eye" | "sequence" | "proposal" | "from" | "to"
                )
            })
        });
        let only_area_beside = arguments.as_object().is_some_and(|o| o.len() == 2);
        let input: LookInput = input(arguments)?;
        if let Some(found) = self.look_up(&input, preset_only, only_area_beside)? {
            return Ok(found);
        }
        if input.media.is_some() && input.sequence.is_some() {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                "`sequence` chooses program frames or what a sequence says; not with `media`",
            ));
        }
        if let Some(read) = self.look_named(&input, beside_eye)? {
            return Ok(read);
        }
        asked_together(&input)?;
        if input.media.is_none()
            && input.at.is_empty()
            && !input.transcript
            && !input.contrast
            && !input.loudness
            && matches!(input.cuts, None | Some(Toggle::Off))
            && input.track.is_none()
            && input.sheet.is_none()
            && input.sequence.is_none()
            && input.proposal.is_none()
        {
            return self.overview();
        }
        self.look_at_project(&input)
    }

    /// `look` at what is read beside the project, each alone: a font, effects found by what
    /// they do, an effect or a clip property, a file of the project's folder, or a preset.
    /// None where none of them is asked for.
    fn look_up(
        &self,
        input: &LookInput,
        preset_only: bool,
        only_area_beside: bool,
    ) -> Result<Option<Value>, McpError> {
        let area = match input.area.as_deref() {
            Some(name) => Some(kerf_model::Area::parse(name).ok_or_else(|| {
                McpError::new(
                    McpCode::InvalidArguments,
                    format!("no area `{name}`: edit, color, motion, vfx, or image"),
                )
            })?),
            None => None,
        };
        if let Some(font) = &input.font {
            if !preset_only {
                return Err(McpError::new(McpCode::InvalidArguments, "use font alone"));
            }
            return Ok(Some(font_description(font)));
        }
        if let Some(words) = &input.effects {
            if !(preset_only || area.is_some() && only_area_beside) {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    "use effects alone, or with area",
                ));
            }
            return Ok(Some(
                json!({"effects": find_fx(words, area), "properties": find_properties(words)}),
            ));
        }
        if area.is_some() {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                "area narrows effects; pass effects too",
            ));
        }
        if let Some(id) = &input.effect {
            if !preset_only {
                return Err(McpError::new(McpCode::InvalidArguments, "use effect alone"));
            }
            return effect_reference(id).map(Some);
        }
        if let Some(file) = &input.file {
            if !preset_only {
                return Err(McpError::new(McpCode::InvalidArguments, "use file alone"));
            }
            return self.look_at_file(file).map(Some);
        }
        if input.preset.is_some() || input.presets {
            if !preset_only {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    "use preset or presets alone",
                ));
            }
            let found = match &input.preset {
                Some(file) => self
                    .inspect_preset(&self.project_at(None)?.1, file)
                    .map(|inspection| json!(inspection)),
                None => self.list_presets(),
            };
            return found.map(Some);
        }
        Ok(None)
    }

    /// `look` at what one argument names, read alone or with the few that say of which
    /// sequence, proposal, or range: where the eye is across cuts, what frames cost, a
    /// signal's keys from a sound, a take held to its guide, a shot, a mesh's file, what shows
    /// and what stands, and recordings lined up. None where none of them is asked for.
    fn look_named(&self, input: &LookInput, beside_eye: bool) -> Result<Option<Value>, McpError> {
        if let Some(subjects) = eye_subjects(input.eye.as_ref())? {
            if !beside_eye {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    "`eye` is of the cuts of a sequence's picture, and is used alone, or with `sequence`, `proposal`, `from`, or `to`",
                ));
            }
            let (revision, project) = self.project_at(input.proposal.as_deref())?;
            return self
                .cuts_seen(&project, revision, input, &subjects)
                .map(Some);
        }
        if input.cost {
            if input.media.is_some() {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    "`cost` is of program frames, not with `media`",
                ));
            }
            if input.at.is_empty() {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    "`cost` is of the frames at `at`: pass the positions to read",
                ));
            }
            let (revision, project) = self.project_at(input.proposal.as_deref())?;
            return self
                .frame_costs(&project, input.sequence.as_deref(), revision, &input.at)
                .map(Some);
        }
        if let Some(signal) = &input.signal {
            if input.media.is_some() || !input.at.is_empty() || input.sequence.is_some() {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    "`signal` names its clips, and is used alone or with `proposal`",
                ));
            }
            let (revision, project) = self.project_at(input.proposal.as_deref())?;
            return self.signal_from_sound(&project, revision, signal).map(Some);
        }
        if let Some(take) = &input.follows {
            if input.media.is_some() || !input.at.is_empty() {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    "`follows` names its media, and is used alone, or with `sequence` or `proposal`",
                ));
            }
            let (revision, project) = self.project_at(input.proposal.as_deref())?;
            return self
                .take_follows(&project, input.sequence.as_deref(), revision, take)
                .map(Some);
        }
        if let Some(held) = &input.held {
            if input.media.is_some() || !input.at.is_empty() {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    "`held` names a take and its depth, and is used alone, or with `sequence` or `proposal`",
                ));
            }
            let (revision, project) = self.project_at(input.proposal.as_deref())?;
            return self
                .take_held(&project, input.sequence.as_deref(), revision, held)
                .map(Some);
        }
        if let Some(placed) = &input.placed {
            if input.media.is_some() || !input.at.is_empty() {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    "`placed` names its take and its clip, and is used alone, or with `sequence` or `proposal`",
                ));
            }
            let (revision, project) = self.project_at(input.proposal.as_deref())?;
            return self
                .take_placed(&project, input.sequence.as_deref(), revision, placed)
                .map(Some);
        }
        if input.graph.is_some() || input.takes.is_some() {
            return self.look_at_shot(input).map(Some);
        }
        if let Some(mesh) = &input.mesh {
            if input.media.is_some() || !input.at.is_empty() || input.sequence.is_some() {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    "`mesh` names its clip, and is used alone or with `proposal`",
                ));
            }
            let (revision, project) = self.project_at(input.proposal.as_deref())?;
            return self.mesh_file(&project, revision, mesh).map(Some);
        }
        if input.shows || input.stands {
            return self.look_shows(input).map(Some);
        }
        if !input.sync.is_empty() {
            let (revision, project) = self.project_at(input.proposal.as_deref())?;
            return self.line_up(&project, revision, &input.sync).map(Some);
        }
        Ok(None)
    }

    /// `look` with `graph` or `takes`: a shot as the editor's graph shows it, or its takes
    /// held to its guide.
    fn look_at_shot(&self, input: &LookInput) -> Result<Value, McpError> {
        if (input.graph.is_some() && input.takes.is_some())
            || input.media.is_some()
            || !input.at.is_empty()
            || input.sequence.is_some()
        {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                "`graph` and `takes` name a media: each is used alone, or with `proposal`",
            ));
        }
        let (revision, project) = self.project_at(input.proposal.as_deref())?;
        let mut result = match (&input.graph, &input.takes) {
            (Some(media), _) => self.shot_graph(&project, media)?,
            (_, Some(media)) => self.shot_takes(&project, media)?,
            _ => unreachable!("one of them is given"),
        };
        result["revision"] = json!(revision);
        if let Some(id) = &input.proposal {
            result["proposal"] = json!(id);
        }
        Ok(result)
    }

    /// `look` with `shows` or `stands`: what shows of each clip on the frames at `at`, what
    /// each clip in depth stands on, or both.
    fn look_shows(&self, input: &LookInput) -> Result<Value, McpError> {
        if input.media.is_some() {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                if input.shows {
                    "`shows` is of program frames, not with `media`"
                } else {
                    "`stands` is of the clips in depth of a program's sequence, not with `media`"
                },
            ));
        }
        if input.shows && input.at.is_empty() {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                "`shows` is of the frames at `at`: pass the positions to read",
            ));
        }
        let (revision, project) = self.project_at(input.proposal.as_deref())?;
        let sequence = input.sequence.as_deref();
        let mut result = if input.shows {
            self.frames_show(&project, sequence, revision, &input.at)?
        } else {
            json!({
                "sequence": program(&project, sequence)?.id.to_string(),
                "revision": revision,
            })
        };
        if input.stands {
            result["stands"] = self.clips_stand(&project, sequence, &input.at)?;
        }
        Ok(result)
    }

    /// `look` at the sequence or a media of the project, or of what a pending proposal would
    /// make of it.
    fn look_at_project(&self, input: &LookInput) -> Result<Value, McpError> {
        let (revision, project) = self.project_at(input.proposal.as_deref())?;
        let default_width = if input.sheet.is_some() {
            DEFAULT_SHEET_TILE_WIDTH
        } else {
            DEFAULT_FRAME_WIDTH
        };
        let width = input.width.unwrap_or(default_width).clamp(64, 1920);
        if input.loudness {
            // Of the sequence only, which it says of a look that asks more.
            return self.program_loudness(&project, revision, input);
        }
        if let Some(pauses) = cuts_asked(input.cuts.as_ref())? {
            return self.program_cuts(&project, revision, input, pauses);
        }
        let mut result = match input.media.as_deref() {
            None if input.contrast => self.program_contrast(&project, revision, input)?,
            None if input.track.is_some() => self.program_track(&project, revision, input)?,
            None if input.transcript => self.program_transcript(&project, revision, input)?,
            None if input.sheet.is_some() => {
                self.program_sheet(&project, revision, input, width)?
            }
            None if input.at.is_empty() => {
                self.document_shown(&project, revision, input.proposal.is_some())
            }
            None if input.scopes => self.scopes_beside(&project, revision, input)?,
            None => self.program_frames(
                &project,
                input.sequence.as_deref(),
                revision,
                &input.at,
                width,
            )?,
            Some(media_id) => self.look_at_media(&project, media_id, input, width)?,
        };
        if let Some(id) = &input.proposal {
            result["proposal"] = json!(id);
        }
        Ok(result)
    }

    /// The annotated document of `project`, and what it would have made: `waiting` where it
    /// is what a proposal would make, which makes nothing until it is accepted.
    fn document_shown(&self, project: &Project, revision: u64, waiting: bool) -> Value {
        let mut shown = json!({
            "revision": revision,
            "document": print_document(project, catalog(), PrintOptions { annotate: true }),
        });
        // What it would have made, as `look` with no arguments says of the project:
        // of a proposal, with what would refuse the making once it is accepted.
        let to_make = self.to_make(project, waiting);
        if !to_make.is_empty() {
            shown["toMake"] = json!(to_make);
        }
        shown
    }

    /// `look` with `scopes`: what they read at `at`, of a proposal beside what the project
    /// reads there.
    fn scopes_beside(
        &self,
        project: &Project,
        revision: u64,
        input: &LookInput,
    ) -> Result<Value, McpError> {
        // Of a proposal, with what the project reads there beside it.
        let before = match &input.proposal {
            Some(_) => self.project_at(None).ok().map(|(_, project)| project),
            None => None,
        };
        self.program_scopes(
            project,
            before.as_ref(),
            input.sequence.as_deref(),
            revision,
            &input.at,
        )
    }

    /// `look` with `media`: a contact sheet of it, its index, or its frames at `at`.
    fn look_at_media(
        &self,
        project: &Project,
        media_id: &str,
        input: &LookInput,
        width: u32,
    ) -> Result<Value, McpError> {
        let media = project.media(media_id).ok_or_else(|| {
            McpError::new(
                McpCode::UnknownMedia,
                format!(
                    "no media `{media_id}` in the project{}",
                    self.in_pending(media_id, "proposal")
                ),
            )
        })?;
        if input.sheet.is_some() {
            self.source_sheet(media, input, width)
        } else if input.at.is_empty() {
            self.media_index(project, media, input)
        } else {
            self.source_frames(media, &input.at, width)
        }
    }
}

/// Refuses a `look` at the sequence or a media whose arguments do not go together, before
/// anything is read.
fn asked_together(input: &LookInput) -> Result<(), McpError> {
    if input.track.is_some() && (input.media.is_some() || !input.at.is_empty()) {
        return Err(McpError::new(
            McpCode::InvalidArguments,
            "track follows the program over `from`–`to`; not with `media` or `at`",
        ));
    }
    let ranged = input.transcript
        || input.sheet.is_some()
        || input.media.is_some()
        || input.track.is_some()
        || input.contrast
        || cuts_asked(input.cuts.as_ref())?.is_some();
    if !ranged && (input.from.is_some() || input.to.is_some()) {
        return Err(McpError::new(
            McpCode::InvalidArguments,
            "from and to choose a range for `transcript`, `sheet`, `contrast`, `cuts`, `eye`, or a media's onsets",
        ));
    }
    if let Some(count) = input.sheet {
        if !(2..=MAX_SHEET_FRAMES).contains(&count) {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                format!("a sheet has 2 to {MAX_SHEET_FRAMES} frames"),
            ));
        }
        if input.transcript || !input.at.is_empty() {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                "pass one of `sheet`, `at`, or `transcript`",
            ));
        }
    }
    if input.media.is_none() && input.transcript && !input.at.is_empty() {
        return Err(McpError::new(
            McpCode::InvalidArguments,
            "pass `at` for frames or `transcript` for what is said, not both",
        ));
    }
    if input.words && !input.transcript {
        return Err(McpError::new(
            McpCode::InvalidArguments,
            "words times a transcript; pass transcript: true",
        ));
    }
    if input.scopes
        && (input.at.is_empty()
            || input.media.is_some()
            || input.transcript
            || input.sheet.is_some())
    {
        return Err(McpError::new(
            McpCode::InvalidArguments,
            "scopes read the program at `at`; pass `at`, and not `media`, `transcript`, or `sheet`",
        ));
    }
    if input.facts && input.media.is_none() {
        return Err(McpError::new(
            McpCode::InvalidArguments,
            "facts are of a media; pass `media`",
        ));
    }
    if input.contrast
        && (input.media.is_some()
            || !input.at.is_empty()
            || input.transcript
            || input.sheet.is_some()
            || input.track.is_some()
            || input.scopes)
    {
        return Err(McpError::new(
            McpCode::InvalidArguments,
            "contrast reads the sequence's text; pass only `from`, `to`, `sequence`, or `proposal`",
        ));
    }
    if input.at.len() > MAX_LOOK_FRAMES {
        return Err(McpError::new(
            McpCode::InvalidArguments,
            format!("look shows at most {MAX_LOOK_FRAMES} frames at a time"),
        ));
    }
    // A transcript nothing here can make is refused now, not after the media is indexed
    // or the program's sound rendered for it.
    if input.transcript {
        kerf_engine::transcription_available().map_err(engine_error)?;
    }
    Ok(())
}

/// What `cuts` is given: nothing where it is not asked for, and else from what length a pause
/// is listed, in seconds.
fn cuts_asked(given: Option<&Toggle<CutsWith>>) -> Result<Option<f64>, McpError> {
    let wrong = || {
        McpError::new(
            McpCode::InvalidArguments,
            format!(
                "`cuts` is `true`, or says from what length a pause is listed, as {{\"pauses\": 0.5}}: \
                 seconds, {} or more",
                crate::cuts::LEAST_PAUSE_SECONDS
            ),
        )
    };
    match given {
        None | Some(Toggle::Off) => Ok(None),
        Some(Toggle::On | Toggle::With(CutsWith { pauses: None })) => {
            Ok(Some(crate::cuts::KEPT_PAUSE_SECONDS))
        }
        Some(Toggle::With(CutsWith {
            pauses: Some(seconds),
        })) if *seconds >= crate::cuts::LEAST_PAUSE_SECONDS => Ok(Some(*seconds)),
        Some(Toggle::With(_) | Toggle::Wrong) => Err(wrong()),
    }
}

/// What `eye` is given: nothing where it is not asked for, and else what the eye is on in
/// each shot it names, a clip of the shot's guide by the shot's media.
fn eye_subjects(
    given: Option<&Toggle<EyeWith>>,
) -> Result<Option<BTreeMap<String, String>>, McpError> {
    let wrong = || {
        McpError::new(
            McpCode::InvalidArguments,
            "`eye` is `true`, or names what the eye is on in a shot whose steps do not, as {\"subjects\": {\"<media>\": \"<clip of its guide>\"}}",
        )
    };
    match given {
        None | Some(Toggle::Off) => Ok(None),
        Some(Toggle::On) => Ok(Some(BTreeMap::new())),
        Some(Toggle::With(EyeWith { subjects })) => Ok(Some(subjects.clone().unwrap_or_default())),
        Some(Toggle::Wrong) => Err(wrong()),
    }
}

/// What `look` says of a font: its faces, their weights, axes and the OpenType features each has,
/// or the families a name no font has could mean. A face's own name ("Inter Black") or a family
/// written with a style ("NanumSquare Bold") is its family, with the style it names.
fn font_description(wanted: &str) -> Value {
    use kerf_text::fonts;
    // Four places say all a font does, where a float says it with noise.
    let place = |value: f32| (f64::from(value) * 10000.0).round() / 10000.0;
    let named = fonts::family_named(crate::text_lint::family_of(wanted));
    let faces = named
        .as_ref()
        .map(|(family, _)| fonts::faces_of(family))
        .unwrap_or_default();
    if faces.is_empty() {
        fonts::scan_system();
        let words = wanted.trim().to_lowercase();
        // A name with a dot in front is the system's own, and it does not offer them.
        let families: Vec<String> = fonts::families()
            .into_iter()
            .map(|(name, _)| name)
            .filter(|name| !name.starts_with('.') && name.to_lowercase().contains(&words))
            .collect();
        return json!({"font": wanted, "found": false, "families": families});
    }
    let faces: Vec<Value> = faces
        .iter()
        .map(|face| {
            let axes: Vec<Value> = face
                .axes()
                .iter()
                .map(|axis| {
                    json!({
                        "tag": String::from_utf8_lossy(&axis.tag),
                        "min": place(axis.min),
                        "default": place(axis.default),
                        "max": place(axis.max),
                    })
                })
                .collect();
            let mut described = json!({
                "style": face.info.style,
                "weight": face.info.weight,
                "italic": face.info.italic,
                "axes": axes,
                "features": face.features().iter().map(|tag| String::from_utf8_lossy(tag).into_owned()).collect::<Vec<_>>(),
            });
            if face.info.color {
                described["color"] = json!(true);
            }
            described
        })
        .collect();
    let (family, style) = named.unwrap_or_default();
    let mut described = json!({"font": family, "found": true, "faces": faces});
    if let Some(style) = style {
        described["style"] = json!(style);
    }
    described
}

/// What `look` with `effect` says of `id`: a clip property's reference, or every parameter of
/// an effect, transition, or generator of the catalog.
fn effect_reference(id: &str) -> Result<Value, McpError> {
    if let Some(property) = clip_properties().find(|param| param.name == id) {
        return Ok(json!({
            "property": property.name,
            "reference": kerf_lang::property_reference(property),
        }));
    }
    // The reference names each with its version, as documents do: `title@1` is `title`,
    // and a version this Kerf does not read is said as a document's would be.
    let (id, version) = match id.split_once('@') {
        Some(_) => {
            let (id, version) = kerf_lang::parse_fx_ref(id)
                .map_err(|message| McpError::new(McpCode::InvalidArguments, message))?;
            (id.to_string(), version)
        }
        None => (id.to_owned(), None),
    };
    let id = id.as_str();
    let fx = catalog().fx(id).ok_or_else(|| {
        let message = match kerf_model::catalog::retired(id) {
            Some(_) => kerf_model::catalog::unknown("effect", id),
            None => {
                let names = catalog()
                    .all()
                    .map(|fx| fx.id.as_str())
                    .chain(clip_properties().map(|param| param.name.as_str()));
                match kerf_model::catalog::nearest(id, names) {
                    Some(near) => {
                        format!("no effect `{id}` in the catalog; did you mean `{near}`?")
                    }
                    None => format!(
                        "no effect `{id}` in the catalog; `effects` finds one by what it does"
                    ),
                }
            }
        };
        McpError::new(McpCode::InvalidArguments, message)
    })?;
    if let Some(version) = version {
        fx.reads(version)
            .map_err(|message| McpError::new(McpCode::InvalidArguments, message))?;
    }
    Ok(json!({
        "effect": fx.id.as_str(),
        "category": fx.category,
        "area": fx.area().name(),
        "engine": fx.engine.name(),
        "reference": kerf_lang::fx_reference(fx),
    }))
}

/// Catalog entries matching `words`, by how many of the words their id,
/// category, and doc contain, then in catalog order.
fn find_fx(words: &str, area: Option<kerf_model::Area>) -> Vec<Value> {
    let words: Vec<String> = words
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut found: Vec<(usize, &kerf_model::FxDef)> = catalog()
        .all()
        .filter(|fx| area.is_none_or(|area| fx.area() == area))
        .filter_map(|fx| {
            // What its parameters are for counts too: a light is what casts a shadow.
            let params: String = fx
                .params
                .iter()
                .map(|param| format!(" {} {}", param.name, param.doc))
                .collect();
            let haystack = format!("{} {} {}{params}", fx.id, fx.category, fx.doc).to_lowercase();
            let whole: Vec<&str> = haystack
                .split(|c: char| !c.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .collect();
            // A word found whole counts twice what one found inside another
            // does: `glow` over `glowing`.
            let score: usize = words
                .iter()
                .map(|word| {
                    if whole.contains(&word.as_str()) {
                        2
                    } else if haystack.contains(word.as_str()) {
                        1
                    } else {
                        0
                    }
                })
                .sum();
            (score > 0).then_some((score, fx))
        })
        .collect();
    found.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    found
        .into_iter()
        .take(MAX_FOUND_FX)
        .map(|(_, fx)| {
            json!({
                "id": fx.id.as_str(),
                "kind": fx.kind.name(),
                "category": fx.category,
                "area": fx.area().name(),
                "doc": fx.doc,
            })
        })
        .collect()
}

/// The clip properties, of pictures and of sound.
fn clip_properties() -> impl Iterator<Item = &'static kerf_model::ParamDef> {
    [kerf_model::Domain::Video, kerf_model::Domain::Audio]
        .into_iter()
        .flat_map(|domain| catalog().props(domain).iter())
}

/// The clip properties whose name or guidance match `words`, best first: what no effect does, such as a
/// `bevel`, is found the way an effect is.
fn find_properties(words: &str) -> Vec<Value> {
    let words: Vec<String> = words
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut found: Vec<(usize, &kerf_model::ParamDef)> = clip_properties()
        .filter_map(|param| {
            let haystack = format!("{} {}", param.name, param.doc).to_lowercase();
            let score = words
                .iter()
                .filter(|word| haystack.contains(word.as_str()))
                .count();
            (score > 0).then_some((score, param))
        })
        .collect();
    found.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    found
        .into_iter()
        .take(MAX_FOUND_PROPERTIES)
        .map(|(_, param)| json!({"id": param.name, "reference": kerf_lang::property_reference(param)}))
        .collect()
}
