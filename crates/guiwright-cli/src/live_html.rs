//! `guiwright live-html` — the §12 live-mode emitter over the CLI doors
//! (m87): attach the M18 live feed to a **running** session over any
//! door (UDS, TCP, WebSocket), accumulate the scrubber model while the
//! session runs, and emit the self-contained scrubber HTML mid-session —
//! M23's `render_live_html`, stamped `gwtrace/v1 (live)`, no protocol
//! change (the capture rides the subscribe vocabulary the session
//! already serves).
//!
//! The capture window is bounded: the pump ends early after a quiet
//! stretch (the session stopped producing events), so an idle session
//! does not pin the command for the full window. The written file
//! replays everything the feed saw up to the moment of emission — the
//! offline checkpoint the §12 archive row gives finished runs, now
//! available while the session is still live.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use guiwright_viewer::LiveFeed;

use crate::session::{ConnectTarget, Connection};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LiveHtmlError {
    #[error("wire: {0}")]
    Wire(#[from] guiwright_proto::GwpError),
    #[error("viewer: {0}")]
    Viewer(#[from] guiwright_viewer::ViewerError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// The default capture window (`--duration-ms` unset): long enough for
/// an active session to show its shape, short enough for a checkpoint.
pub const DEFAULT_CAPTURE_MS: u64 = 1_000;

/// The quiet stretch that ends the capture early (the M18 pacing rule:
/// a quiet window is a `None`, never an error — four consecutive quiet
/// windows mean the session is idle and the capture is complete).
const QUIET_WINDOWS: u32 = 4;

/// Attach to `target`, accumulate the live scrubber for up to
/// `duration_ms`, and write the scrubber HTML to `out` (default
/// `guiwright-live.html` in the current directory). The emission happens
/// **mid-session**: the feed stays attached for the whole window, and
/// the HTML replays the tree dumps, frame axis, events and logs the feed
/// saw up to that moment.
pub async fn live_html(
    target: &ConnectTarget,
    out: Option<PathBuf>,
    duration_ms: u64,
    test: Option<String>,
) -> Result<PathBuf, LiveHtmlError> {
    let connection = Connection::connect(target).await?;
    let mut feed = LiveFeed::new(connection);
    feed.attach(crate::AGENT).await?;
    if let Some(test) = test {
        feed.set_test(test);
    }

    let deadline = Instant::now() + Duration::from_millis(duration_ms.max(1));
    let mut quiet = 0_u32;
    while Instant::now() < deadline {
        // One event per pump (§12: "one applied event per call"); the
        // frame axis rides `FramePresented`, tree moves auto-sync a
        // fresh dump — the same accumulation the M18 scrubber model
        // rides. A subscribed session pumps at its idle tick, so frames
        // keep the window busy; the capture ends early when the door
        // goes quiet or closes — the checkpoint still emits what the
        // feed saw (the session's end never eats the capture).
        match feed.pump(Duration::from_millis(50)).await {
            Ok(Some(_change)) => quiet = 0,
            Ok(None) => {
                quiet += 1;
                if quiet >= QUIET_WINDOWS {
                    break;
                }
            }
            Err(_closed) => break,
        }
    }

    let html = feed.render_html();
    let _ = feed.bye().await;
    let out = out.unwrap_or_else(default_out);
    std::fs::write(&out, html)?;
    Ok(out)
}

/// The default output path: the session-checkpoint file in the caller's
/// working directory.
fn default_out() -> PathBuf {
    PathBuf::from("guiwright-live.html")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_out_is_the_cwd_checkpoint() {
        assert_eq!(default_out(), PathBuf::from("guiwright-live.html"));
        assert_eq!(DEFAULT_CAPTURE_MS, 1_000);
    }
}
