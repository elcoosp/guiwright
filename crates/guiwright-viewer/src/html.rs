//! The self-contained `trace.html` emitter (§12 "self-contained
//! trace.html: timeline scrubber (frames), tree inspector at any point,
//! input replay overlay, log view, protocol calls").
//!
//! M2 shape: the scrubber + tree inspector + event/log views + clock
//! ledger in **one HTML file** — the timeline payload rides an embedded
//! JSON blob, the UI is vanilla JS, there are no external assets and no
//! server. The WASM viewer (§12's full vision) consumes the same
//! timeline model; the archive reader is the shared substrate.

use crate::archive::TraceArchive;
use crate::live::LiveTimeline;
use crate::timeline::Timeline;

/// One embedded snapshot payload row.
#[derive(serde::Serialize)]
struct NodeRow<'a> {
    id: u64,
    parent: Option<u64>,
    role: &'a str,
    label: Option<&'a str>,
    value: Option<&'a str>,
    bounds: Option<[f64; 4]>,
}

/// One embedded tick row.
#[derive(serde::Serialize)]
struct TickRow<'a> {
    frame: u64,
    hash: Option<u64>,
    clock: Option<&'a str>,
    png: bool,
    events: Vec<EventRow<'a>>,
}

#[derive(serde::Serialize)]
struct EventRow<'a> {
    seq: u64,
    kind: &'a str,
    detail: &'a str,
}

#[derive(serde::Serialize)]
struct Payload<'a> {
    test: &'a str,
    adapter: &'a str,
    schema: &'a str,
    ticks: Vec<TickRow<'a>>,
    snapshots: Vec<SnapshotRow<'a>>,
    logs: Vec<&'a str>,
}

#[derive(serde::Serialize)]
struct SnapshotRow<'a> {
    frame: u64,
    nodes: Vec<NodeRow<'a>>,
}

/// Render the self-contained viewer HTML for an archive + timeline.
pub fn render_html(archive: &TraceArchive, timeline: &Timeline) -> String {
    let payload = build_payload(
        &archive.meta.test,
        &archive.meta.adapter.name,
        &archive.meta.schema,
        timeline,
        &archive.logs,
    );
    let json = serde_json::to_string(&payload).unwrap_or_else(|_| "{\"ticks\":[]}".into());
    embed_payload(&json)
}

/// Render the self-contained viewer HTML for a **live capture** (§12's
/// follow-on, M23): the same scrubber over everything a
/// [`LiveTimeline`](crate::live::LiveTimeline) has accumulated — one
/// payload builder, so live and archive emit the byte-identical template.
/// The schema stamp reads `gwtrace/v1 (live)`: an honest marker that the
/// payload is a capture point of a running session, not a finished
/// archive.
pub fn render_live_html(live: &LiveTimeline) -> String {
    let timeline = live.timeline();
    let payload = build_payload(
        live.test(),
        live.adapter().unwrap_or("(unknown)"),
        "gwtrace/v1 (live)",
        &timeline,
        live.logs(),
    );
    let json = serde_json::to_string(&payload).unwrap_or_else(|_| "{\"ticks\":[]}".into());
    embed_payload(&json)
}

/// Embed the payload JSON into the template — angle brackets are escaped
/// so the script data can never close its own tag.
fn embed_payload(json: &str) -> String {
    let json = json.replace('<', "\\u003c").replace('>', "\\u003e");
    html_template(&json)
}

/// The emitter's single payload builder (§12): archive and live captures
/// differ only in where the header strings and logs come from — the
/// scrubber payload itself is the timeline's, verbatim.
fn build_payload<'a>(
    test: &'a str,
    adapter: &'a str,
    schema: &'a str,
    timeline: &'a Timeline,
    logs: &'a [String],
) -> Payload<'a> {
    let ticks = timeline
        .ticks
        .iter()
        .map(|tick| TickRow {
            frame: tick.frame_seq,
            hash: tick.hash,
            clock: tick.clock_command.as_deref(),
            png: tick.has_png,
            events: tick
                .events
                .iter()
                .map(|ev| EventRow {
                    seq: ev.seq,
                    kind: ev.kind.as_str(),
                    detail: &ev.detail,
                })
                .collect(),
        })
        .collect();

    let snapshots = timeline
        .snapshot_payloads()
        .iter()
        .map(|(frame, nodes)| SnapshotRow {
            frame: *frame,
            nodes: nodes
                .iter()
                .map(|n| NodeRow {
                    id: n.id,
                    parent: n.parent,
                    role: n.role.as_str(),
                    label: n.label.as_deref(),
                    value: n.value.as_deref(),
                    bounds: n.bounds,
                })
                .collect(),
        })
        .collect();

    let logs = logs.iter().map(String::as_str).collect();

    Payload {
        test,
        adapter,
        schema,
        ticks,
        snapshots,
        logs,
    }
}

