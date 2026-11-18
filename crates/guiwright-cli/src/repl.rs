//! `guiwright repl` — the live session debugger (architecture §13 "a
//! REPL attaches to a live session for ad-hoc locators/eval" + §16 v0.4
//! "REPL / debugging — `guiwright repl` over GWP").
//!
//! The core is a pure [`Repl`] structure: feed it one input line, get
//! structured output lines back. The binary (and the tests) drive the
//! same path — scripted use and interactive use cannot diverge.
//!
//! Every command maps onto an existing GWP request (§7): the protocol is
//! unchanged. Rejections print as error lines and never abort the
//! session — a debugging tool must survive its user's mistakes.

use std::fmt::Write;
use std::time::Duration;

use crate::session::{ConnectTarget, Connection};
use guiwright_proto::GwpError;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{
    AdapterInfoV1, ClockV1, DescribeAllV1, DescribeV1, DescribedV1, ErrorCode, EventKindV1,
    EventV1, GestureV1, GwpEvent, InputEventV1, InspectV1, MockV1, NodeKeyV1, PointerButtonV1,
    RequestV1, ResponseV1, ScreenshotV1, SelectorV1, SemActionV1, SnapshotV1, SubscribeV1,
    TextMatchV1, VideoOptsV1, VideoV1, WindowV1,
};
use guiwright_proto::wait::WaitState;

/// One output line of a REPL step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// Command result (stdout).
    Ok(String),
    /// Informational (help, banner, hints) — stdout.
    Info(String),
    /// A strict-mode narrowing notice recorded while codegen ran (§13).
    Warn(String),
    /// A failed command — stderr in the interactive front-end.
    Err(String),
}

impl Line {
    /// The rendered form, as the front-end prints it (`error: …` for
    /// [`Line::Err`], verbatim otherwise).
    pub fn render(&self) -> String {
        match self {
            Line::Ok(s) | Line::Info(s) => s.clone(),
            Line::Warn(s) => format!("warning: {s}"),
            Line::Err(s) => format!("error: {s}"),
        }
    }
}

/// A resolved node handle held by the REPL — the "current node" follow-up
/// actions apply to (§8.2: handle + generation, the ABA guard).
#[derive(Clone, Debug, PartialEq)]
pub struct CurrentNode {
    pub node: NodeKeyV1,
    pub generation: u32,
    pub rect: Option<[f64; 4]>,
    /// Human-readable description (the selector form).
    pub describe: String,
}

/// The result of one REPL step.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StepOutcome {
    pub lines: Vec<Line>,
    /// `bye`/`exit`/`quit` — the front-end should close the session.
    pub exit: bool,
}

/// The live session debugger. Connect through [`Repl::connect`], feed
/// lines through [`Repl::step`].
#[derive(Debug)]
pub struct Repl {
    conn: Connection,
    /// Adapter info from the handshake (§7 `Welcome`).
    adapter: AdapterInfoV1,
    /// Resolve/action deadline in ms (the REPL `timeout` command, §8.3
    /// "configurable per action/locator/global").
    timeout_ms: u64,
    /// The current node handle (actions apply to it).
    current: Option<CurrentNode>,
    /// This connection's active window target (§16 v0.4, M6): (handle,
    /// title). Display-only state — the server bridge scopes the requests,
    /// the REPL just remembers what it aimed at.
    window: Option<(String, String)>,
    /// The §7 event subscription (M12): `events on` sets this; events
    /// drain between commands (see [`Repl::drain_events`]).
    events_subscribed: bool,
    /// Set by `bye`/`exit`/`quit`.
    exit_requested: bool,
}

impl Repl {
    /// Attach to a remote-mode session and perform the §7 handshake.
    pub async fn connect(target: &ConnectTarget) -> Result<Self, GwpError> {
        let mut conn = Connection::connect(target).await?;
        let adapter = conn.handshake(crate::AGENT).await?;
        Ok(Repl {
            conn,
            adapter,
            timeout_ms: 5_000,
            current: None,
            window: None,
            events_subscribed: false,
            exit_requested: false,
        })
    }

    /// The adapter info from the handshake (name/framework/caps).
    pub fn adapter(&self) -> &AdapterInfoV1 {
        &self.adapter
    }

    /// The current node handle, if one is resolved.
    pub fn current(&self) -> Option<&CurrentNode> {
        self.current.as_ref()
    }

    /// The configured timeout (ms).
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    /// Override the resolve/action deadline (the runner injects the
    /// spec's `timeout_ms` here; client-side only, no wire change).
    pub fn set_timeout_ms(&mut self, ms: u64) {
        self.timeout_ms = ms;
    }

