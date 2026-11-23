//! kerf-playback's integration tests, one test binary, each former test file a module.

mod ffmpeg_playback;
mod release_gate;

use kerf_playback::{Drawers, PlaybackConfig, PlaybackError, PlaybackSession};
use std::ops::Deref;
use std::time::Duration;

/// How long a test waits for its drawing to settle: far longer than bringing a GPU up takes
/// on any machine, so that a wait that runs out is a fault to report, not a slow machine.
const SETTLES_WITHIN: Duration = Duration::from_secs(120);

/// A session the compositor draws, with the drawers the test holds for it. Let go, it closes
/// them and waits for the frames they were on and for the GPU a frame had begun to bring up.
///
/// A test that opens a session `with_composited` opens it here. Playing one asks the
/// compositor whether the GPU could draw each frame, which has it bring the GPU up on a
/// thread of its own, whatever the frame's size; a frame of 640x360 or more does the same
/// when it is drawn. A test binary that exits under that thread, at work in the GPU's
/// libraries, now and then dies as it ends, after every test has passed.
struct Composited {
    session: PlaybackSession,
    drawers: Drawers,
}

impl Composited {
    fn open(project: kerf_model::Project, config: PlaybackConfig) -> Result<Self, PlaybackError> {
        let drawers = Drawers::default();
        let session = PlaybackSession::open(project, config.with_drawers(drawers.clone()))?;
        Ok(Self { session, drawers })
    }
}

impl Deref for Composited {
    type Target = PlaybackSession;

    fn deref(&self) -> &PlaybackSession {
        &self.session
    }
}

impl Drop for Composited {
    fn drop(&mut self) {
        // Closed first: the session is still open, and would go on asking.
        self.drawers.close();
        let settled = kerf_playback::drawing_settled(SETTLES_WITHIN);
        // A test that has failed says why itself; a second panic would end the process.
        if !std::thread::panicking() {
            assert!(
                settled,
                "a composited session's drawing had not settled {SETTLES_WITHIN:?} after its \
                 drawers were closed: a frame is still being drawn, or the GPU is still being \
                 brought up"
            );
        }
    }
}
