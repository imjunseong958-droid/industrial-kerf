//! Every shot of a project a recipe makes, in one reading: which take each
//! is, how many it has, who is in it, and whether it wants making.
//!
//! A project with a dozen made shots is otherwise asked about one media at
//! a time. This reads the takes' folders and records and runs nothing.

use super::takes::Standing;
use super::Unmade;
use kerf_model::{Content, Project};
use std::path::Path;

/// One media a recipe makes, as things stand.
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    /// The media's id, and the recipe file its steps are still in, where they have not
    /// been carried into the document: such a shot is listed, and nothing makes it.
    pub media: String,
    pub recipe: Option<String>,
    /// How long the media is, in seconds.
    pub seconds: f64,
    /// How many clips of the project play it.
    pub clips: usize,
    /// How many takes it has, and which one the media is, where it is one.
    pub takes: usize,
    pub chosen: Option<u32>,
    /// The cast and the styles its recipe names, where the recipe reads.
    pub cast: Vec<String>,
    pub styles: Vec<String>,
    /// Why it wants making, where it does.
    pub wants: Option<Unmade>,
}

/// The shots of `project`, kept in `directory`, in the order of its media.
pub fn shots(project: &Project, directory: &Path) -> Vec<Shot> {
    project
        .media
        .iter()
        .filter_map(|media| {
            let recipe = super::uncarried(media).map(str::to_owned);
            if media.steps.is_empty() && recipe.is_none() {
                return None;
            }
            // The steps are read, and what they read hashed, once for both questions.
            let standing = Standing::of(project, directory, media);
            let made = match &standing {
                Some(standing) => standing.takes().ok(),
                None => super::takes(project, directory, media).ok(),
            }
            .unwrap_or_default();
            let (cast, styles) = super::cast::named_by(media);
            Some(Shot {
                media: media.id.to_string(),
                seconds: media.duration.seconds(),
                clips: project
                    .sequences
                    .iter()
                    .flat_map(|sequence| &sequence.tracks)
                    .flat_map(|track| &track.clips)
                    .filter(|clip| match &clip.content {
                        Content::Media { media: played, .. } => *played == media.id,
                        // A mesh clip shows a set a model made by reading its file.
                        Content::Generator { params, .. } => params.values().any(|param| {
                            matches!(param, kerf_model::Param::Static(kerf_model::Value::Text(read)) if *read == media.path)
                        }),
                        _ => false,
                    })
                    .count(),
                takes: made.len(),
                chosen: made.iter().find(|take| take.chosen).map(|take| take.number),
                cast,
                styles,
                wants: super::unmade(project, directory, media),
                recipe,
            })
        })
        .collect()
}

impl Shot {
    /// The shot as a line of `kerf shots`.
    pub fn line(&self) -> String {
        let take = match (self.chosen, self.takes) {
            (_, 0) => "no take".to_owned(),
            (Some(chosen), takes) => format!("take {chosen} of {takes}"),
            (None, takes) => format!("none of its {takes} takes"),
        };
        let mut line = format!(
            "{} {}  {take}  {}",
            if self.wants.is_some() { "!" } else { " " },
            self.media,
            seconds(self.seconds)
        );
        let shows: Vec<&str> = self
            .cast
            .iter()
            .chain(&self.styles)
            .map(String::as_str)
            .collect();
        if !shows.is_empty() {
            line.push_str(&format!("  {}", shows.join(", ")));
        }
        if self.clips == 0 {
            line.push_str("  (no clip plays it)");
        }
        match (&self.recipe, &self.wants) {
            // Its steps are in a recipe's file: what it wants is carrying.
            (Some(recipe), _) => line.push_str(&format!(
                "  to carry: its steps are in {recipe}; `kerf carry {}` writes them under it",
                self.media
            )),
            (None, Some(wants)) => line.push_str(&format!("  to make: {}", wants.reason())),
            (None, None) => {}
        }
        line
    }
}

fn seconds(seconds: f64) -> String {
    // No seconds at all add up to a zero that prints its sign.
    let text = format!("{:.2}", seconds + 0.0);
    format!("{}s", text.trim_end_matches('0').trim_end_matches('.'))
}

/// What `shots` come to, as the last line of `kerf shots`.
pub fn summary(shots: &[Shot]) -> String {
    let wanting: Vec<&Shot> = shots.iter().filter(|shot| shot.wants.is_some()).collect();
    let made: f64 = shots
        .iter()
        .filter(|shot| shot.wants.is_none())
        .map(|shot| shot.seconds)
        .sum();
    let all: f64 = shots.iter().map(|shot| shot.seconds).sum();
    let count = |number: usize| format!("{number} shot{}", if number == 1 { "" } else { "s" });
    if wanting.is_empty() {
        format!("{}, {}, all made", count(shots.len()), seconds(all))
    } else {
        format!(
            "{}, {} to make ({}); {} of {} made",
            count(shots.len()),
            wanting.len(),
            wanting
                .iter()
                .map(|shot| shot.media.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            seconds(made),
            seconds(all)
        )
    }
}
