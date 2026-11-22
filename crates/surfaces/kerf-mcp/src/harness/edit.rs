//! `edit`: a script, a preset, or the steps and the cast an edit writes, staged as a proposal
//! for the user with its diff, its check findings, and its preview, and accepted where the
//! person lets an agent's edits be; and a stale proposal brought onto the project as it is.

use super::preview::changed_range;
use super::shots::{ShotChanges, WorkflowKept};
use super::tools::{input, EditInput};
use super::{catalog, diagnostics, not_pending, operation_key, parse};
use crate::{engine_error, KerfMcpRouter, McpError};
use kerf_lang::{apply_script, apply_script_with, print_document, PrintOptions};
use kerf_model::Project;
use kerf_runtime::{Author, CachedFacts, ProposalEntry, StateSnapshot};
use serde_json::{json, Value};

impl KerfMcpRouter {
    pub(crate) fn edit(&self, arguments: Value) -> Result<Value, McpError> {
        // `revise` starts again from the project, so what its proposal added is not there either.
        let building_on = arguments.get("extend").is_some();
        self.run_edit(arguments).map_err(|mut error| {
            // What is not in the project may be in a proposal not built on.
            if !building_on && error.code == "SCRIPT_REFUSED" {
                let hint = ["no clip `", "unknown media `", "no clip or marker `"]
                    .iter()
                    .find_map(|before| {
                        let (_, rest) = error.message.split_once(before)?;
                        rest.split_once('`').map(|(name, _)| name.to_owned())
                    })
                    .map(|name| self.in_pending(&name, "extend"));
                if let Some(hint) = hint {
                    error.message.push_str(&hint);
                }
            }
            error
        })
    }