    /// Execute one input line. Blank lines and `#` comments produce no
    /// output; unknown or malformed commands produce exactly one error
    /// line; every failure leaves the session usable.
    pub async fn step(&mut self, input: &str) -> StepOutcome {
        let trimmed = input.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return StepOutcome::default();
        }
        let tokens = match tokenize(trimmed) {
            Ok(tokens) => tokens,
            Err(err) => return Self::err(err),
        };
        match self.execute(&tokens).await {
            Ok(lines) => {
                let drained = self.drain_events().await;
                StepOutcome {
                    lines: drained.into_iter().chain(lines).collect(),
                    exit: self.exit_requested,
                }
            }
            Err(err) => {
                let drained = self.drain_events().await;
                let mut lines = drained;
                lines.push(Line::Err(err));
                StepOutcome {
                    lines,
                    exit: self.exit_requested,
                }
            }
        }
    }

    /// The event drain between commands (§7, M12): while subscribed, the
    /// events that arrived while the last command ran print ahead of its
    /// output — the live-debug view. The contract is honest about the
    /// line-oriented shape: what landed mid-command is flushed here; what
    /// lands while the prompt waits shows up with the next command.
    async fn drain_events(&mut self) -> Vec<Line> {
        const MAX_DRAIN: usize = 32;
        if !self.events_subscribed {
            return Vec::new();
        }
        let mut lines = Vec::new();
        for _ in 0..MAX_DRAIN {
            match self.conn.poll_event().await {
                Ok(Some(event)) => lines.push(Line::Info(render_event(&event))),
                Ok(None) => break,
                Err(err) => {
                    lines.push(Line::Warn(format!(
                        "event stream broke ({err}) — re-run `events on` to resubscribe"
                    )));
                    self.events_subscribed = false;
                    break;
                }
            }
        }
        if lines.len() == MAX_DRAIN {
            lines.push(Line::Info("… more events pending".into()));
        }
        lines
    }

    /// The `last` line: the current node's identity + rect (§8.2 output).
    fn render_current(node: Option<&CurrentNode>) -> Line {
        match node {
            Some(node) => {
                let mut text = format!("{} (generation {})", node.describe, node.generation);
                if let Some([x0, y0, x1, y1]) = node.rect {
                    let _ = write!(text, " rect=[{x0:.0},{y0:.0} → {x1:.0},{y1:.0}]");
                }
                Line::Ok(text)
            }
            None => Line::Info("no node resolved — run `get …` first".into()),
        }
    }

    fn err(message: impl Into<String>) -> StepOutcome {
        StepOutcome {
            lines: vec![Line::Err(message.into())],
            exit: false,
        }
    }

    async fn request(&mut self, request: RequestV1) -> Result<ResponseV1, String> {
        self.conn
            .request(request)
            .await
            .map_err(|err| render_gwp_error(&err))
    }

    async fn execute(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        match tokens[0].as_str() {
            "help" | "?" => Ok(Self::help()),
            "ping" => {
                self.request(RequestV1::Ping).await?;
                Ok(vec![Line::Ok("pong".into())])
            }
            "snapshot" => self.snapshot_command(tokens).await,
            "events" => self.events_command(tokens).await,
            "timeout" => {
                let ms = parse_ms(tokens.get(1))?;
                self.timeout_ms = ms;
                Ok(vec![Line::Ok(format!("timeout = {ms} ms"))])
            }
            "last" => Ok(vec![Self::render_current(self.current.as_ref())]),
            "get" => self.get_command(tokens).await,
            "hit" => self.hit_command(tokens).await,
            "describe" => self.describe_command(tokens).await,
            "count" => self.count_command(tokens).await,
            "all" => self.all_command(tokens).await,
            "texts" => self.texts_command(tokens).await,
            "values" => self.values_command(tokens).await,
            "wait" => self.wait_command(tokens).await,
            "expect" => {
                // The runner's assertion wording over the same resolve
                // path (M5): like `get`, stores the current node, but
                // failures read as expectation failures.
                let mut get_tokens = vec!["get".to_string()];
                get_tokens.extend_from_slice(&tokens[1..]);
                match self.get_command(&get_tokens).await {
                    Ok(lines) => Ok(lines),
                    Err(err) => Err(format!("expectation failed: {err}")),
                }
            }
            "click" => self.simple_action(tokens, SemActionV1::Click).await,
            "dblclick" => self.simple_action(tokens, SemActionV1::DoubleClick).await,
            "hover" => self.simple_action(tokens, SemActionV1::Hover).await,
            "focus" => self.simple_action(tokens, SemActionV1::Focus).await,
            "blur" => self.simple_action(tokens, SemActionV1::Blur).await,
            "check" => self.simple_action(tokens, SemActionV1::Check).await,
            "uncheck" => self.simple_action(tokens, SemActionV1::Uncheck).await,
            "fill" | "setvalue" => {
                let text = one_argument(tokens, "fill <text>")?;
                self.action(SemActionV1::SetValue { text }).await
            }
            "press" => {
                let key = tokens
                    .get(1)
                    .ok_or("`press` needs a key — e.g. `press Enter`")?
                    .clone();
                let modifiers = tokens[2..].to_vec();
                self.action(SemActionV1::Press { key, modifiers }).await
            }
            "scroll" => {
                let dx = tokens
                    .get(1)
                    .ok_or("`scroll` needs dx and dy — e.g. `scroll 0 -120`")?
                    .parse::<f64>()
                    .map_err(|_| format!("`dx` is not a number: {}", tokens[1]))?;
                let dy = tokens
                    .get(2)
                    .ok_or("`scroll` needs dx and dy — e.g. `scroll 0 -120`")?
                    .parse::<f64>()
                    .map_err(|_| format!("`dy` is not a number: {}", tokens[2]))?;
                self.action(SemActionV1::Scroll { dx, dy }).await
            }
            "clock" => self.clock_command(tokens).await,
            "mock" => self.mock_command(tokens).await,
            "inspect" => self.inspect_command(tokens).await,
            "codegen" => self.codegen_command(tokens).await,
            "video" => self.video_command(tokens).await,
            "window" => self.window_command(tokens).await,
            "mouse" => self.mouse_command(tokens).await,
            "screenshot" => self.screenshot_command(tokens).await,
            "key" => {
                let key = tokens
                    .get(1)
                    .ok_or("`key` needs a key — e.g. `key Enter`")?
                    .clone();
                let modifiers = tokens[2..].to_vec();
                self.gesture_request(GestureV1::Key { key, modifiers })
                    .await
            }
            "text" => {
                let text = one_argument(tokens, "text <string>")?;
                self.gesture_request(GestureV1::Type { text }).await
            }
            "bye" | "exit" | "quit" => {
                let lines = match self.request(RequestV1::Bye).await {
                    Ok(_) => vec![Line::Ok("session closed".into())],
                    // The reply races the disconnect; a failed Bye still
                    // ends the loop (the session is gone either way).
                    Err(_) => vec![Line::Info("session closed".into())],
                };
                self.exit_requested = true;
                Ok(lines)
            }
            other => Err(format!(
                "unknown command `{other}` — type `help` for the command list"
            )),
        }
    }

    // --- inspector overlay (§13, M8) ---------------------------------------

    /// `inspect on | off | clear | <get-args…>` (§13): the live overlay
    /// session. `inspect <get-args…>` resolves like `get` (same bookkeeping,
    /// so `last`/actions keep working) and aims the hover outline at the
    /// resolved handle — the REPL's inspect-the-tree loop. Every command
    /// maps 1:1 onto an `Inspect` request; the overlay itself composites
    /// server-side at present time.
    async fn inspect_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        match tokens.get(1).map(String::as_str) {
            Some("on") => {
                self.request(RequestV1::Inspect(InspectV1::EnableOverlay))
                    .await?;
                Ok(vec![Line::Ok(
                    "inspect: overlay on — badges track interactive nodes".into(),
                )])
            }
            Some("off") => {
                self.request(RequestV1::Inspect(InspectV1::DisableOverlay))
                    .await?;
                Ok(vec![Line::Ok("inspect: overlay off".into())])
            }
            Some("clear") => {
                self.request(RequestV1::Inspect(InspectV1::Highlight { node: None }))
                    .await?;
                Ok(vec![Line::Ok("inspect: outline cleared".into())])
            }
            Some(_) => {
                // `inspect <get-args…>`: the exact `get` resolution path,
                // then `Highlight` with the resolved handle.
                let mut get_tokens = vec!["get".to_string()];
                get_tokens.extend_from_slice(&tokens[1..]);
                let mut lines = self.get_command(&get_tokens).await?;
                let node = self.current.as_ref().map(|current| current.node.clone());
                let describe = self
                    .current
                    .as_ref()
                    .map(|current| current.describe.clone())
                    .unwrap_or_default();
                self.request(RequestV1::Inspect(InspectV1::Highlight { node }))
                    .await?;
                lines.push(Line::Ok(format!("highlighted {describe}")));
                Ok(lines)
            }
            None => Err(
                "`inspect` needs `on`, `off`, `clear` or get-args — e.g. `inspect role Button Save"
                    .into(),
            ),
        }
    }

    // --- locator resolution (§8) --------------------------------------------

    async fn get_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        let (selector, describe) = parse_selector(
            tokens
                .get(1..)
                .ok_or("`get` needs a locator — role | text | label | testid | value")?,
        )?;
        match self
            .request(RequestV1::Resolve {
                selector,
                timeout_ms: self.timeout_ms,
            })
            .await?
        {
            ResponseV1::Resolved {
                node,
                generation,
                rect,
            } => {
                let mut rendered = format!("resolved {describe} (generation {generation})");
                if let Some([x0, y0, x1, y1]) = rect {
                    let _ = write!(rendered, " rect=[{x0:.0},{y0:.0} → {x1:.0},{y1:.0}]");
                }
                self.current = Some(CurrentNode {
                    node,
                    generation,
                    rect,
                    describe,
                });
                Ok(vec![Line::Ok(rendered)])
            }
            other => Err(unexpected("Resolved", &other)),
        }
    }

    // --- hit-test read (§8.3's mechanism over the wire, M32) ---------------

    /// `hit <x> <y>`: which node occupies the point — physical px, the
    /// same space `get`'s rects print, so the coordinates of a resolve
    /// reply can be fed straight back. A hit stores the current node (so
    /// `last` and the actions aim at it — the click-what-you-see loop);
    /// a miss is an info line, not an error (the read is total), and it
    /// leaves the current node untouched.
    async fn hit_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        let x = tokens
            .get(1)
            .ok_or("`hit` needs x and y — e.g. `hit 76 32`")?
            .parse::<f64>()
            .map_err(|_| format!("`x` is not a number: {}", tokens[1]))?;
        let y = tokens
            .get(2)
            .ok_or("`hit` needs x and y — e.g. `hit 76 32`")?
            .parse::<f64>()
            .map_err(|_| format!("`y` is not a number: {}", tokens[2]))?;
        if tokens.len() > 3 {
            return Err("`hit` takes exactly two arguments — `hit <x> <y>`".into());
        }
        match self.request(RequestV1::HitTest { x, y }).await? {
            ResponseV1::HitTest { hit: Some(match_) } => {
                // The handle form is the action vocabulary (`last`, the
                // actions aim at it) and the facts' fallback shape.
                let handle = describe_node(&match_.node);
                // The M37 enrichment: one `Describe` round trip behind the
                // hit gives the row its role/name (the m32 record's named
                // rendering gap). An enrichment failure degrades to the
                // bare handle form — the hit itself is the read, and the
                // facts ride one extra round trip (a node removed between
                // the two reads is the honest case: the hit happened, the
                // facts are gone).
                let facts = self
                    .describe_facts(&match_.node, match_.generation)
                    .await
                    .unwrap_or_else(|| handle.clone());
                let mut rendered = format!(
                    "hit ({x}, {y}) → {facts} ({handle}) (generation {})",
                    match_.generation
                );
                if let Some([x0, y0, x1, y1]) = match_.rect {
                    let _ = write!(rendered, " rect=[{x0:.0},{y0:.0} → {x1:.0},{y1:.0}]");
                }
                self.current = Some(CurrentNode {
                    node: match_.node,
                    generation: match_.generation,
                    rect: match_.rect,
                    describe: handle,
                });
                Ok(vec![Line::Ok(rendered)])
            }
            ResponseV1::HitTest { hit: None } => {
                Ok(vec![Line::Info(format!("nothing at ({x}, {y})"))])
            }
            other => Err(unexpected("HitTest", &other)),
        }
    }

    /// `describe <get-args…>` (M37 — the m32/m36 records' named REPL
    /// consumer): one node's identity facts — role, accessible name,
    /// value, bounds — through the M36 `Describe` domain. The locator
    /// resolves strict-mode first (the same surface `get` speaks, but
    /// with the M35 read-pass's snapshot semantics — `timeout_ms: 0`, no
    /// arrival wait; narrowing rides the selector), then the handle
    /// answers the facts — the dump row's facts, one node instead of a
    /// walk. A read, not a resolve-store: `last` and the actions keep
    /// aiming where `get`/`hit` aimed (the M35 read rule), and a missing
    /// match is the immediate strict-mode error, never a partial row.
    async fn describe_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        let (selector, _describe) = parse_selector(
            tokens
                .get(1..)
                .ok_or("`describe` needs a locator — role | text | label | testid | value")?,
        )?;
        let ResponseV1::Resolved {
            node, generation, ..
        } = self
            .request(RequestV1::Resolve {
                selector,
                // The M35 snapshot-read rule: a debugging read never
                // waits — an absent match errors immediately (the m36
                // record's semantics, and `describe` is its REPL row).
                timeout_ms: 0,
            })
            .await?
        else {
            return Err("`describe` resolves a locator first — unexpected Resolve reply".into());
        };
        let ResponseV1::Described(row) = self
            .request(RequestV1::Describe(DescribeV1 {
                node: node.clone(),
                generation,
            }))
            .await?
        else {
            return Err("`describe` reads the resolved handle — unexpected Describe reply".into());
        };
        Ok(vec![Line::Ok(render_described(&row))])
    }

    /// The M37 hit-row enrichment's round trip: role/name for a handle,
    /// `None` when the facts cannot ride (an old driver, a node that left
    /// between the hit and the describe).
    async fn describe_facts(&mut self, node: &NodeKeyV1, generation: u32) -> Option<String> {
        match self
            .request(RequestV1::Describe(DescribeV1 {
                node: node.clone(),
                generation,
            }))
            .await
        {
            Ok(ResponseV1::Described(row)) => {
                let mut text = row.role;
                if let Some(name) = row.name.as_deref() {
                    let _ = write!(text, " {name:?}");
                }
                Some(text)
            }
            _ => None,
        }
    }

    // --- set reads (the M30 domain's REPL rows, M35) -----------------------

    /// `count <get-args…>` (M35 — the m30 record's named REPL gap): the
    /// set read as a number — how many nodes match right now, strict
    /// mode never consulted. A read, not a resolve-store: `last` and the
    /// actions keep aiming where `get`/`hit` aimed, and an empty set
    /// answers 0 (the snapshot rule, never an error). One `ResolveAll`
    /// round trip — the same domain the TypeScript `resolveAll()` speaks.
    async fn count_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        let (selector, describe) = parse_selector(
            tokens
                .get(1..)
                .ok_or("`count` needs a locator — role | text | label | testid | value")?,
        )?;
        match self.request(RequestV1::ResolveAll { selector }).await? {
            ResponseV1::ResolvedAll { matches } => Ok(vec![Line::Ok(format!(
                "{} match{} ({describe})",
                matches.len(),
                if matches.len() == 1 { "" } else { "es" }
            ))]),
            other => Err(unexpected("ResolvedAll", &other)),
        }
    }

    /// `all <get-args…>` (M35): the set read as rows — one line per
    /// match in document order, each row directly usable as the
    /// `Action` handle (the dump-row rule, M11). A read like `count`:
    /// no current-node store. An empty set is one info line — the read
    /// is total, never an error.
    ///
    /// The rows are the M39 enrichment (the m37 record's standing gap —
    /// "enumerating a set's facts would open one `Describe` round trip
    /// per row"): one `DescribeAll` round trip carries the identity
    /// facts (role/name/value) beside the generation and rect, the
    /// `describe` row's rendering at set width. A driver that predates
    /// M39 degrades silently to the M35 rows (identity + generation +
    /// rect over `ResolveAll`) — the m37 enrichment rule: the read
    /// itself is the answer, the facts ride one round trip.
    async fn all_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        let (selector, _) = parse_selector(
            tokens
                .get(1..)
                .ok_or("`all` needs a locator — role | text | label | testid | value")?,
        )?;
        match self
            .request(RequestV1::DescribeAll(DescribeAllV1 {
                selector: selector.clone(),
            }))
            .await
        {
            Ok(ResponseV1::DescribedAll { rows }) if rows.is_empty() => {
                Ok(vec![Line::Info("no matches".into())])
            }
            Ok(ResponseV1::DescribedAll { rows }) => Ok(rows
                .iter()
                .map(|row| Line::Ok(render_described(row)))
                .collect()),
            // The enrichment rides one round trip; a driver without the
            // M39 domain falls back to the M35 walk (the m37 degrade
            // rule — a graceful, documented fallback, not an error).
            Err(_) => match self.request(RequestV1::ResolveAll { selector }).await? {
                ResponseV1::ResolvedAll { matches } if matches.is_empty() => {
                    Ok(vec![Line::Info("no matches".into())])
                }
                ResponseV1::ResolvedAll { matches } => Ok(matches
                    .iter()
                    .map(|row| {
                        let mut text = format!(
                            "{} (generation {})",
                            describe_node(&row.node),
                            row.generation
                        );
                        if let Some([x0, y0, x1, y1]) = row.rect {
                            let _ = write!(text, " rect=[{x0:.0},{y0:.0} → {x1:.0},{y1:.0}]");
                        }
                        Line::Ok(text)
                    })
                    .collect()),
                other => Err(unexpected("ResolvedAll", &other)),
            },
            Ok(other) => Err(unexpected("DescribedAll", &other)),
        }
    }

    /// `texts <get-args…>` (M40 — the m35 record's named follow-up: "a
    /// `texts <get-args…>` row lands when a debugging session asks for
    /// it — one more client-side rendering over the same domain"): the
    /// set's *content* read, the m29 kit reader's REPL half. Each row is
    /// the match's text in document order — the accessible name first,
    /// the value as the fallback (the source order the text locators
    /// match, the `HasText` check's read) — beside the handle form the
    /// action vocabulary aims with. One `DescribeAll` round trip (M39):
    /// the walk's rows already carry the name and the value, so the read
    /// costs what `all` costs — the m35 batch note's one-`Describe`-per-
    /// row objection is retired by the domain that shipped after it.
    ///
    /// A read like `count`/`all`: no current-node store, strict mode
    /// aside (the non-strict walk, hidden and disabled included), an
    /// empty set answers one info line — never an error. A node with
    /// neither name nor value renders `""`, keeping row positions 1:1
    /// with the document order the set read established (the m31 rule).
    async fn texts_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        self.bulk_content_command(tokens, false).await
    }

    /// `values <get-args…>` (M40 — the m31 kit reader's REPL half, the
    /// m35 record's named follow-up): the [`Self::texts_command`] read
    /// with the source order flipped for form semantics — the value
    /// first, the accessible name as the fallback (the `getByValue`
    /// source order) — so the fixture's Email input answers
    /// `"ada@example.dev"` here and `"Email"` to `texts`, and the two
    /// reads can never silently agree on the wrong datum. Everything
    /// else is the same shape: one `DescribeAll` round trip, the
    /// snapshot-read family rules, `""` for a node with neither fact.
    async fn values_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        self.bulk_content_command(tokens, true).await
    }

    /// The shared bulk-content assembly: both rows differ only in the
    /// source order they read the `DescribedV1` facts with (and the
    /// [`NotWaiter`]-free shape — the read is a snapshot, no waiter).
    /// `value_first` selects the m31 order over the m29 one.
    ///
    /// The fallback is the m35 batch note's cost shape, honestly kept:
    /// a driver that predates the M39 domain cannot answer the content
    /// in one round trip, so the read degrades to the `ResolveAll` walk
    /// plus one `Describe` per row — the exact cost the m35 record
    /// named when it left the rows out. A row whose `Describe` fails
    /// (the node left between the reads — the m37 honest case) renders
    /// `""` rather than dropping the row: positions stay 1:1 with the
    /// walk.
    async fn bulk_content_command(
        &mut self,
        tokens: &[String],
        value_first: bool,
    ) -> Result<Vec<Line>, String> {
        let (selector, _describe) = parse_selector(
            tokens
                .get(1..)
                .ok_or("needs a locator — role | text | label | testid | value")?,
        )?;
        match self
            .request(RequestV1::DescribeAll(DescribeAllV1 {
                selector: selector.clone(),
            }))
            .await
        {
            Ok(ResponseV1::DescribedAll { rows }) if rows.is_empty() => {
                Ok(vec![Line::Info("no matches".into())])
            }
            Ok(ResponseV1::DescribedAll { rows }) => Ok(rows
                .iter()
                .map(|row| Line::Ok(render_bulk_row(row, value_first)))
                .collect()),
            // The facts ride one round trip on an M39 driver; a driver
            // without the domain falls back to the per-row walk (the
            // m37 degrade rule — a graceful, documented fallback, not
            // an error).
            Err(_) => match self.request(RequestV1::ResolveAll { selector }).await? {
                ResponseV1::ResolvedAll { matches } if matches.is_empty() => {
                    Ok(vec![Line::Info("no matches".into())])
                }
                ResponseV1::ResolvedAll { matches } => {
                    let mut lines = Vec::with_capacity(matches.len());
                    for match_ in &matches {
                        let row = self.describe_row(&match_.node, match_.generation).await;
                        let (name, value) = row.map_or((None, None), |row| (row.name, row.value));
                        let content = if value_first {
                            value.or(name).unwrap_or_default()
                        } else {
                            name.or(value).unwrap_or_default()
                        };
                        lines.push(Line::Ok(format!(
                            "{} {content:?}",
                            describe_node(&match_.node)
                        )));
                    }
                    Ok(lines)
                }
                other => Err(unexpected("ResolvedAll", &other)),
            },
            Ok(other) => Err(unexpected("DescribedAll", &other)),
        }
    }

    /// The per-row facts fetch for the pre-M39 fallback: the whole
    /// `DescribedV1` a `Describe` round trip answers, `None` when the
    /// facts cannot ride (a node that left between the reads — the m37
    /// honest case; the caller renders `""` rather than dropping the
    /// row, keeping positions 1:1 with the walk).
    async fn describe_row(&mut self, node: &NodeKeyV1, generation: u32) -> Option<DescribedV1> {
        match self
            .request(RequestV1::Describe(DescribeV1 {
                node: node.clone(),
                generation,
            }))
            .await
        {
            Ok(ResponseV1::Described(row)) => Some(row),
            _ => None,
        }
    }

    /// `wait <state> <get-args…>` (m43 — the m42 client-side wait's
    /// interactive half, the m35/m40 one-milestone-per-surface pattern):
    /// the [`ClientConn::wait_for`] seam driven by the `timeout <ms>`
    /// budget. The states are the kit's `WaitState` vocabulary —
    /// attached / visible / hidden / stable — re-evaluated over the
    /// describe-all rows on every event wake. A positive state answers
    /// one ok line naming the satisfying row (the action vocabulary's
    /// handle form); `hidden` is satisfied by absence; a spent budget
    /// prints the typed timeout (state, locator, last evaluation). A
    /// read, not a resolve-store: `last` keeps the node `get` aimed at.
    /// The wait owns the event filter for its duration — an `events on`
    /// filter is restored afterwards (best effort), so the live-debug
    /// stream keeps flowing.
    async fn wait_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        let state = match tokens.get(1).map(String::as_str) {
            Some("attached") => WaitState::Attached,
            Some("visible") => WaitState::Visible,
            Some("hidden") => WaitState::Hidden,
            Some("stable") => WaitState::Stable,
            _ => return Err("wait needs a state — attached | visible | hidden | stable".into()),
        };
        let (selector, _describe) = parse_selector(
            tokens
                .get(2..)
                .ok_or("needs a locator — wait <state> <get-args…>")?,
        )?;
        let outcome = self
            .conn
            .wait_for(selector, state, Duration::from_millis(self.timeout_ms))
            .await;
        // The wait installed its own event filter; restore the `events on`
        // one (best effort — a broken stream surfaces at the next drain).
        if self.events_subscribed {
            let _ = self
                .conn
                .subscribe(&[
                    EventKindV1::TreeChanged,
                    EventKindV1::FramePresented,
                    EventKindV1::Log,
                ])
                .await;
        }
        match outcome {
            Ok(Some(row)) => {
                let mut line = format!(
                    "wait {} — {} {:?} (generation {})",
                    state.describe(),
                    describe_node(&row.node),
                    row.name.as_deref().unwrap_or_default(),
                    row.generation,
                );
                if let Some([x0, y0, x1, y1]) = row.bounds {
                    let _ = write!(line, " rect=[{x0:.0},{y0:.0} → {x1:.0},{y1:.0}]");
                }
                Ok(vec![Line::Ok(line)])
            }
            Ok(None) => Ok(vec![Line::Ok(format!(
                "wait {} — satisfied by absence (no visible rows)",
                state.describe()
            ))]),
            Err(err) => Err(render_gwp_error(&err)),
        }
    }

    // --- actions (§8.4) -------------------------------------------------------

    async fn simple_action(
        &mut self,
        tokens: &[String],
        action: SemActionV1,
    ) -> Result<Vec<Line>, String> {
        if tokens.len() > 1 {
            return Err(format!("`{}` takes no arguments", tokens[0]));
        }
        self.action(action).await
    }

    async fn action(&mut self, action: SemActionV1) -> Result<Vec<Line>, String> {
        let Some(current) = self.current.clone() else {
            return Err("no node resolved — run `get …` first".into());
        };
        match self
            .request(RequestV1::Action {
                node: current.node,
                generation: current.generation,
                action,
                force: false,
                timeout_ms: self.timeout_ms,
            })
            .await?
        {
            ResponseV1::ActionDone { frame_seq, detail } => Ok(vec![Line::Ok(format!(
                "{}: {detail} (frame {frame_seq})",
                current.describe
            ))]),
            other => Err(unexpected("ActionDone", &other)),
        }
    }

    // --- window domain (§16 v0.4, M6) -----------------------------------------

    /// `window list` / `window use <handle>` — the multi-window targets
    /// (§16 v0.4). `Use` aims THIS connection: subsequent locators resolve
    /// inside that window's subtree only (the server bridge scopes the
    /// requests; the REPL records the active handle for the prompt/list).
    async fn window_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        match tokens.get(1).map(String::as_str) {
            Some("list") | None => {
                if tokens.get(1).is_none() {
                    return Err("`window` needs a subcommand — list | use <handle>".into());
                }
                match self.request(RequestV1::Window(WindowV1::List)).await? {
                    ResponseV1::Windows { windows } => {
                        let mut lines =
                            vec![Line::Ok(format!("window targets ({}):", windows.len()))];
                        for w in &windows {
                            let marker = match &self.window {
                                Some((handle, _)) if *handle == w.handle => "*",
                                _ => " ",
                            };
                            let focus = if w.focused { " focused" } else { "" };
                            lines.push(Line::Ok(format!(
                                "{marker} {} {}{focus}",
                                w.handle, w.title
                            )));
                        }
                        Ok(lines)
                    }
                    other => Err(unexpected("Windows", &other)),
                }
            }
            Some("use") => {
                let handle = tokens
                    .get(2)
                    .ok_or("`window use` needs a handle — see `window list`")?
                    .clone();
                match self
                    .request(RequestV1::Window(WindowV1::Use { handle }))
                    .await?
                {
                    ResponseV1::WindowUsed { handle } => {
                        let title = self.listed_title(&handle).await.unwrap_or_default();
                        self.window = Some((handle.clone(), title.clone()));
                        Ok(vec![Line::Ok(format!(
                            "now targeting {handle}{}",
                            if title.is_empty() {
                                String::new()
                            } else {
                                format!(" ({title})")
                            }
                        ))])
                    }
                    other => Err(unexpected("WindowUsed", &other)),
                }
            }
            Some(other) => Err(format!(
                "unknown window command `{other}` — list | use <handle>"
            )),
        }
    }

    /// The title of a window target, for the `window use` confirmation
    /// line (a second `List` round-trip — cold path, interactive only).
    async fn listed_title(&mut self, handle: &str) -> Option<String> {
        match self.request(RequestV1::Window(WindowV1::List)).await {
            Ok(ResponseV1::Windows { windows }) => windows
                .into_iter()
                .find(|w| w.handle == handle)
                .map(|w| w.title),
            _ => None,
        }
    }

    // --- raw input & gestures (§9, M7) -------------------------------------

    /// The `mouse` domain: the synthesized gestures (`click | dblclick |
    /// drag | wheel`) and the raw primitive events (`move | down | up`).
    /// Everything rides the connection's active window target (M6) and
    /// window-relative logical-px coordinates (§9); the driver synthesizes
    /// the raw batch from its adapter's pacing table.
    async fn mouse_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        match tokens.get(1).map(String::as_str) {
            Some("click" | "dblclick") => {
                let count = if tokens[1] == "dblclick" { 2 } else { 1 };
                let (x, y) = two_nums(tokens, 2, "mouse click <x> <y> [right|middle]")?;
                let button = pointer_button(tokens.get(4))?;
                self.gesture_request(GestureV1::Click {
                    x,
                    y,
                    button,
                    count,
                })
                .await
            }
            Some("drag") => {
                let (from_x, from_y, to_x, to_y) =
                    four_nums(tokens, 2, "mouse drag <x1> <y1> <x2> <y2> [right|middle]")?;
                let button = pointer_button(tokens.get(6))?;
                self.gesture_request(GestureV1::Drag {
                    from_x,
                    from_y,
                    to_x,
                    to_y,
                    button,
                })
                .await
            }
            Some("wheel") => {
                let (dx, dy) = two_nums(tokens, 2, "mouse wheel <dx> <dy>")?;
                self.gesture_request(GestureV1::Wheel { dx, dy }).await
            }
            Some("move") => {
                let (x, y) = two_nums(tokens, 2, "mouse move <x> <y>")?;
                self.raw_input(vec![InputEventV1::PointerMove { x, y }])
                    .await
            }
            Some("down") => {
                let (x, y) = two_nums(tokens, 2, "mouse down <x> <y> [right|middle]")?;
                let button = pointer_button(tokens.get(4))?;
                self.raw_input(vec![InputEventV1::PointerDown { x, y, button }])
                    .await
            }
            Some("up") => {
                let (x, y) = two_nums(tokens, 2, "mouse up <x> <y> [right|middle]")?;
                let button = pointer_button(tokens.get(4))?;
                self.raw_input(vec![InputEventV1::PointerUp { x, y, button }])
                    .await
            }
            Some(other) => Err(format!(
                "unknown mouse command `{other}` — click | dblclick | drag | wheel | move | down | up"
            )),
            None => Err(
                "usage: mouse click | dblclick | drag | wheel | move | down | up — see `help`"
                    .into(),
            ),
        }
    }

    /// Send a gesture request and render the `GestureDone` synthesis
    /// summary.
    async fn gesture_request(&mut self, gesture: GestureV1) -> Result<Vec<Line>, String> {
        match self.request(RequestV1::Gesture(gesture)).await? {
            ResponseV1::GestureDone { frame_seq, detail } => {
                Ok(vec![Line::Ok(format!("{detail} @ frame {frame_seq}"))])
            }
            other => Err(unexpected("GestureDone", &other)),
        }
    }

    /// Send a raw event batch and render the `InputQueued` frame.
    async fn raw_input(&mut self, events: Vec<InputEventV1>) -> Result<Vec<Line>, String> {
        match self.request(RequestV1::Input { events }).await? {
            ResponseV1::InputQueued { frame_seq } => {
                Ok(vec![Line::Ok(format!("input queued @ frame {frame_seq}"))])
            }
            other => Err(unexpected("InputQueued", &other)),
        }
    }

    // --- clock domain (§14) -----------------------------------------------------

    async fn clock_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        let cmd = match tokens.get(1).map(String::as_str) {
            Some("install") => ClockV1::InstallVirtual,
            Some("ff") => ClockV1::FastForward {
                ms: parse_ms(tokens.get(2))?,
            },
            Some("runfor") => ClockV1::RunFor {
                ms: parse_ms(tokens.get(2))?,
            },
            Some("pause") => ClockV1::Pause,
            Some("resume") => ClockV1::Resume,
            Some("set") => ClockV1::SetTime {
                ms: parse_ms(tokens.get(2))?,
            },
            Some("state") | None => ClockV1::State,
            Some(other) => {
                return Err(format!(
                    "unknown clock command `{other}` — install | ff | runfor | pause | resume | set | state"
                ));
            }
        };
        match self.request(RequestV1::Clock(cmd)).await? {
            ResponseV1::ClockState {
                virtual_time_ms,
                paused,
                steps,
            } => {
                let time = virtual_time_ms.map_or_else(|| "wall".into(), |ms| format!("{ms}ms"));
                Ok(vec![Line::Ok(format!(
                    "clock: virtual={time} paused={paused} steps={steps}"
                ))])
            }
            other => Err(unexpected("ClockState", &other)),
        }
    }

    // --- mock domain (§14 state seams) -------------------------------------------

    async fn mock_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        let cmd = match tokens.get(1).map(String::as_str) {
            Some("install") => MockV1::InstallSeam {
                id: tokens
                    .get(2)
                    .ok_or("`mock install` needs a seam id")?
                    .clone(),
                response: tokens.get(3).cloned(),
            },
            Some("clear") => MockV1::Clear {
                id: tokens.get(2).ok_or("`mock clear` needs a seam id")?.clone(),
            },
            Some("call") => MockV1::Call {
                id: tokens.get(2).ok_or("`mock call` needs a seam id")?.clone(),
                scope: joined(tokens.get(3..)).unwrap_or_else(|| "default".into()),
            },
            Some("reset") => MockV1::ResetState {
                scope: joined(tokens.get(2..)).unwrap_or_else(|| "default".into()),
            },
            other => {
                return Err(format!(
                    "unknown mock command `{}` — install | clear | call | reset",
                    other.unwrap_or("")
                ));
            }
        };
        match self.request(RequestV1::Mock(cmd)).await? {
            ResponseV1::MockDone { id, detail } => {
                Ok(vec![Line::Ok(format!("mock {id}: {detail}"))])
            }
            other => Err(unexpected("MockDone", &other)),
        }
    }

    // --- inspect domain (§13 codegen, M4) -----------------------------------------

    async fn codegen_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        match tokens.get(1).map(String::as_str) {
            Some("start") => {
                self.request(RequestV1::Inspect(InspectV1::StartRecording))
                    .await?;
                Ok(vec![Line::Ok(
                    "codegen: recording — act with `get` + actions, then `codegen stop`".into(),
                )])
            }
            Some("stop") => {
                match self
                    .request(RequestV1::Inspect(InspectV1::StopRecording))
                    .await?
                {
                    ResponseV1::Codegen { code, warnings } => {
                        let mut lines = vec![Line::Ok(format!(
                            "codegen: {} line(s), {} warning(s)",
                            code.lines().count(),
                            warnings.len()
                        ))];
                        for code_line in code.lines() {
                            lines.push(Line::Ok(code_line.to_string()));
                        }
                        for warning in warnings {
                            lines.push(Line::Warn(warning));
                        }
                        Ok(lines)
                    }
                    other => Err(unexpected("Codegen", &other)),
                }
            }
            other => Err(format!(
                "unknown codegen command `{}` — start | stop",
                other.unwrap_or("")
            )),
        }
    }

    // --- video domain (§10) ---------------------------------------------------------

    async fn video_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        match tokens.get(1).map(String::as_str) {
            Some("start") => {
                let fps = match tokens.get(2) {
                    Some(value) => value
                        .parse::<u32>()
                        .map_err(|_| format!("`fps` is not a number: {value}"))?,
                    None => 0, // driver default (§10)
                };
                let dir = tokens.get(3).cloned();
                self.request(RequestV1::Video(VideoV1::Start(VideoOptsV1 { fps, dir })))
                    .await?;
                Ok(vec![Line::Ok("video: recording started".into())])
            }
            Some("stop") => match self.request(RequestV1::Video(VideoV1::Stop)).await? {
                ResponseV1::VideoFile {
                    path,
                    frames,
                    dropped,
                    duration_ms,
                } => Ok(vec![Line::Ok(format!(
                    "video: {path} ({frames} frames, {dropped} dropped, {duration_ms}ms)"
                ))]),
                other => Err(unexpected("VideoFile", &other)),
            },
            other => Err(format!(
                "unknown video command `{}` — start | stop",
                other.unwrap_or("")
            )),
        }
    }

    // --- tree dump (§7 tree domain, M11) -----------------------------------------

    /// `snapshot` — the summary (the M0 reply); `snapshot full` — the
    /// whole tree as data, depth-first per window root, indented by the
    /// `children` counts (§7 `Snapshot::Full` → `TreeDump`). The focused
    /// node is marked so live debugging sees selection changes.
    async fn snapshot_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        match tokens.get(1).map(String::as_str) {
            // `snapshot` (no subcommand): the M0 summary.
            None => match self.request(RequestV1::TreeSnapshot).await? {
                ResponseV1::TreeSnapshot {
                    tree_seq,
                    node_count,
                } => Ok(vec![Line::Ok(format!(
                    "tree @{tree_seq}: {node_count} nodes"
                ))]),
                other => Err(unexpected("TreeSnapshot", &other)),
            },
            // `snapshot full` (M11): the whole tree.
            Some("full") => Self::render_dump(
                self.request(RequestV1::Snapshot(SnapshotV1::Full)).await?,
                "full",
            ),
            // `snapshot under <get-args…>` (M13): one subtree, resolved
            // like `get` (auto-wait, strict mode, window scope).
            Some("under") => {
                let (selector, describe) = parse_selector(tokens.get(2..).ok_or(
                    "`snapshot under` needs a locator — e.g. `snapshot under role Group`",
                )?)?;
                let dump = self
                    .request(RequestV1::Snapshot(SnapshotV1::Filtered {
                        selector,
                        timeout_ms: self.timeout_ms,
                    }))
                    .await?;
                Self::render_dump(dump, &format!("under {describe}"))
            }
            Some(other) => Err(format!(
                "unknown `snapshot` form `{other}` — snapshot | snapshot full | snapshot under <get-args…>"
            )),
        }
    }

    /// Render a `TreeDump` reply: the header line, then the DFS rows
    /// indented by the rebuilt hierarchy (children counts + a running
    /// stack; each walk starts at depth 0, §16 v0.4).
    fn render_dump(resp: ResponseV1, form: &str) -> Result<Vec<Line>, String> {
        let ResponseV1::TreeDump {
            tree_seq,
            node_count,
            focus,
            nodes,
        } = resp
        else {
            return Err(unexpected("TreeDump", &resp));
        };
        let mut lines = vec![Line::Ok(format!(
            "tree @{tree_seq}: {node_count} nodes ({form})"
        ))];
        let mut remaining: Vec<u32> = Vec::new();
        for node in &nodes {
            while let Some(top) = remaining.last() {
                if *top == 0 {
                    remaining.pop();
                } else {
                    break;
                }
            }
            let depth = remaining.len();
            if depth > 0 {
                *remaining.last_mut().expect("depth > 0 implies a parent") -= 1;
            }
            let mut row = String::new();
            let _ = write!(row, "{}{}", "  ".repeat(depth), node.role);
            if let Some(name) = &node.name {
                let _ = write!(row, " \"{name}\"");
            }
            if let Some(value) = &node.value {
                let _ = write!(row, " = {value}");
            }
            if let Some([x0, y0, x1, y1]) = node.bounds {
                let _ = write!(row, " @ [{x0:.0},{y0:.0} → {x1:.0},{y1:.0}]");
            }
            if Some(&node.key) == focus.as_ref() {
                row.push_str(" (focus)");
            }
            lines.push(Line::Info(row));
            if node.children > 0 {
                remaining.push(node.children);
            }
        }
        Ok(lines)
    }

    // --- event channel (§7, M12) ---------------------------------------------

    /// `events on | off` (§7, M12 + the log-tail milestone): subscribe
    /// this connection to tree + frame events **and** the session log
    /// tail's records; the drain between commands prints what arrived (see
    /// [`Repl::drain_events`]). The `Log` kind needs the server-side tap
    /// (`ServerOptions::log_tail`) — a session spawned without it answers
    /// a typed `Unsupported` and the error surfaces here verbatim.
    async fn events_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        match tokens.get(1).map(String::as_str) {
            Some("on") => {
                let resp = self
                    .request(RequestV1::Subscribe(SubscribeV1::On {
                        kinds: vec![
                            EventKindV1::TreeChanged,
                            EventKindV1::FramePresented,
                            EventKindV1::Log,
                        ],
                    }))
                    .await?;
                let ResponseV1::Subscribed { kinds } = resp else {
                    return Err(unexpected("Subscribed", &resp));
                };
                self.events_subscribed = true;
                let names: Vec<&str> = kinds
                    .iter()
                    .map(|kind| match kind {
                        EventKindV1::TreeChanged => "tree_changed",
                        EventKindV1::FramePresented => "frame_presented",
                        EventKindV1::Log => "log",
                    })
                    .collect();
                Ok(vec![Line::Ok(format!(
                    "events: subscribed ({}) — flushed between commands",
                    names.join(", ")
                ))])
            }
            Some("off") => {
                self.request(RequestV1::Subscribe(SubscribeV1::Off)).await?;
                self.events_subscribed = false;
                Ok(vec![Line::Ok("events: unsubscribed".into())])
            }
            _ => Err("`events` needs `on` or `off`".into()),
        }
    }

    // --- render domain (§7, M10) -------------------------------------------------

    /// `screenshot [path]` — pull the window frame as PNG (§10 capture).
    /// The client owns the artifact path (Playwright parity): a missing
    /// argument writes `guiwright-shot-<epoch-ms>.png` into the process
    /// cwd. Element shots are the programmatic surface (the TypeScript
    /// client's `screenshotElement`) — the REPL holds no selector to
    /// re-resolve honestly.
    async fn screenshot_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        let path = match tokens.get(1) {
            Some(path) => path.clone(),
            None => format!(
                "guiwright-shot-{}.png",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis())
            ),
        };
        match self
            .request(RequestV1::Screenshot(ScreenshotV1::Window))
            .await?
        {
            ResponseV1::Shot {
                frame_seq,
                width,
                height,
                png,
            } => {
                std::fs::write(&path, &png).map_err(|e| format!("{path}: {e}"))?;
                Ok(vec![Line::Ok(format!(
                    "shot: {path} ({width}×{height}, frame {frame_seq})"
                ))])
            }
            other => Err(unexpected("Shot", &other)),
        }
    }

    // --- help -------------------------------------------------------------------------

    fn help() -> Vec<Line> {
        [
            "commands:",
            "  ping                        liveness probe",
            "  snapshot                    tree snapshot summary",
            "  snapshot full               the whole tree, indented (§7 dump)",
            "  snapshot under <get-args…>  one subtree, indented (M13)",
            "  events on | off             tree/frame events flushed between commands (M12)",
            "  get role <Role> [name]      resolve a locator (strict mode)",
            "  get text|label|value <text> resolve a locator (strict mode)",
            "  get testid <id>             resolve a locator (strict mode)",
            "  hit <x> <y>                 which node occupies the point (M32 hit-test read)",
            "  describe <get-args…>        one node's identity facts (M37)",
            "  count <get-args…>           the set size, strict mode aside (M35 read pass)",
            "  all <get-args…>             the set rows, one line each (M35 read pass)",
            "  texts <get-args…>           the set's text content, label first (M40)",
            "  values <get-args…>          the set's values, value first (M40)",
            "  wait <state> <get-args…>    attached|visible|hidden|stable (m43)",
            "  expect <get-args…>          resolve + assert (the runner wording)",
            "  click | dblclick | hover | focus | blur | check | uncheck",
            "  fill <text> | setvalue <text>",
            "  press <key> [modifiers…]    e.g. `press Enter`, `press a Ctrl`",
            "  scroll <dx> <dy>",
            "  mouse click <x> <y> [right]  synthetic click (§9, active window)",
            "  mouse dblclick | drag | wheel — synthesized with adapter pacing",
            "  mouse move | down | up <x> <y> — raw primitive pointer events",
            "  key <key> [modifiers…]      e.g. `key Enter`, `key a Ctrl`",
            "  text <string>               commit text in one step",
            "  clock install | ff <ms> | runfor <ms> | pause | resume | set <ms> | state",
            "  mock install <id> [response] | clear <id> | call <id> [scope]",
            "  mock reset <scope>",
            "  inspect on | off | clear   the §13 overlay session",
            "  inspect <get-args…>        resolve + highlight the node",
            "  window list                 the session's window targets",
            "  window use <handle>         aim this connection at one target",
            "  codegen start | stop        record actions as kit code (§13)",
            "  video start [fps] [dir] | video stop",
            "  screenshot [path]           pull a PNG shot of the window (§10)",
            "  timeout <ms>                resolve/action deadline",
            "  last                        show the current node handle",
            "  bye | exit | quit           close the session",
            "blank lines and `#` comments are ignored.",
        ]
        .into_iter()
        .map(|s| Line::Info(s.to_string()))
        .collect()
    }
}