const HTML_HEAD: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Guiwright trace viewer</title>
<style>
  :root { color-scheme: dark; --bg: #14161a; --panel: #1c1f24; --line: #2b3038;
          --fg: #dfe3e8; --dim: #8b939e; --accent: #4f9cf9; --warn: #f9a54f; }
  * { box-sizing: border-box; }
  body { margin: 0; font: 13px/1.45 ui-monospace, "Cascadia Mono", Menlo, monospace;
         background: var(--bg); color: var(--fg); display: flex; flex-direction: column;
         height: 100vh; }
  header { padding: 10px 14px; border-bottom: 1px solid var(--line); display: flex;
           gap: 12px; align-items: baseline; }
  header h1 { font-size: 14px; margin: 0; }
  header .meta { color: var(--dim); }
  #scrubber-wrap { padding: 8px 14px; border-bottom: 1px solid var(--line);
                   background: var(--panel); }
  #scrubber { width: 100%; accent-color: var(--accent); }
  #tick-info { color: var(--dim); margin-top: 4px; min-height: 1.2em; }
  main { flex: 1; display: grid; grid-template-columns: 1.2fr 1fr; min-height: 0; }
  section { border-right: 1px solid var(--line); overflow: auto; padding: 10px 14px; }
  section:last-child { border-right: none; }
  h2 { font-size: 12px; text-transform: uppercase; letter-spacing: .08em;
       color: var(--dim); margin: 0 0 8px; }
  .tree ul { list-style: none; margin: 0; padding-left: 18px; }
  .tree > ul { padding-left: 0; }
  .node { padding: 1px 4px; border-radius: 3px; cursor: pointer; white-space: nowrap; }
  .node:hover { background: #262b33; }
  .node.removed { color: var(--warn); text-decoration: line-through; }
  .node.focused { outline: 1px solid var(--accent); }
  .role { color: var(--accent); }
  .label { color: var(--dim); }
  .pending { color: var(--warn); }
  .ev { padding: 2px 0; border-bottom: 1px dashed var(--line); }
  .ev .kind { color: var(--accent); text-transform: uppercase; font-size: 11px;
              margin-right: 6px; }
  .ev.clock .kind { color: var(--warn); }
  .log { color: var(--dim); white-space: pre-wrap; }
  .framebox { margin-top: 8px; }
  .framebox img { max-width: 100%; border: 1px solid var(--line); }
</style>
</head>
<body>
<header>
  <h1>Guiwright trace</h1>
  <span class="meta" id="meta"></span>
</header>
<div id="scrubber-wrap">
  <input id="scrubber" type="range" min="0" max="0" value="0">
  <div id="tick-info"></div>
</div>
<main>
  <section class="tree"><h2>UI tree</h2><div id="tree"></div>
    <div class="framebox" id="framebox"></div>
  </section>
  <section>
    <h2>Events</h2><div id="events"></div>
    <h2>Logs</h2><div id="logs"></div>
  </section>
</main>
<script>
const PAYLOAD = window.__GW_TRACE__;
"#;

const HTML_TAIL: &str = r#"
document.getElementById("meta").textContent =
  `${PAYLOAD.test} — adapter ${PAYLOAD.adapter} — ${PAYLOAD.schema}`;

const ticks = PAYLOAD.ticks;
const scrubber = document.getElementById("scrubber");
scrubber.max = Math.max(ticks.length - 1, 0);

function snapshotAt(frame) {
  let best = null;
  for (const snap of PAYLOAD.snapshots) {
    if (snap.frame <= frame) best = snap;
    else break;
  }
  return best;
}

function renderTree(frame) {
  const snap = snapshotAt(frame);
  const el = document.getElementById("tree");
  if (!snap) { el.textContent = "(no snapshot yet)"; return; }
  const byParent = new Map();
  for (const n of snap.nodes) {
    const key = n.parent === null ? 0 : n.parent;
    (byParent.get(key) || byParent.set(key, []).get(key)).push(n);
  }
  const removedHere = new Set();
  for (const t of ticks) {
    if (t.frame <= snap.frame || t.frame > frame) continue;
    // ids recorded as removed live only in the timeline payload events
    // for M2; the tree view shows the snapshot payload verbatim.
  }
  let html = "<ul>";
  const walk = (parent) => {
    const kids = byParent.get(parent) || [];
    for (const n of kids) {
      const cls = n.id === undefined ? "" : "";
      html += `<li><div class="node${cls}" data-id="${n.id}">` +
        `<span class="role">${n.role}</span>` +
        (n.label ? ` <span class="label">“${n.label}”</span>` : "") +
        (n.value ? ` <span class="label">= ${n.value}</span>` : "") +
        `</div>`;
      const grandkids = byParent.get(n.id);
      if (grandkids) { html += "<ul>"; walk(n.id); html += "</ul>"; }
      html += "</li>";
    }
  };
  walk(0);
  html += "</ul>";
  el.innerHTML = html;
}

function renderEvents(frame) {
  const ev = document.getElementById("events");
  const upto = ticks.filter(t => t.frame <= frame);
  const recent = upto.slice(-30).reverse();
  ev.innerHTML = recent.map(t =>
    t.events.map(e =>
      `<div class="ev ${e.kind === "clock" ? "clock" : ""}">` +
      `<span class="kind">${e.kind}</span>@f${t.frame} ${escapeHtml(e.detail)}</div>`
    ).join("") || `<div class="ev"><span class="kind">frame</span>@f${t.frame}</div>`
  ).join("");
}

function renderLogs() {
  document.getElementById("logs").innerHTML =
    PAYLOAD.logs.map(l => `<div class="log">${escapeHtml(l)}</div>`).join("")
    || "<div class='log'>(no log lines)</div>";
}

function escapeHtml(s) {
  return s.replace(/[&<>"']/g, c =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

function show(index) {
  const tick = ticks[index];
  if (!tick) return;
  scrubber.value = index;
  const parts = [`frame ${tick.frame}`];
  if (tick.hash !== undefined) parts.push(`hash ${tick.hash.toString(16)}`);
  if (tick.clock) parts.push(`clock ${tick.clock}`);
  if (tick.png) parts.push("png stored");
  document.getElementById("tick-info").textContent = parts.join(" · ");
  renderTree(tick.frame);
  renderEvents(tick.frame);
}

scrubber.addEventListener("input", () => show(Number(scrubber.value)));
renderLogs();
show(ticks.length > 0 ? ticks.length - 1 : 0);
</script>
</body>
</html>
"#;

fn html_template(payload_json: &str) -> String {
    format!("{HTML_HEAD}\nwindow.__GW_TRACE__ = {payload_json};\n{HTML_TAIL}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_embeds_payload_and_is_self_contained() {
        let html = html_template(r#"{"ticks":[{"frame":1}]}"#);
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("window.__GW_TRACE__ = {\"ticks\":[{\"frame\":1}]};"));
        assert!(html.contains("<script>"));
        assert!(html.contains("</html>"));
        // Self-contained: no external asset references.
        assert!(!html.contains("src=\"http"));
        assert!(!html.contains("href=\"http"));
        assert!(!html.contains("import "));
    }

    #[test]
    fn angle_brackets_are_escaped_in_payload() {
        let html = embed_payload(r#"{"logs":["a<b>c"]}"#);
        assert!(
            html.contains("\\u003cb\\u003e"),
            "no raw < inside script data"
        );
        assert!(!html.contains("<b>"), "{html}");
    }

    // --- The live-capture emitter (§12 follow-on, M23) --------------------

    #[test]
    fn a_live_capture_renders_the_same_scrubber() {
        use guiwright_proto::schema::NodeKeyV1;
        use guiwright_proto::schema::SnapshotNodeV1;

        let row = |id: u64, role: &str, name: &str, children: u32| SnapshotNodeV1 {
            key: NodeKeyV1::AccessKit {
                tree: [0u8; 16],
                node: id,
            },
            role: role.into(),
            name: Some(name.into()),
            value: None,
            bounds: Some([0.0, 0.0, 10.0, 10.0]),
            children,
        };

        let mut live = crate::live::LiveTimeline::new("gwviewer-live/0.1");
        live.set_adapter("mock");
        live.record_frame(2, 0x2);
        live.record_event("click", "at (1,1)");
        live.record_log("{\"message\":\"a<b>\"}");
        live.record_snapshot(
            &[row(1, "Window", "W", 1), row(2, "Button", "Save", 0)],
            None,
        );

        let html = render_live_html(&live);
        assert!(html.starts_with("<!doctype html>"), "the same template");
        // The header strings ride the payload JSON; the browser composes
        // the header line (`${PAYLOAD.test} — adapter ${PAYLOAD.adapter}`).
        assert!(
            html.contains(r#""test":"gwviewer-live/0.1""#),
            "the test label"
        );
        assert!(html.contains(r#""adapter":"mock""#), "the adapter name");
        assert!(html.contains("(live)"), "the live schema stamp");
        assert!(html.contains("Save"), "the dump rides the payload");
        assert!(
            html.contains("a\\u003cb\\u003e"),
            "log lines are angle-escaped like the archive path"
        );
        assert!(!html.contains("src=\"http"), "still self-contained");
    }

    #[test]
    fn a_capture_without_an_adapter_names_it_honestly() {
        let live = crate::live::LiveTimeline::new("bare/0.1");
        let html = render_live_html(&live);
        assert!(
            html.contains(r#""adapter":"(unknown)""#),
            "the honest unknown: {html:.200}"
        );
    }
}
