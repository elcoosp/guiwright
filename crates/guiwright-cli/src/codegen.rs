//! `guiwright codegen <app-cmd…>` — the §13 standalone codegen session
//! (the m4 §4 deferred row, unclosed through m7 §4 "unchanged from
//! M5/M6"): the launcher spawns the app binary with the §5.3
//! `GUIWRIGHT_DRIVER` launch-mode env, waits for the door to open,
//! attaches, and runs the overlay session the deferral named — M8's
//! `EnableOverlay` + the interaction recorder — then hands the
//! generated kit code back when the window ends.
//!
//! The window ends three ways, each honest about the server-side
//! single-slot recorder:
//!
//! * `--duration-ms N` elapses (or Ctrl-C on a unix child in its own
//!   process group) while the door still serves — `StopRecording` hands
//!   back the kit code (the m87 wire codegen shape), the app is reaped.
//! * the app closes the door first — the recorder died with the
//!   session, so the launcher answers the typed [`CodegenError::
//!   RecordingLost`] instead of pretending to capture (the counterpart
//!   of the m87 live-html early end, which still emits what it saw;
//!   a recording cannot survive its session).
//!
//! Every step rides existing wire domains — no protocol change (the
//! M2/M4 conclusion): the door wait is plain connection attempts, the
//! overlay session is the `Inspect` domain, the liveness probe is
//! `Ping`, and the capture is the recorder's `StopRecording` reply.

use std::path::PathBuf;
use std::process::Command as StdCommand;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use guiwright_proto::GwpError;
use guiwright_proto::schema::{AdapterInfoV1, InspectV1, RequestV1, ResponseV1};

use crate::session::{ConnectTarget, Connection};

/// The door-wait deadline (`--attach-ms` unset): long enough for a GUI
/// app to boot its event loop and serve, short enough that a broken
/// launch fails fast.
pub const DEFAULT_ATTACH_MS: u64 = 10_000;

/// The door-wait poll cadence (a connect attempt against a not-yet-bound
/// socket is free — the retry loop holds no protocol state).
const DOOR_POLL_MS: u64 = 50;

/// The in-window liveness cadence (`Ping`, §7): the only polling in the
/// launcher, and only while it idles outside the wire conversation.
const LIVENESS_POLL_MS: u64 = 100;

/// One launch request: the app command plus the wrapper's flags (the
/// `Command::Codegen` argv, already parsed).
#[derive(Clone, Debug)]
pub struct CodegenLaunch {
    /// The app command: `command[0]` is the program, the rest its argv.
    /// The launcher injects `GUIWRIGHT_DRIVER` (§5.3) into the child's
    /// env — the app-side serving contract every remote-mode app
    /// implements — and leaves the rest of the environment inherited.
    pub command: Vec<String>,
    /// Door override (`--uds <path>` / `--tcp <addr>`). `None` picks a
    /// fresh temp UDS path, so concurrent launches never collide.
    pub door: Option<ConnectTarget>,
    /// Door-wait deadline in ms (`--attach-ms`); default
    /// [`DEFAULT_ATTACH_MS`].
    pub attach_ms: Option<u64>,
    /// Capture window in ms (`--duration-ms`). `None` records until the
    /// door closes — the interactive finish is Ctrl-C (on unix the child
    /// rides its own process group, so only the launcher sees the
    /// signal); on Windows the console event reaches the child too, so
    /// the honest finish there is `--duration-ms`.
    pub duration_ms: Option<u64>,
    /// Also write the generated kit code to this file (`--out`); stdout
    /// carries it regardless.
    pub out: Option<PathBuf>,
}

