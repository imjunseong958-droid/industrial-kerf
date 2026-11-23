//! What an export asks kerfd for and where it goes: the qualities the
//! toolbar's Export renders, the request for each, and where the project's
//! master is kept to go.

use crate::prefs::{MasterOutput, Prefs};
use kerf_bridge::event::{RenderCodec, RenderQuality};
use kerf_bridge::{Command, ExportRequest};
use std::path::Path;

/// Where Finish's deliveries go, inside the project.
pub const DELIVERIES: &str = "deliveries/";

/// The `export` request for `quality` in `project`. A master (`master`)
/// or a review copy (`review`, H.264) goes into the project's deliveries/,
/// named after its folder: `<folder>_master`, `<folder>_draft`. Another
/// quality (`full`, `preview`), or a project with no folder name, goes
/// where kerfd puts it, under its own name.
pub fn export_request(project: Option<&Path>, quality: &str) -> Command {
    let quality = RenderQuality::from_name(quality);
    let folder = project
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.trim().is_empty());
    let delivered = match quality {
        RenderQuality::Master => Some("master"),
        RenderQuality::Review => Some("draft"),
        _ => None,
    };
    let mut request = kerf_bridge::ExportRequest {
        codec: (quality == RenderQuality::Review).then_some(RenderCodec::H264),
        quality,
        ..kerf_bridge::ExportRequest::default()
    };
    if let (Some(folder), Some(suffix)) = (folder, delivered) {
        request.output = Some(DELIVERIES.to_owned());
        request.name = Some(format!("{folder}_{suffix}"));
    }
    Command::Export(request)
}

/// What the toolbar's Export renders in one press, as its menu chooses it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    /// The finished file: ProRes 422 HQ, unless the project keeps
    /// another codec for it ([`master_makes`]).
    #[default]
    Master,
    /// A review copy: H.264.
    Draft,
}

impl Quality {
    pub const ALL: [Quality; 2] = [Quality::Master, Quality::Draft];

    /// As the bridge's `export` takes it.
    pub fn wire(self) -> &'static str {
        match self {
            Quality::Master => "master",
            Quality::Draft => "review",
        }
    }

    /// As File ▸ Export… asks the registry's `file.export` for it: the
    /// full-size render (`full`) for Master, `preview` for Draft, both
    /// where kerfd puts them rather than into deliveries/.
    pub fn file_wire(self) -> &'static str {
        match self {
            Quality::Master => "full",
            Quality::Draft => "preview",
        }
    }

    /// As people read it, after "Export" on the button.
    pub fn name(self) -> &'static str {
        match self {
            Quality::Master => "Master",
            Quality::Draft => "Draft",
        }
    }

    /// What it makes, as its row in the menu says: the master's own
    /// codec, which [`master_makes`] gives way to when the project keeps
    /// another.
    pub fn makes(self) -> &'static str {
        match self {
            Quality::Master => "ProRes 422 HQ",
            Quality::Draft => "H.264 review",
        }
    }

    /// What the button does with it, as its tooltip says.
    pub fn action(self) -> &'static str {
        match self {
            Quality::Master => "Export master",
            Quality::Draft => "Export draft",
        }
    }
}

/// The export command that renders the project's master:
/// `{type: export, quality: master}`, to
/// deliveries/ as `<project>_master` unless `kept` (the project's
/// [`MasterOutput`]) names another folder (empty: where kerfd puts it) or
/// file name; its codec when it is not ProRes 422 HQ, a master's own.
pub fn master_request(project: Option<&Path>, kept: Option<&MasterOutput>) -> Command {
    let folder_name = project
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.trim().is_empty());
    let mut folder = folder_name
        .as_ref()
        .map(|_| DELIVERIES.to_owned())
        .unwrap_or_default();
    let mut name = folder_name
        .as_ref()
        .map(|project| format!("{project}_master"))
        .unwrap_or_default();
    if let Some(kept) = kept {
        if let Some(own) = &kept.folder {
            folder = own.clone();
        }
        if let Some(own) = kept
            .name
            .as_deref()
            .map(str::trim)
            .filter(|own| !own.is_empty())
        {
            name = own.to_owned();
        }
    }
    let codec = kept
        .and_then(|kept| kept.codec.as_deref())
        .map(str::trim)
        .filter(|codec| !codec.is_empty() && *codec != MASTER_CODEC.as_str())
        .map(RenderCodec::from_name);
    Command::Export(ExportRequest {
        quality: RenderQuality::Master,
        codec,
        sequence: None,
        output: (!folder.is_empty()).then_some(folder),
        name: (!name.is_empty()).then_some(name),
    })
}

