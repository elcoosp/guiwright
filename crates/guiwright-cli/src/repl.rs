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

use crate::session::{ConnectTarget, Connection};
use guiwright_proto::GwpError;
use guiwright_proto::schema::{
    AdapterInfoV1, ClockV1, ErrorCode, InspectV1, MockV1, NodeKeyV1, RequestV1, ResponseV1,
    SelectorV1, SemActionV1, TextMatchV1, VideoOptsV1, VideoV1, WindowV1,
};

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
            Err(err) => return self.err(err),
        };
        match self.execute(&tokens).await {
            Ok(lines) => StepOutcome {
                lines,
                exit: self.exit_requested,
            },
            Err(err) => self.err(err),
        }
    }

    fn err(&self, message: impl Into<String>) -> StepOutcome {
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
            "help" | "?" => Ok(self.help()),
            "ping" => {
                self.request(RequestV1::Ping).await?;
                Ok(vec![Line::Ok("pong".into())])
            }
            "snapshot" => match self.request(RequestV1::TreeSnapshot).await? {
                ResponseV1::TreeSnapshot {
                    tree_seq,
                    node_count,
                } => Ok(vec![Line::Ok(format!(
                    "tree @{tree_seq}: {node_count} nodes"
                ))]),
                other => Err(unexpected("TreeSnapshot", &other)),
            },
            "timeout" => {
                let ms = parse_ms(tokens.get(1))?;
                self.timeout_ms = ms;
                Ok(vec![Line::Ok(format!("timeout = {ms} ms"))])
            }
            "last" => Ok(vec![match &self.current {
                Some(node) => {
                    let mut text = format!("{} (generation {})", node.describe, node.generation);
                    if let Some([x0, y0, x1, y1]) = node.rect {
                        text.push_str(&format!(" rect=[{x0:.0},{y0:.0} → {x1:.0},{y1:.0}]"));
                    }
                    Line::Ok(text)
                }
                None => Line::Info("no node resolved — run `get …` first".into()),
            }]),
            "get" => self.get_command(tokens).await,
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
            "codegen" => self.codegen_command(tokens).await,
            "video" => self.video_command(tokens).await,
            "window" => self.window_command(tokens).await,
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

    // --- locator resolution (§8) --------------------------------------------

    async fn get_command(&mut self, tokens: &[String]) -> Result<Vec<Line>, String> {
        let sub = tokens
            .get(1)
            .ok_or("`get` needs a locator — role | text | label | testid | value")?;
        let selector = match sub.as_str() {
            "role" => {
                let role = tokens
                    .get(2)
                    .ok_or("`get role` needs a role — e.g. `get role Button Save`")?;
                let name = joined(tokens.get(3..));
                SelectorV1::Role {
                    role: canonical_role(role),
                    name: name.map(|text| TextMatchV1 {
                        text,
                        exact: false,
                        ignore_case: true,
                    }),
                }
            }
            "text" => SelectorV1::Text(match_arg(tokens.get(2..), "get text <text>")?),
            "label" => SelectorV1::Label(match_arg(tokens.get(2..), "get label <text>")?),
            "testid" => SelectorV1::TestId(
                tokens
                    .get(2)
                    .ok_or("`get testid` needs an id — e.g. `get testid save-btn`")?
                    .clone(),
            ),
            "value" => SelectorV1::Value(match_arg(tokens.get(2..), "get value <text>")?),
            other => {
                return Err(format!(
                    "unknown locator `{other}` — role | text | label | testid | value"
                ));
            }
        };
        let describe = render_selector(&selector);
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
                    rendered.push_str(&format!(" rect=[{x0:.0},{y0:.0} → {x1:.0},{y1:.0}]"));
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
                let time = virtual_time_ms
                    .map(|ms| format!("{ms}ms"))
                    .unwrap_or_else(|| "wall".into());
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

    // --- help -------------------------------------------------------------------------

    fn help(&self) -> Vec<Line> {
        [
            "commands:",
            "  ping                        liveness probe",
            "  snapshot                    tree snapshot summary",
            "  get role <Role> [name]      resolve a locator (strict mode)",
            "  get text|label|value <text> resolve a locator (strict mode)",
            "  get testid <id>             resolve a locator (strict mode)",
            "  expect <get-args…>          resolve + assert (the runner wording)",
            "  click | dblclick | hover | focus | blur | check | uncheck",
            "  fill <text> | setvalue <text>",
            "  press <key> [modifiers…]    e.g. `press Enter`, `press a Ctrl`",
            "  scroll <dx> <dy>",
            "  clock install | ff <ms> | runfor <ms> | pause | resume | set <ms> | state",
            "  mock install <id> [response] | clear <id> | call <id> [scope]",
            "  mock reset <scope>",
            "  window list                 the session's window targets",
            "  window use <handle>         aim this connection at one target",
            "  codegen start | stop        record actions as kit code (§13)",
            "  video start [fps] [dir] | video stop",
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