/// What the capture produced.
#[derive(Clone, Debug, PartialEq)]
pub struct CodegenOutcome {
    /// The generated kit program (the recorder's exact lines, the m87
    /// E2E's byte-exact contract).
    pub code: String,
    /// Strict-mode warnings the recorder raised while recording.
    pub warnings: Vec<String>,
    /// The attach report (§7 `Welcome`): what the app serves.
    pub adapter: AdapterInfoV1,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CodegenError {
    /// The app command could not spawn — the wrapper never falls back
    /// to attaching to some other session.
    #[error("cannot spawn `{command}`: {source}")]
    Spawn {
        command: String,
        source: std::io::Error,
    },
    /// The app never opened its door within the attach deadline.
    #[error("the app never opened its door {door} within {attach_ms} ms")]
    DoorTimeout { door: String, attach_ms: u64 },
    /// The app exited before opening its door (a broken launch, not a
    /// slow one — the deadline is not the message).
    #[error("the app exited before opening its door ({status})")]
    ChildExited { status: String },
    /// The app closed its door before the window ended. The server-side
    /// single-slot recorder (§13) died with the session — there is
    /// nothing to hand back, and the error says exactly that instead of
    /// emitting an empty program.
    #[error(
        "the app closed its door before the window ended — the server-side \
         recording died with the session; finish with --duration-ms or \
         Ctrl-C while the app still serves"
    )]
    RecordingLost,
    /// `ws://` doors are attach targets (`guiwright repl`), not launch
    /// targets: the launcher cannot know an app serves WebSocket from
    /// the §5.3 env alone.
    #[error(
        "the codegen launcher spawns the app over a uds or tcp door — \
         ws:// doors attach (guiwright repl / live-html), they are not \
         launch targets"
    )]
    UnsupportedDoor,
    #[error("wire: {0}")]
    Wire(#[from] GwpError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Run the §13 standalone codegen session over `launch`'s app command.
pub async fn codegen_launch(launch: &CodegenLaunch) -> Result<CodegenOutcome, CodegenError> {
    let door = launch.door.clone().unwrap_or_else(default_door);
    let env_value = door_env_value(&door)?;
    let attach_ms = launch.attach_ms.unwrap_or(DEFAULT_ATTACH_MS);
    let mut child = spawn_app(launch, &env_value)?;

    // The door wait: retry plain connect attempts until the app's
    // driver feature binds (§5.3 "the app serves when the feature is
    // compiled in and the env names the door"). A child that dies while
    // we wait is a broken launch, not a slow one.
    let attach_deadline = Instant::now() + Duration::from_millis(attach_ms);
    let mut connection = loop {
        match Connection::connect(&door).await {
            Ok(connection) => break connection,
            Err(_not_open_yet) => {
                if Instant::now() >= attach_deadline {
                    reap_child(&mut child);
                    return Err(CodegenError::DoorTimeout {
                        door: door_display(&door),
                        attach_ms,
                    });
                }
                if let Ok(Some(status)) = child.try_wait() {
                    reap_child(&mut child);
                    return Err(CodegenError::ChildExited {
                        status: status.to_string(),
                    });
                }
                tokio::time::sleep(Duration::from_millis(DOOR_POLL_MS)).await;
            }
        }
    };

    let adapter = connection.handshake(crate::AGENT).await?;

    // The §13 overlay session the deferral named: hover outlines +
    // role badges compositing at present time, then the recorder.
    expect_pong(
        connection
            .request(RequestV1::Inspect(InspectV1::EnableOverlay))
            .await?,
        "EnableOverlay",
    )?;
    expect_pong(
        connection
            .request(RequestV1::Inspect(InspectV1::StartRecording))
            .await?,
        "StartRecording",
    )?;

    let end = capture_window(&mut connection, launch.duration_ms).await;
    match end {
        WindowEnd::Lost => {
            reap_child(&mut child);
            Err(CodegenError::RecordingLost)
        }
        WindowEnd::Captured => {
            let stop = connection
                .request(RequestV1::Inspect(InspectV1::StopRecording))
                .await;
            reap_child(&mut child);
            let (code, warnings) = match stop {
                Ok(ResponseV1::Codegen { code, warnings }) => (code, warnings),
                // A stop that fails after the window "captured" means
                // the door died in the race — the same loss, named the
                // same way.
                Ok(other) => {
                    return Err(
                        GwpError::Protocol(format!("StopRecording answered by {other:?}")).into(),
                    );
                }
                Err(_door_closed_in_the_race) => return Err(CodegenError::RecordingLost),
            };
            if let Some(out) = &launch.out {
                std::fs::write(out, &code)?;
            }
            Ok(CodegenOutcome {
                code,
                warnings,
                adapter,
            })
        }
    }
}

enum WindowEnd {
    /// The window closed while the door still serves: stop and capture.
    Captured,
    /// The door closed first: the recorder died with the session.
    Lost,
}

/// The capture window: end on the deadline or Ctrl-C (capture), or when
/// the app's door stops answering (lost). Liveness is a `Ping` round
/// trip per poll — no subscription, no idle traffic, the honest
/// "is the app still there" probe.
async fn capture_window(connection: &mut Connection, duration_ms: Option<u64>) -> WindowEnd {
    let deadline = duration_ms.map(|ms| Instant::now() + Duration::from_millis(ms.max(1)));
    loop {
        let now = Instant::now();
        if deadline.is_some_and(|d| now >= d) {
            return WindowEnd::Captured;
        }
        let wait = deadline
            .and_then(|d| d.checked_duration_since(now))
            .map_or(Duration::from_millis(LIVENESS_POLL_MS), |left| {
                left.min(Duration::from_millis(LIVENESS_POLL_MS))
            });
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            // Ctrl-C ends the window as a capture: on unix the child
            // rides its own process group (spawn_app), so only the
            // launcher sees the signal.
            _ = tokio::signal::ctrl_c() => return WindowEnd::Captured,
        }
        match connection.request(RequestV1::Ping).await {
            Ok(_) => {}
            Err(_door_closed) => return WindowEnd::Lost,
        }
    }
}