    fn run_edit(&self, arguments: Value) -> Result<Value, McpError> {
        let alone = arguments.as_object().is_some_and(|given| given.len() == 1);
        let input: EditInput = input(arguments)?;
        if let Some(id) = &input.forward {
            if !alone {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "`forward` is passed alone: it runs the proposal's own operations again, with no script of this edit's",
                ));
            }
            return self.forward(id);
        }
        one_kind_asked(&input)?;
        if let Some(file) = &input.save_preset {
            return self.preset_saved(&input, file);
        }
        script_asked(&input)?;
        let replaced = input.extend.as_deref().or(input.revise.as_deref());
        let prior = self.replaced_proposal(&input, replaced)?;
        // `extend` runs the script on what the proposal makes; `revise` on
        // the project.
        let (revision, before) = self.project_at(input.extend.as_deref())?;
        // A preset's clips, markers and transitions are given ids of the
        // project's own: the answer says which, by the ids its pattern has.
        let mut preset_ids = None;
        let source_script = match &input.preset {
            Some(file) => {
                let (script, ids) = self.preset_script(
                    &before,
                    file,
                    input.sequence.as_deref(),
                    &input.bindings,
                    &input.texts,
                )?;
                preset_ids = Some(ids);
                script
            }
            None => input.script.clone().unwrap_or_default(),
        };
        // A recipe carried into the document: the lines that write its steps under its
        // media come first, so that the script's own lines can change them.
        let carried = self.carried(&before, &input)?;
        let source_script = with_carried(&carried, source_script);
        let facts = CachedFacts::new(&self.dir);
        let mut applied =
            apply_script_with(&before, catalog(), &facts, &source_script).map_err(refused)?;
        // A value set in a model's workflow, and a take gone back to, change steps: by lines
        // of the script itself, after its own. No file says how a media is made, and undo
        // takes them back with the rest of the edit. They are read from the steps as the
        // script left them, so that a word it set on the same step stays.
        let ShotChanges { workflows, steps } = self.shot_changes(&applied.project, &input)?;
        let mut source_script = source_script;
        if !workflows.is_empty() || !steps.is_empty() {
            source_script = with_shot_lines(&workflows, &steps, &source_script);
            applied =
                apply_script_with(&before, catalog(), &facts, &source_script).map_err(refused)?;
        }
        // Who and what the edit's steps show that the project has not said yet is added to its
        // cast file, which asks for no shot again; one it has is the person's to change.
        let cast_added = if input.cast.is_empty() {
            Vec::new()
        } else {
            kerf_engine::recipe::cast::Cast::add(&self.dir, &input.cast)
                .map_err(|error| McpError::new("INVALID_CAST", error.to_string()))?
        };
        if print_document(&applied.project, catalog(), PrintOptions::default())
            == print_document(&before, catalog(), PrintOptions::default())
        {
            if !cast_added.is_empty() {
                return Ok(json!({
                    "proposal": null,
                    "status": "written",
                    "cast": cast_added,
                }));
            }
            return Err(McpError::new("NO_CHANGE", "the script changes nothing"));
        }
        // An extension runs the operations of the proposal it extends, which
        // read nothing that could have changed since, and then this script;
        // it keeps both scripts as they were written.
        let (script, written) = match prior.filter(|_| input.extend.is_some()) {
            Some((operations, source, _)) => self.extended(
                (operations.as_str(), source.as_str()),
                &facts,
                &applied.project,
                &source_script,
            )?,
            None => (source_script, None),
        };
        // A revised or extended proposal becomes its next version, which
        // keeps the versions before it for the user to see.
        let author = self.author();
        let snapshot = self.stage_edit(revision, &script, written.as_deref(), &author, replaced)?;
        // The runtime lists the proposal it staged first.
        let staged = first_pending(&snapshot)?;
        let proposal_id = staged.proposal_id.clone();
        let carried_said = self.takes_carried(&before, &applied.project, &carried)?;
        // And each workflow a recipe of it names: a new file, beside the ones before it.
        self.workflows_written(&workflows, &proposal_id)?;
        let mut result = self.staged(staged, revision, &before, &applied.project)?;
        // Where `snap=` put each edge it moved, which the diff shows only as where it is.
        if !applied.snapped.is_empty() {
            result["snapped"] = json!(applied.snapped);
        }
        if !workflows.is_empty() {
            result["workflows"] =
                json!(workflows.iter().map(WorkflowKept::json).collect::<Vec<_>>());
        }
        if !carried_said.is_empty() {
            result["carried"] = json!(carried_said);
        }
        if !cast_added.is_empty() {
            result["cast"] = json!(cast_added);
        }
        // What scopes read of it, each beside what they read before: a grade and its numbers
        // in one call, where an edit and then a look took two.
        if !input.scopes.is_empty() {
            result["scopes"] =
                self.scopes_after(&before, &applied.project, revision, &input.scopes);
        }
        if let Some(ids) = preset_ids {
            result["ids"] = json!(ids);
        }
        // What the person lets an agent's edit be without their review.
        self.accept_if_permitted(&proposal_id, &author, revision, &mut result);
        // What a person would be asked to make, with what would refuse it: a recipe the edit
        // names is mended here, not after someone has tried to run it. A making is asked of
        // the project, so while this waits for review it says that the proposal comes first.
        let waiting = result["status"] == "awaiting_review";
        let to_make = self.to_make(&applied.project, waiting);
        if !to_make.is_empty() {
            result["toMake"] = json!(to_make);
        }
        Ok(result)
    }

    /// `edit` with `savePreset`: the accepted sequence saved to a new preset file, which
    /// takes only `sequence` and `name`.
    fn preset_saved(&self, input: &EditInput, file: &str) -> Result<Value, McpError> {
        if input.extend.is_some()
            || input.revise.is_some()
            || !input.bindings.is_empty()
            || !input.texts.is_empty()
        {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "savePreset takes only sequence and name",
            ));
        }
        let (_, project) = self.project_at(None)?;
        self.save_preset(
            &project,
            file,
            input.sequence.as_deref(),
            input.name.as_deref(),
        )
    }

    /// The pending proposal `replaced` names, which `extend` builds on or `revise` replaces:
    /// its operations, its script as it was written, and its base revision.
    fn replaced_proposal(
        &self,
        input: &EditInput,
        replaced: Option<&str>,
    ) -> Result<Option<(String, String, u64)>, McpError> {
        let prior = match replaced {
            Some(id) => Some(self.pending(id)?),
            None => None,
        };
        // An extension runs on what the version named makes. Named by the id
        // of a version since replaced, it would leave out what the versions
        // after it added, and become the newest all the same.
        if let Some(id) = input.extend.as_deref() {
            if let Some((version, newest, newest_id)) = self.newer_version(id)? {
                return Err(McpError::new(
                    "PROPOSAL_SUPERSEDED",
                    format!(
                        "proposal {id} is version {version} of a proposal now at version \
                         {newest}: built on it, this script would leave out what the versions \
                         since added. To build on all of it, pass the newest, {newest_id}, as \
                         `extend`; to start again from the project, pass `revise`"
                    ),
                ));
            }
        }
        Ok(prior)
    }

    /// What carrying each media of `carry` into the document writes, by its media.
    fn carried(
        &self,
        before: &Project,
        input: &EditInput,
    ) -> Result<Vec<(String, kerf_engine::recipe::Carry)>, McpError> {
        let mut carried = Vec::new();
        for id in &input.carry {
            let carry = kerf_engine::recipe::carry(before, &self.dir, id).map_err(|error| {
                McpError::new(
                    "INVALID_RECIPE",
                    format!("{error}{}", self.in_pending(id, "extend")),
                )
            })?;
            carried.push((id.clone(), carry));
        }
        Ok(carried)
    }

    /// What an extension stages: the operations of the proposal it extends, then
    /// `source_script`, refused where after them it makes other than `applied`, what it made
    /// on its own; and the two scripts as they were written, `source` the extended one's.
    fn extended(
        &self,
        (operations, source): (&str, &str),
        facts: &CachedFacts,
        applied: &Project,
        source_script: &str,
    ) -> Result<(String, Option<String>), McpError> {
        let operations = operations.trim_end();
        let script = format!("{operations}\n{source_script}");
        let (_, base) = self.project_at(None)?;
        let staged = apply_script_with(&base, catalog(), facts, &script)
            .map_err(|errors| refused(after(errors, operations.lines().count())))?;
        // Such as when the operations end on another sequence than
        // this script started on.
        if print_document(&staged.project, catalog(), PrintOptions::default())
            != print_document(applied, catalog(), PrintOptions::default())
        {
            return Err(McpError::new(
                "SCRIPT_REFUSED",
                "after the operations of the proposal it extends, this script does \
                 something else than it did on its own; start it with `use <sequence>`",
            ));
        }
        Ok((
            script,
            Some(format!("{}\n{source_script}", source.trim_end())),
        ))
    }

    /// Stage `script`, `written` as the agent wrote it, as a proposal by `author` on the
    /// project at `revision`: with `replaced`, as that proposal's next version.
    fn stage_edit(
        &self,
        revision: u64,
        script: &str,
        written: Option<&str>,
        author: &Author,
        replaced: Option<&str>,
    ) -> Result<StateSnapshot, McpError> {
        self.runtime
            .stage_script_proposal_by(
                &operation_key("agent-edit"),
                revision,
                script,
                written,
                author,
                replaced,
            )
            .map_err(|error| match error.code.as_str() {
                "PROPOSAL_PENDING" => McpError::new(
                    "PROPOSAL_PENDING",
                    format!(
                        "{}; pass one's id as `revise` to replace it, or `extend` to build on it",
                        error.message
                    ),
                ),
                "PROPOSAL_NOT_PENDING" | "PROPOSAL_NOT_FOUND" => {
                    not_pending(replaced.unwrap_or_default())
                }
                _ => McpError::from(error),
            })
    }

    /// What `edit` says in `carried` of each recipe it carried: its takes held to the steps it
    /// became, which `after` has.
    fn takes_carried(
        &self,
        before: &Project,
        after: &Project,
        carried: &[(String, kerf_engine::recipe::Carry)],
    ) -> Result<Vec<Value>, McpError> {
        // The takes a carried recipe made are held to the steps it became, so that none is
        // asked for again for being carried. A proposal that is rejected leaves them reading
        // as they did: a media that names its recipe goes by what the recipe's take recorded.
        let mut carried_said = Vec::new();
        for (id, carry) in carried {
            let held = kerf_engine::recipe::takes::mark_carried(before, after, &self.dir, id)
                .map_err(engine_error)?;
            let mut said = json!({"media": id, "recipe": carry.recipe, "takesHeld": held});
            if !carry.notes.is_empty() {
                said["notes"] = json!(carry.notes);
            }
            carried_said.push(said);
        }
        Ok(carried_said)
    }

    /// Write the workflow copies an edit keeps, each a new file beside the ones before it;
    /// where one cannot be written, the proposal `proposal_id` it was staged as is rejected.
    fn workflows_written(
        &self,
        workflows: &[WorkflowKept],
        proposal_id: &str,
    ) -> Result<(), McpError> {
        for workflow in workflows {
            if let Err(why) = kerf_shots::workflow::write(&workflow.copy.file, &workflow.copy.text)
            {
                if let Err(error) = self.runtime.reject_proposal(proposal_id) {
                    log::warn!(
                        "proposal {proposal_id} is still pending though its workflow copy was \
                         not written: {}",
                        error.message
                    );
                }
                return Err(McpError::new("INVALID_RECIPE", why));
            }
        }
        Ok(())
    }

    /// What scopes read at `positions` of what an edit makes, `after`, each beside what they
    /// read there `before` it; or why they could not be read.
    fn scopes_after(
        &self,
        before: &Project,
        after: &Project,
        revision: u64,
        positions: &[String],
    ) -> Value {
        let changed = changed_range(before, after).map(|(sequence, ..)| sequence.id.to_string());
        let read =
            self.program_scopes(after, Some(before), changed.as_deref(), revision, positions);
        match read {
            Ok(mut read) => read["readings"].take(),
            Err(error) => json!({"unavailable": error.message}),
        }
    }

    /// Accept the proposal `proposal_id` where the person's own permissions accept an agent's
    /// edits, and say in `result` what they are and what came of it.
    fn accept_if_permitted(
        &self,
        proposal_id: &str,
        author: &Author,
        revision: u64,
        result: &mut Value,
    ) {
        match self.permissions() {
            Ok(permissions) if permissions.accepts_edits() => {
                let accepted = self.runtime.accept_proposal(
                    proposal_id,
                    if author.kind == "mcp" { "mcp" } else { "agent" },
                    &operation_key("agent-accept"),
                    revision,
                );
                result["permissions"] = json!(permissions.mode.name());
                match accepted {
                    Ok(state) => {
                        result["status"] = json!("accepted");
                        result["revision"] = json!(state.revision);
                    }
                    Err(error) => {
                        result["notAccepted"] = json!(format!(
                            "{}; it waits for the person's review",
                            error.message
                        ));
                    }
                }
            }
            Ok(_) => {}
            Err(error) => {
                result["permissionsError"] =
                    json!(format!("{error}: so it waits for the person's review"));
            }
        }
    }

    /// Bring the stale pending proposal `id` onto the project as it is, as
    /// its next version. It waits for review whatever the person lets an
    /// agent's own edits be: it may be another's, and its frames are not the
    /// ones that were looked at.
    fn forward(&self, id: &str) -> Result<Value, McpError> {
        let (revision, before) = self.project_at(None)?;
        let snapshot = self
            .runtime
            .forward_proposal(&operation_key("agent-forward"), revision, id)
            .map_err(|error| match error.code.as_str() {
                "PROPOSAL_NOT_PENDING" | "PROPOSAL_NOT_FOUND" => not_pending(id),
                // The line and the clip that stopped it, as the runtime
                // said them, and what is left to do.
                "PROPOSAL_NOT_FORWARDED" => McpError::new(
                    "PROPOSAL_NOT_FORWARDED",
                    format!(
                        "{}. Nothing was staged and proposal {id} waits as it was; pass it as `revise` with a script written on the project as it is",
                        error.message
                    ),
                ),
                "PROPOSAL_SUPERSEDED" | "PROPOSAL_NOT_STALE" | "PROPOSAL_BASE_GONE" => {
                    McpError::new(&error.code, error.message)
                }
                _ => McpError::from(error),
            })?;
        let staged = first_pending(&snapshot)?;
        let after = apply_script(&before, catalog(), &staged.script)
            .map_err(refused)?
            .project;
        let mut result = self.staged(staged, revision, &before, &after)?;
        result["forwarded"] = json!(id);
        Ok(result)
    }

    /// What `edit` says of the proposal it staged on the project at
    /// `revision`: its id, version and author, its script and diff, what the
    /// checks find in what it makes, and the frames `before` and `after`.
    fn staged(
        &self,
        staged: &ProposalEntry,
        revision: u64,
        before: &Project,
        after: &Project,
    ) -> Result<Value, McpError> {
        let (_, after_document) = self.runtime.proposal_documents(&staged.proposal_id)?;
        let after_program = parse(&after_document)?;
        let mut before_program = before.clone();
        kerf_runtime::resolve_media_paths(&mut before_program, &self.dir);
        let check = crate::qc::check(&after_program, &self.dir).to_json();
        let preview = match changed_range(before, after) {
            None => json!({"unavailable": "the edit changes nothing on the sequence"}),
            Some((sequence, from, to)) => self
                .preview(&sequence, from, to, &before_program, &after_program)
                .unwrap_or_else(|error| json!({"unavailable": error.message})),
        };
        Ok(json!({
            "proposal": staged.proposal_id,
            "version": staged.version,
            "supersedes": staged.supersedes,
            "author": staged.author.name,
            "baseRevision": revision,
            "status": "awaiting_review",
            "script": staged.script,
            "diff": staged.document_diff,
            "check": check,
            "preview": preview,
        }))
    }
}

