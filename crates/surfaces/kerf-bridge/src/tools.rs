//! Kerf's own tools, as the bridge runs them in this process: its agent's (`look`, `edit`,
//! `render` and `make`, on the agent's router) and the two it calls itself for presets.
//!
//! A tool draws here, and the render or making one starts goes on here after it has
//! answered, so each begins on the bridge's drawers (`Shared::drawers`), as the bridge's own
//! drawing does: once the bridge is let go a call is refused and nothing more is started,
//! and what is about to exit waits for what was under way. A making is stopped with the
//! makings the bridge shows (`media::cancel_makings`); a render is stopped here. A call
//! under way has nothing to stop it: it is waited for to its end, for as long as whoever
//! waits will.

use crate::Shared;
use kerf_mcp::{Begins, Held, KerfMcpRouter};
use kerf_playback::OnFrame;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The renders the tools started and are still on, by job: what the bridge stops as it
/// closes.
#[derive(Default)]
pub(crate) struct Renders(Mutex<BTreeSet<String>>);

impl Renders {
    fn jobs(&self) -> MutexGuard<'_, BTreeSet<String>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What a tool call, or what one started, holds of the bridge until it is over.
struct Begun {
    /// The render it is, to be forgotten when it ends.
    render: Option<(Arc<Shared>, String)>,
    _drawing: OnFrame,
}

impl Drop for Begun {
    fn drop(&mut self) {
        if let Some((shared, job)) = &self.render {
            shared.tool_renders.jobs().remove(job);
            // A bridge closing just as the render ended wrote its marker after the render
            // had looked for it: nothing else would take it away.
            let _ = std::fs::remove_file(kerf_runtime::render_cancel_path(&shared.directory, job));
        }
    }
}

/// Why a tool is refused once the bridge is let go.
const CLOSING: &str = "the project is being closed";

/// What lets the tools of `shared`'s bridge begin: its drawers.
fn gate(
    shared: Arc<Shared>,
) -> impl Fn(Begins<'_>) -> Result<Held, String> + Send + Sync + 'static {
    move |begins| {
        // A render is known before it may begin, so that a bridge let go in between finds
        // it to stop.
        let render = match begins {
            Begins::Render(job) => {
                shared.tool_renders.jobs().insert(job.to_owned());
                Some((shared.clone(), job.to_owned()))
            }
            Begins::Call | Begins::Making(_) => None,
        };
        let Some(drawing) = shared.drawers.begin() else {
            if let Some((shared, job)) = &render {
                shared.tool_renders.jobs().remove(job);
            }
            return Err(CLOSING.to_owned());
        };
        Ok(Box::new(Begun {
            render,
            _drawing: drawing,
        }))
    }
}

/// Have what `router` and its copies run begin on the drawers of `shared`'s bridge.
pub(crate) fn begin_on_drawers(router: &KerfMcpRouter, shared: &Arc<Shared>) {
    router.draw_through(gate(shared.clone()));
}

/// The bridge closed: the renders its tools started stop, as the render queue's does.
pub(crate) fn stop_renders(shared: &Shared) {
    for job in shared.tool_renders.jobs().iter() {
        // The render watches for this marker; nobody is left to see it.
        let marker = kerf_runtime::render_cancel_path(&shared.directory, job);
        if let Err(error) = std::fs::write(&marker, b"") {
            log::warn!(
                "render {job} goes on after the bridge closed: {}: {error}",
                marker.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_closed_bridge_lets_no_tool_begin_and_stops_the_renders_tools_began() {
        let folder = tempfile::tempdir().expect("a folder");
        let shared = Arc::new(crate::tests_support::shared(folder.path()));
        let begin = gate(shared.clone());
        let marker = |job: &str| kerf_runtime::render_cancel_path(folder.path(), job);
        let refused = |begins: Begins<'_>| begin(begins).err();

        // Open, a call begins and is over, and nothing of it is left.
        drop(begin(Begins::Call).expect("a call begins"));
        // A render and a making go on after their calls.
        let render = begin(Begins::Render("render-1")).expect("a render begins");
        let making = begin(Begins::Making("m1")).expect("a making begins");
        let ended = begin(Begins::Render("render-2")).expect("another render begins");
        drop(ended);

        // The bridge is let go, as `Drop for Bridge` does it: no tool begins any more, and
        // each is told why.
        shared.drawers.close();
        for begins in [
            Begins::Call,
            Begins::Render("render-3"),
            Begins::Making("m2"),
        ] {
            assert_eq!(refused(begins).as_deref(), Some(CLOSING), "{begins:?}");
        }
        // The render still under way is told to stop; the one that had ended, and the one
        // that never began, are not.
        stop_renders(&shared);
        assert!(marker("render-1").is_file());
        assert!(!marker("render-2").exists());
        assert!(!marker("render-3").exists());

        // What is about to exit waits for the two that go on, and no longer than them.
        assert!(!kerf_playback::drawing_settled(Duration::from_millis(50)));
        drop(render);
        // A marker its render never took away goes with it.
        assert!(!marker("render-1").exists());
        assert!(!kerf_playback::drawing_settled(Duration::from_millis(50)));
        drop(making);
        assert!(kerf_playback::drawing_settled(Duration::from_secs(30)));
    }
}