/// Spawn the app with the §5.3 launch-mode env. On unix the child gets
/// its own process group: the Ctrl-C that ends the capture window
/// reaches only the launcher, which then reaps the app itself.
fn spawn_app(launch: &CodegenLaunch, env_value: &str) -> Result<std::process::Child, CodegenError> {
    let Some((program, args)) = launch.command.split_first() else {
        return Err(CodegenError::Spawn {
            command: String::new(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty app command"),
        });
    };
    let mut cmd = StdCommand::new(program);
    cmd.args(args);
    cmd.env("GUIWRIGHT_DRIVER", env_value);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().map_err(|source| CodegenError::Spawn {
        command: program.clone(),
        source,
    })
}

/// Reap the app: the launcher owns the child it spawned. `kill` on an
/// already-exited child is an error — best-effort by contract.
fn reap_child(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The §5.3 launch-mode value the app reads to serve its door.
fn door_env_value(door: &ConnectTarget) -> Result<String, CodegenError> {
    match door {
        ConnectTarget::Uds(path) => Ok(format!("uds://{}", path.display())),
        ConnectTarget::Tcp(addr) => Ok(format!("tcp://{addr}")),
        ConnectTarget::Ws(_) => Err(CodegenError::UnsupportedDoor),
    }
}

/// The door as an error message names it (a socket path or address —
/// `ConnectTarget` carries no `Display`).
fn door_display(door: &ConnectTarget) -> String {
    match door {
        ConnectTarget::Uds(path) => path.display().to_string(),
        ConnectTarget::Tcp(addr) | ConnectTarget::Ws(addr) => addr.clone(),
    }
}

/// The default launch door: a fresh UDS path in the temp dir, unique per
/// process (nanosecond stamp + a monotonic launcher sequence) so
/// concurrent launches never collide.
fn default_door() -> ConnectTarget {
    use std::sync::atomic::{AtomicU64, Ordering};
    static DOOR_SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let seq = DOOR_SEQ.fetch_add(1, Ordering::Relaxed);
    ConnectTarget::Uds(std::env::temp_dir().join(format!(
        "guiwright-codegen-{}-{}-{}.sock",
        std::process::id(),
        nanos,
        seq
    )))
}

/// The ack-only `Inspect` commands answer `Pong` (§7); anything else is
/// a protocol bug worth naming, not squinting at.
fn expect_pong(reply: ResponseV1, what: &'static str) -> Result<(), GwpError> {
    match reply {
        ResponseV1::Pong => Ok(()),
        other => Err(GwpError::Protocol(format!("{what} answered by {other:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn door_env_value_speaks_the_launch_mode_forms() {
        let uds = ConnectTarget::Uds(PathBuf::from("/tmp/app.gwp"));
        assert_eq!(
            door_env_value(&uds).expect("uds form"),
            "uds:///tmp/app.gwp"
        );
        let tcp = ConnectTarget::Tcp("127.0.0.1:41234".into());
        assert_eq!(
            door_env_value(&tcp).expect("tcp form"),
            "tcp://127.0.0.1:41234"
        );
        let ws = ConnectTarget::Ws("ws://127.0.0.1:41234".into());
        assert!(
            matches!(door_env_value(&ws), Err(CodegenError::UnsupportedDoor)),
            "ws doors are attach targets, not launch targets"
        );
    }

    #[test]
    fn default_door_is_a_fresh_temp_socket() {
        let first = default_door();
        let second = default_door();
        let ConnectTarget::Uds(first) = &first else {
            panic!("the default door is uds");
        };
        let ConnectTarget::Uds(second) = &second else {
            panic!("the default door is uds");
        };
        assert!(
            first
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("guiwright-codegen-")),
            "the temp name names the launcher: {first:?}"
        );
        assert_ne!(first, second, "two launches never share a door path");
    }
}