/// Refuses an edit that asks for other than one kind of thing: a script, with or without the
/// steps, recipes and cast it writes, or those alone; a preset; or a preset to save.
fn one_kind_asked(input: &EditInput) -> Result<(), McpError> {
    if [
        input.script.is_some()
            || !input.workflow_set.is_empty()
            || input.restore_take.is_some()
            || !input.carry.is_empty()
            || !input.cast.is_empty(),
        input.preset.is_some(),
        input.save_preset.is_some(),
    ]
    .into_iter()
    .filter(|v| *v)
    .count()
        != 1
    {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "pass exactly one of script (with or without workflowSet, carry and cast), any of those alone, preset, or savePreset",
        ));
    }
    // A recipe carried is read as it stands: what else this edit would write of one,
    // it writes after, as lines of a script that set the steps it became.
    if !input.carry.is_empty() && (!input.workflow_set.is_empty() || input.restore_take.is_some()) {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "`carry` writes a recipe's steps under its media as they are: change them in the script of the same edit (`step <media> set <n> …`), or in an edit after it",
        ));
    }
    Ok(())
}

/// Refuses a script given with what is for presets, and `extend` given with `revise`.
fn script_asked(input: &EditInput) -> Result<(), McpError> {
    if input.name.is_some()
        || (input.script.is_some()
            && (input.sequence.is_some() || !input.bindings.is_empty() || !input.texts.is_empty()))
    {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "sequence, bindings and texts are for presets; name is for savePreset",
        ));
    }
    if input.extend.is_some() && input.revise.is_some() {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "pass `extend` or `revise`, not both",
        ));
    }
    Ok(())
}

