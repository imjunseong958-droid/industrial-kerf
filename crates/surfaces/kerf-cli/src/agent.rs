//! `kerf --terminal` and `kerf --prompt`: the terminal agent's turns, and what they print.

use crate::{open_router, print_snapshot};
use anyhow::Result;
use kerf_agent::{AgentEvent, AgentSession, AgentTargetSummary};
use kerf_mcp::KerfMcpRouter;
use kerf_skills::Skill;
use std::io::{self, Write};
use std::path::Path;

/// Wait for the renders the agent started: they run on this process's
/// threads and die with it. Ctrl-C stops them with the process.
pub(crate) fn finish_renders(router: &KerfMcpRouter) {
    let running = router.running_renders();
    if running > 0 {
        eprintln!("waiting for {running} render job(s) to finish; Ctrl-C to stop");
    }
    router.finish_renders(false);
    // A making the agent started goes on until it is made; Ctrl-C stops it, and its model's
    // program takes back what it asked a server for.
    let making = router.running_makings();
    if making > 0 {
        eprintln!("waiting for {making} making(s) to finish; Ctrl-C to stop");
        let stop = kerf_engine::CancellationToken::new();
        let _held = kerf_engine::process::sidecar::stop_on_termination(&stop);
        while router.running_makings() > 0 && !stop.is_cancelled() {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        router.finish_makings(stop.is_cancelled());
    }
}

pub(crate) fn print_skills(skills: &[Skill]) {
    if skills.is_empty() {
        println!("no skills loaded");
        return;
    }
    for skill in skills {
        println!("{} · {}", skill.name, skill.description);
    }
}

pub(crate) fn run_prompt(session: &mut AgentSession, prompt: &str) -> Result<String> {
    let mut text = String::new();
    let result = session.run_with_events(prompt, |event| match event {
        AgentEvent::TextDelta(delta) => {
            print!("{delta}");
            // Left in the buffer, it shows with the next line.
            let _ = io::stdout().flush();
            text.push_str(&delta);
        }
        AgentEvent::ToolCallStarted { name, targets, .. } => {
            eprintln!("\n▸ tool {name} {}", target_summary(&targets));
        }
        AgentEvent::ToolCallFinished {
            name, ok, summary, ..
        } => {
            if !ok {
                eprintln!(
                    "✗ tool {name} failed: {}",
                    summary.error.as_deref().unwrap_or("unknown tool error")
                );
            }
        }
        AgentEvent::Aborted => eprintln!("\nagent aborted"),
        AgentEvent::TurnStarted { .. } | AgentEvent::Done { .. } | AgentEvent::Context { .. } => {}
    })?;
    if text.is_empty() {
        text = result;
    }
    Ok(text)
}

pub(crate) fn run_repl(session: &mut AgentSession, directory: &Path) -> Result<()> {
    println!("kerf, session dir: {}", directory.display());
    println!("Type /quit to exit, /timeline to inspect the current revision.");
    let stdin = io::stdin();
    loop {
        print!("❯ ");
        io::stdout().flush()?;
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim();
        match line {
            "" => continue,
            "/quit" | "/q" | "exit" => break,
            "/timeline" => {
                let router = open_router(directory, None)?;
                print_snapshot(router.runtime(), "timeline");
            }
            "/save" => {
                session.save(&directory.join("conversation.json"))?;
                println!("saved conversation.json");
            }
            prompt => {
                // Its answer was printed as it streamed in.
                run_prompt(session, prompt)?;
                println!();
            }
        }
    }
    println!("bye");
    Ok(())
}

fn target_summary(targets: &AgentTargetSummary) -> String {
    let mut parts = Vec::new();
    add_target_summary(&mut parts, "source", &targets.sources);
    add_target_summary(&mut parts, "clip", &targets.clips);
    add_target_summary(&mut parts, "track", &targets.tracks);
    add_target_summary(&mut parts, "job", &targets.jobs);
    if parts.is_empty() {
        "(no targets)".into()
    } else {
        parts.join("; ")
    }
}

fn add_target_summary(parts: &mut Vec<String>, label: &str, values: &[String]) {
    if !values.is_empty() {
        parts.push(format!("{label}: {}", values.join(", ")));
    }
}