/// What an `export` command asks for; None for any other command.
pub fn export_of(command: &Command) -> Option<&ExportRequest> {
    match command {
        Command::Export(request) => Some(request),
        _ => None,
    }
}

/// The `export` request the toolbar's Export, ⌘E (`export.master`) and
/// Finish send for `quality` (`master`, `review`, `full`, `preview`) in
/// `project`: a master as [`master_request`] makes it, with what `prefs`
/// keep for the project; anything else as
/// [`export_request`] makes it. `editor_finish::export::render_master` sends
/// it, and `Session::export` is to.
pub fn request(project: Option<&Path>, prefs: &Prefs, quality: &str) -> Command {
    if quality == Quality::Master.wire() {
        let kept = project.and_then(|project| prefs.master_output(project));
        master_request(project, kept)
    } else {
        export_request(project, quality)
    }
}

/// A master's own codec, which goes without saying.
const MASTER_CODEC: RenderCodec = RenderCodec::ProresHq;

/// What a master in `codec` (as kerfd names it) is: short, as Export's
/// menu says it; the extension kerfd gives its file; and in full, as
/// Finish's Deliver row says it. None for a codec kerfd renders no master
/// in.
fn master_codec(codec: &RenderCodec) -> Option<(&'static str, &'static str, &'static str)> {
    match codec {
        RenderCodec::ProresHq => Some((
            "ProRes 422 HQ",
            "mov",
            "ProRes 422 HQ in QuickTime, with 24-bit sound",
        )),
        RenderCodec::H264 => Some(("H.264", "mp4", "H.264 in MP4, with its sound")),
        RenderCodec::Hevc => Some(("HEVC", "mp4", "HEVC in MP4, with its sound")),
        RenderCodec::HevcHdr => Some((
            "HEVC 10-bit HDR",
            "mp4",
            "HEVC 10-bit HDR in MP4, with its sound",
        )),
        RenderCodec::Other(_) => None,
    }
}

/// What the project's master is made as, short, as Export's menu says it
/// beside Master and Render master: "ProRes 422 HQ" unless `kept` names
/// another codec ("HEVC 10-bit HDR"). One kerfd does not render a master
/// in is named as it was kept.
pub fn master_makes(kept: Option<&MasterOutput>) -> String {
    let command = master_request(None, kept);
    let codec = export_of(&command)
        .and_then(|request| request.codec.clone())
        .unwrap_or(MASTER_CODEC);
    master_codec(&codec).map_or_else(|| codec.to_string(), |(short, _, _)| short.to_owned())
}

/// Where a master `master_request` sends goes, as Finish's Deliver row says
/// it ("deliveries/atlas_master.mov"), and what it makes. Without a folder
/// it goes where kerfd puts renders.
pub fn master_goes(project: Option<&Path>, kept: Option<&MasterOutput>) -> (String, String) {
    let command = master_request(project, kept);
    let request = export_of(&command);
    let codec = request
        .and_then(|request| request.codec.clone())
        .unwrap_or(MASTER_CODEC);
    let name = request
        .and_then(|request| request.name.as_deref())
        .unwrap_or("master");
    let (file, makes) = match master_codec(&codec) {
        Some((_, extension, makes)) => (format!("{name}.{extension}"), makes.to_owned()),
        // kerfd turns it down, so there is no file to name.
        None => (
            name.to_owned(),
            format!("{codec}, which Kerf renders no master in"),
        ),
    };
    let goes = match request.and_then(|request| request.output.as_deref()) {
        Some(folder) => format!("{}{file}", shown_folder(folder)),
        None => file,
    };
    (goes, makes)
}

/// The folder a render goes into as people read it: ending in a slash
/// ("deliveries/", "/Volumes/out/").
pub fn shown_folder(folder: &str) -> String {
    if folder.ends_with('/') || folder.ends_with('\\') {
        folder.to_owned()
    } else {
        format!("{folder}/")
    }
}