/// The proposal the runtime staged, which it lists first.
fn first_pending(snapshot: &StateSnapshot) -> Result<&ProposalEntry, McpError> {
    let Some(staged) = snapshot
        .proposals
        .first()
        .filter(|entry| entry.status == "pending")
    else {
        return Err(McpError::new(
            "RUNTIME_ERROR",
            "the runtime did not keep the proposal",
        ));
    };
    Ok(staged)
}

/// `source_script` after the lines that write the steps of each recipe `carried` under its
/// media.
fn with_carried(carried: &[(String, kerf_engine::recipe::Carry)], source_script: String) -> String {
    if carried.is_empty() {
        source_script
    } else {
        let lines: String = carried
            .iter()
            .flat_map(|(_, carry)| &carry.script)
            .map(|line| format!("{line}\n"))
            .collect();
        format!("{lines}{source_script}")
    }
}

/// `source_script` with the lines that write what `workflowSet` and `restoreTake` set: the
/// workflow copies it keeps, as comments, before it, and the steps' lines after it.
fn with_shot_lines(workflows: &[WorkflowKept], steps: &[String], source_script: &str) -> String {
    let shown: String = workflows.iter().map(WorkflowKept::lines).collect();
    let steps: String = steps.iter().map(|line| format!("{line}\n")).collect();
    format!("{shown}{}{steps}", with_newline(source_script))
}

/// A script refusal, with each line numbered as the agent wrote it.
fn refused(errors: Vec<kerf_model::Diagnostic>) -> McpError {
    McpError::new("SCRIPT_REFUSED", diagnostics(&errors))
}

/// `errors` of a script run after `lines` lines of other script, with
/// their lines counted in that script.
fn after(mut errors: Vec<kerf_model::Diagnostic>, lines: usize) -> Vec<kerf_model::Diagnostic> {
    for error in &mut errors {
        if let Some(span) = error.span.as_mut() {
            span.line = span.line.saturating_sub(lines);
        }
    }
    errors
}

/// `text` ending with a line break, unless it is empty.
fn with_newline(text: &str) -> String {
    if text.is_empty() || text.ends_with('\n') {
        text.to_owned()
    } else {
        format!("{text}\n")
    }
}