/// Role names on the wire are the AccessKit `Debug` names (`Button`, …).
/// Accept the lowercased form too — `get role button Save` resolves like
/// `get role Button Save` (the driver degrades unknown names to
/// `Unknown`, which would silently miss).
fn canonical_role(role: &str) -> String {
    let mut chars = role.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The client-side handle form of a wire node identity (`accesskit#2`,
/// `iced#7`) — the same shape the core's `NodeKey::describe` renders
/// server-side. The hit-test reply carries no selector, so the handle is
/// the describe.
fn describe_node(node: &NodeKeyV1) -> String {
    match node {
        NodeKeyV1::AccessKit { node, .. } => format!("accesskit#{node}"),
        NodeKeyV1::Logical { adapter, id } => format!("{adapter}#{id}"),
    }
}

/// The M36 describe row, rendered for the REPL (M37): the dump row's
/// facts — `Button "Save" (generation 0) rect=[…]`, the value form
/// `TextInput "Email" value "…" …`. The value prints because it is the
/// fact the `value` locator vocabulary matches (§8.1) — the same
/// rendering the `snapshot full` rows use for content, one node.
fn render_described(row: &DescribedV1) -> String {
    let mut out = row.role.clone();
    if let Some(name) = row.name.as_deref() {
        let _ = write!(out, " {name:?}");
    }
    if let Some(value) = row.value.as_deref() {
        let _ = write!(out, " value {value:?}");
    }
    let _ = write!(out, " (generation {})", row.generation);
    if let Some([x0, y0, x1, y1]) = row.bounds {
        let _ = write!(out, " rect=[{x0:.0},{y0:.0} → {x1:.0},{y1:.0}]");
    }
    out
}

/// The M40 bulk-content row, rendered for the REPL: the handle form the
/// action vocabulary aims with (`accesskit#2`), the match's text in the
/// reader's source order quoted the way the dump rows quote content.
/// `value_first` selects the m31 order (value, label fallback) over the
/// m29 one (label, value fallback); a node with neither fact renders
/// `""` — row positions stay 1:1 with the walk (the m31 rule).
fn render_bulk_row(row: &DescribedV1, value_first: bool) -> String {
    let name = row.name.as_deref();
    let value = row.value.as_deref();
    let content = if value_first {
        value.or(name).unwrap_or_default()
    } else {
        name.or(value).unwrap_or_default()
    };
    format!("{} {content:?}", describe_node(&row.node))
}

/// Parse the `get`-shaped locator vocabulary shared by `get` (§8) and
/// `snapshot under` (M13): one selector + its rendered form (the same
/// describe both print). The tokens start at the locator kind — `role …`,
/// `text …`, `label …`, `testid …`, `value …`.
fn parse_selector(tokens: &[String]) -> Result<(SelectorV1, String), String> {
    let sub = tokens
        .first()
        .ok_or("needs a locator — role | text | label | testid | value")?;
    let selector = match sub.as_str() {
        "role" => {
            let role = tokens
                .get(1)
                .ok_or("`role` needs a role — e.g. `role Button Save`")?;
            let name = joined(tokens.get(2..));
            SelectorV1::Role {
                role: canonical_role(role),
                name: name.map(|text| TextMatchV1 {
                    text,
                    exact: false,
                    ignore_case: true,
                }),
            }
        }
        "text" => SelectorV1::Text(match_arg(tokens.get(1..), "text <text>")?),
        "label" => SelectorV1::Label(match_arg(tokens.get(1..), "label <text>")?),
        "testid" => SelectorV1::TestId(
            tokens
                .get(1)
                .ok_or("`testid` needs an id — e.g. `testid save-btn`")?
                .clone(),
        ),
        "value" => SelectorV1::Value(match_arg(tokens.get(1..), "value <text>")?),
        other => {
            return Err(format!(
                "unknown locator `{other}` — role | text | label | testid | value"
            ));
        }
    };
    let describe = render_selector(&selector);
    Ok((selector, describe))
}

/// The one-line event rendering between commands (§7, M12): the kind, the
/// seq, and the facts a debugging human wants (the frame hash).
fn render_event(event: &GwpEvent) -> String {
    let GwpEvent::V1(inner) = event;
    match inner {
        EventV1::TreeChanged { seq } => format!("event tree_changed seq={seq}"),
        EventV1::FramePresented { seq, hash } => {
            format!("event frame_presented seq={seq} hash={hash:#x}")
        }
        // The log-tail milestone: each record is one `logs.ndjson`-shape
        // JSON line — pretty-print level + target + message for the REPL
        // reader. A line that does not parse (an app-fed record in another
        // shape) prints raw: the wire format stays the source of truth.
        EventV1::Log { record } => match serde_json::from_str::<serde_json::Value>(record) {
            Ok(value) => format!(
                "event log {} {}: {}",
                value["level"].as_str().unwrap_or("?"),
                value["target"].as_str().unwrap_or("?"),
                value["message"].as_str().unwrap_or("?"),
            ),
            Err(_) => format!("event log {record}"),
        },
    }
}

/// Render a server rejection the way the front-end prints it: the error
/// code is part of the message (strict-mode violations, timeouts, stale
/// generations are all debugging signals, §8.2).
fn render_gwp_error(err: &GwpError) -> String {
    match err {
        GwpError::Rejected { code, message } => match code {
            ErrorCode::StrictModeViolation => format!("strict-mode: {message}"),
            ErrorCode::Timeout => format!("timeout: {message}"),
            ErrorCode::StaleGeneration => format!("stale node: {message}"),
            other => format!("{other:?}: {message}"),
        },
        other => other.to_string(),
    }
}

fn unexpected(expected: &str, got: &ResponseV1) -> String {
    format!("unexpected reply — expected {expected}, got {got:?}")
}

/// `"…"`-aware tokenizer: whitespace splits, double-quoted spans group
/// (with `\"` / `\\` escapes), `#` at a token boundary starts a comment.
fn tokenize(input: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if c == '#' {
            break;
        }
        let mut token = String::new();
        if c == '"' {
            chars.next();
            loop {
                match chars.next() {
                    Some('"') => break,
                    Some('\\') => match chars.next() {
                        Some(escaped) => token.push(escaped),
                        None => return Err("unterminated escape in quoted argument".into()),
                    },
                    Some(ch) => token.push(ch),
                    None => return Err("unterminated quoted argument".into()),
                }
            }
            tokens.push(token);
        } else {
            while let Some(&next) = chars.peek() {
                if next.is_whitespace() {
                    break;
                }
                token.push(next);
                chars.next();
            }
            tokens.push(token);
        }
    }
    Ok(tokens)
}

fn parse_ms(value: Option<&String>) -> Result<u64, String> {
    let value = value.ok_or("a millisecond value is required")?;
    value
        .parse::<u64>()
        .map_err(|_| format!("not a millisecond value: {value}"))
}

/// One required non-flag argument.
fn one_argument(tokens: &[String], usage: &str) -> Result<String, String> {
    tokens
        .get(1)
        .cloned()
        .ok_or_else(|| format!("`{usage}` needs an argument"))
}

/// Parse one f64 argument (the gesture vocabulary's coordinates/deltas).
fn parse_num(token: &str, what: &str) -> Result<f64, String> {
    token
        .parse::<f64>()
        .map_err(|_| format!("`{what}` is not a number: {token}"))
}

/// Two f64 arguments starting at `start` (e.g. `mouse click <x> <y>`).
fn two_nums(tokens: &[String], start: usize, usage: &str) -> Result<(f64, f64), String> {
    let x = tokens.get(start).ok_or_else(|| format!("usage: {usage}"))?;
    let y = tokens
        .get(start + 1)
        .ok_or_else(|| format!("usage: {usage}"))?;
    Ok((parse_num(x, "x")?, parse_num(y, "y")?))
}

/// Four f64 arguments starting at `start` (e.g. `mouse drag <x1> <y1> <x2> <y2>`).
fn four_nums(tokens: &[String], start: usize, usage: &str) -> Result<(f64, f64, f64, f64), String> {
    let (x1, y1) = two_nums(tokens, start, usage)?;
    let x2 = tokens
        .get(start + 2)
        .ok_or_else(|| format!("usage: {usage}"))?;
    let y2 = tokens
        .get(start + 3)
        .ok_or_else(|| format!("usage: {usage}"))?;
    Ok((x1, y1, parse_num(x2, "x2")?, parse_num(y2, "y2")?))
}

/// The optional trailing button word (`right` / `middle`); default primary.
fn pointer_button(token: Option<&String>) -> Result<PointerButtonV1, String> {
    match token.map(String::as_str) {
        None | Some("left" | "primary") => Ok(PointerButtonV1::Primary),
        Some("right" | "secondary") => Ok(PointerButtonV1::Secondary),
        Some("middle") => Ok(PointerButtonV1::Middle),
        Some(other) => Err(format!("unknown button `{other}` — left | right | middle")),
    }
}

/// The remaining tokens joined by single spaces (unquoted multi-word
/// names), or `None` when nothing follows.
fn joined(tokens: Option<&[String]>) -> Option<String> {
    let tokens = tokens?;
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

/// A required `TextMatchV1` argument (substring, case-insensitive — the
/// REPL's default matching posture).
fn match_arg(tokens: Option<&[String]>, usage: &str) -> Result<TextMatchV1, String> {
    let text = joined(tokens).ok_or(usage)?;
    Ok(TextMatchV1 {
        text,
        exact: false,
        ignore_case: true,
    })
}

/// `Button "Save"` — the reader-facing selector form.
fn render_selector(selector: &SelectorV1) -> String {
    match selector {
        SelectorV1::Role { role, name } => match name {
            Some(matched) => format!("{role} {:?}", matched.text),
            None => role.clone(),
        },
        SelectorV1::Text(m) => format!("text {:?}", m.text),
        SelectorV1::Label(m) => format!("label {:?}", m.text),
        SelectorV1::TestId(id) => format!("testid {id:?}"),
        SelectorV1::Value(m) => format!("value {:?}", m.text),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizer_groups_quotes_and_strips_comments() {
        assert_eq!(
            tokenize(r#"get role Button "Save All""#).unwrap(),
            vec![
                "get".to_string(),
                "role".to_string(),
                "Button".to_string(),
                "Save All".to_string()
            ]
        );
        assert_eq!(
            tokenize("click # trailing comment").unwrap(),
            vec!["click".to_string()]
        );
        assert_eq!(
            tokenize(r#"fill "say \"hi\"""#).unwrap(),
            vec!["fill".to_string(), r#"say "hi""#.to_string()]
        );
        assert_eq!(tokenize("   ").unwrap(), Vec::<String>::new());
        assert!(tokenize(r#"fill "open"#).is_err());
    }

    #[test]
    fn canonical_role_uppercases_the_first_letter() {
        assert_eq!(canonical_role("button"), "Button");
        assert_eq!(canonical_role("Button"), "Button");
        assert_eq!(canonical_role("TextInput"), "TextInput");
    }

    #[test]
    fn error_lines_render_with_prefix() {
        assert_eq!(Line::Err("boom".into()).render(), "error: boom");
        assert_eq!(Line::Warn("narrowed".into()).render(), "warning: narrowed");
        assert_eq!(Line::Ok("done".into()).render(), "done");
    }
}
