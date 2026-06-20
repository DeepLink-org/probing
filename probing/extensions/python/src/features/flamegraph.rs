//! Interactive HTML visualizations without inferno.
//!
//! - `FlamegraphKind::Classic` — stacked CPU-style flamegraph (pprof).
//! - `FlamegraphKind::TorchModule` — modern module performance explorer for torch.

use html_escape::encode_text;
use serde_json::json;

const CLASSIC_FRAME_HEIGHT: f64 = 18.0;
const CLASSIC_GRAPH_WIDTH: f64 = 1200.0;
const TORCH_FRAME_HEIGHT: f64 = 32.0;
const TORCH_GRAPH_WIDTH: f64 = 1400.0;
const MIN_RENDER_WIDTH: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlamegraphKind {
    Classic,
    TorchModule,
}

#[derive(Debug, Clone)]
pub struct FlamegraphOptions {
    pub title: String,
    pub count_name: String,
    pub kind: FlamegraphKind,
    pub subtitle: String,
    /// Torch explorer metric id (`duration`, `delta_mb`, `peak_mb`).
    pub metric: Option<String>,
}

impl Default for FlamegraphOptions {
    fn default() -> Self {
        Self {
            title: "Flamegraph".to_string(),
            count_name: "samples".to_string(),
            kind: FlamegraphKind::Classic,
            subtitle: String::new(),
            metric: None,
        }
    }
}

#[derive(Debug, Clone)]
struct Node {
    name: String,
    value: u64,
    children: Vec<Node>,
}

#[derive(Debug, Clone)]
struct PlacedFrame {
    id: usize,
    parent: Option<usize>,
    name: String,
    value: u64,
    x: f64,
    y: f64,
    w: f64,
    depth: usize,
}

#[derive(Debug)]
pub struct Flamegraph {
    root: Node,
    total: u64,
}

impl Flamegraph {
    pub fn from_folded_lines(lines: &[String]) -> Option<Self> {
        let mut root = Node {
            name: "all".to_string(),
            value: 0,
            children: Vec::new(),
        };

        for line in lines {
            if let Some((path, value)) = parse_folded_line(line) {
                insert_path(&mut root, &path, value);
            }
        }

        if root.value == 0 {
            return None;
        }

        let total = root.value;
        Some(Self { root, total })
    }

    pub fn render_html(&self, options: &FlamegraphOptions) -> String {
        match options.kind {
            FlamegraphKind::TorchModule => self.render_torch_html(options),
            FlamegraphKind::Classic => self.render_classic_html(options),
        }
    }

    /// Serializable payload for the web UI (`format=json` API).
    pub fn json_payload(&self, options: &FlamegraphOptions) -> String {
        let (width, frame_height, torch) = match options.kind {
            FlamegraphKind::TorchModule => (TORCH_GRAPH_WIDTH, TORCH_FRAME_HEIGHT, true),
            FlamegraphKind::Classic => (CLASSIC_GRAPH_WIDTH, CLASSIC_FRAME_HEIGHT, false),
        };
        let frames = self.layout_frames(frame_height, width);
        let frames_json = frames_to_json(&frames, torch);
        let profile = match options.kind {
            FlamegraphKind::TorchModule if options.count_name == "samples" => "cpu-stack",
            FlamegraphKind::TorchModule => "torch-module",
            FlamegraphKind::Classic => "classic",
        };
        let mut payload = json!({
            "profile": profile,
            "title": options.title,
            "subtitle": options.subtitle,
            "countName": options.count_name,
            "total": self.total,
            "width": width,
            "frameHeight": frame_height,
            "frames": frames_json,
        });
        if let Some(metric) = &options.metric {
            payload["metric"] = json!(metric);
        }
        payload.to_string()
    }

    fn layout_frames(&self, frame_height: f64, graph_width: f64) -> Vec<PlacedFrame> {
        let mut frames = Vec::new();
        let mut next_id = 0usize;
        let mut ctx = LayoutCtx {
            frame_height,
            frames: &mut frames,
            next_id: &mut next_id,
        };
        layout_node(&self.root, None, 0, 0.0, 0.0, graph_width, &mut ctx);
        frames
    }

    fn render_classic_html(&self, options: &FlamegraphOptions) -> String {
        let frames = self.layout_frames(CLASSIC_FRAME_HEIGHT, CLASSIC_GRAPH_WIDTH);
        let frames_json = frames_to_json(&frames, false);

        let payload = json!({
            "title": options.title,
            "countName": options.count_name,
            "total": self.total,
            "width": CLASSIC_GRAPH_WIDTH,
            "frameHeight": CLASSIC_FRAME_HEIGHT,
            "frames": frames_json,
        });

        let payload_str = payload.to_string();
        let title = encode_text(&options.title);

        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>{title}</title>
<style>
html, body {{ margin: 0; padding: 0; background: #f8f9fa; font-family: ui-sans-serif, system-ui, sans-serif; }}
.probing-fg {{ display: flex; flex-direction: column; height: 100vh; box-sizing: border-box; }}
.probing-fg-header {{ padding: 8px 12px; background: #fff; border-bottom: 1px solid #e5e7eb; }}
.probing-fg-header h1 {{ margin: 0; font-size: 14px; font-weight: 600; color: #111827; }}
.probing-fg-hint {{ font-size: 12px; color: #6b7280; }}
.probing-fg-body {{ flex: 1; overflow: auto; background: #fff; }}
#probing-fg-svg {{ display: block; width: 100%; min-height: 200px; }}
.fg-frame {{ cursor: pointer; }}
.fg-frame rect {{ transition: opacity 0.1s; }}
.fg-frame:hover rect {{ opacity: 0.85; }}
#probing-fg-tooltip {{
  position: fixed; display: none; pointer-events: none;
  background: rgba(17,24,39,0.92); color: #fff; font-size: 12px;
  padding: 6px 10px; border-radius: 4px; z-index: 9999;
  max-width: 420px; white-space: pre-wrap;
}}
</style>
</head>
<body>
<div class="probing-fg" id="probing-flamegraph-root">
  <div class="probing-fg-header">
    <h1>{title}</h1>
    <span class="probing-fg-hint">Click a frame to zoom · Esc or click background to reset</span>
  </div>
  <div class="probing-fg-body">
    <svg id="probing-fg-svg" xmlns="http://www.w3.org/2000/svg" role="img">
      <g id="probing-fg-canvas"></g>
    </svg>
  </div>
  <div id="probing-fg-tooltip"></div>
  <script type="application/json" id="probing-fg-data">{payload_str}</script>
  <script>{CLASSIC_FLAMEGRAPH_JS}</script>
</div>
</body>
</html>"#
        )
    }

    fn render_torch_html(&self, options: &FlamegraphOptions) -> String {
        let frames = self.layout_frames(TORCH_FRAME_HEIGHT, TORCH_GRAPH_WIDTH);
        let frames_json = frames_to_json(&frames, true);

        let payload = json!({
            "profile": match options.count_name.as_str() {
                "samples" => "cpu-stack",
                _ => "torch-module",
            },
            "title": options.title,
            "subtitle": options.subtitle,
            "countName": options.count_name,
            "total": self.total,
            "width": TORCH_GRAPH_WIDTH,
            "frameHeight": TORCH_FRAME_HEIGHT,
            "frames": frames_json,
        });

        let payload_str = payload.to_string();
        let title = encode_text(&options.title);
        let subtitle = encode_text(&options.subtitle);

        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>{title}</title>
<style>
:root {{
  --bg: #0b0f14;
  --surface: #121820;
  --surface-2: #1a222d;
  --border: #2a3544;
  --text: #e8edf4;
  --muted: #8b9bb4;
  --accent: #5b9cf5;
  --forward: #3b82f6;
  --step: #f59e0b;
  --backward: #a855f7;
}}
* {{ box-sizing: border-box; }}
html, body {{ margin: 0; height: 100%; background: var(--bg); color: var(--text);
  font-family: "Inter", ui-sans-serif, system-ui, sans-serif; }}
.probing-torch-module {{ display: flex; flex-direction: column; height: 100vh; }}
.torch-header {{ padding: 14px 18px 10px; border-bottom: 1px solid var(--border);
  background: linear-gradient(180deg, var(--surface) 0%, var(--bg) 100%); }}
.torch-header h1 {{ margin: 0; font-size: 15px; font-weight: 600; letter-spacing: -0.02em; }}
.torch-subtitle {{ margin: 4px 0 0; font-size: 12px; color: var(--muted); }}
.torch-toolbar {{ display: flex; flex-wrap: wrap; gap: 10px; align-items: center;
  padding: 10px 18px; background: var(--surface); border-bottom: 1px solid var(--border); }}
.torch-search {{ flex: 1; min-width: 180px; max-width: 320px; padding: 7px 12px;
  border-radius: 8px; border: 1px solid var(--border); background: var(--surface-2);
  color: var(--text); font-size: 13px; }}
.torch-search:focus {{ outline: none; border-color: var(--accent); box-shadow: 0 0 0 2px rgba(91,156,245,0.25); }}
.torch-phases {{ display: flex; gap: 6px; }}
.torch-phase-btn {{ padding: 6px 12px; border-radius: 999px; border: 1px solid var(--border);
  background: transparent; color: var(--muted); font-size: 12px; cursor: pointer; transition: all 0.15s; }}
.torch-phase-btn.active {{ color: var(--text); border-color: transparent; }}
.torch-phase-btn[data-phase="all"].active {{ background: var(--surface-2); }}
.torch-phase-btn[data-phase="forward"].active {{ background: rgba(59,130,246,0.2); color: #93c5fd; }}
.torch-phase-btn[data-phase="step"].active {{ background: rgba(245,158,11,0.2); color: #fcd34d; }}
.torch-phase-btn[data-phase="backward"].active {{ background: rgba(168,85,247,0.2); color: #d8b4fe; }}
.torch-stats {{ display: flex; gap: 8px; font-size: 11px; color: var(--muted); }}
.torch-stat {{ padding: 4px 10px; border-radius: 6px; background: var(--surface-2); border: 1px solid var(--border); }}
.torch-stat strong {{ color: var(--text); font-weight: 600; }}
.torch-crumbs {{ padding: 6px 18px; font-size: 12px; color: var(--muted);
  border-bottom: 1px solid var(--border); background: var(--bg); }}
.torch-crumbs button {{ background: none; border: none; color: var(--accent); cursor: pointer;
  font-size: 12px; padding: 0; }}
.torch-crumbs button:hover {{ text-decoration: underline; }}
.torch-body {{ flex: 1; overflow: auto; padding: 8px 12px 16px; }}
#probing-torch-svg {{ display: block; width: 100%; min-height: 240px; }}
.torch-frame {{ cursor: pointer; }}
.torch-frame rect {{ transition: filter 0.12s, opacity 0.12s; }}
.torch-frame:hover rect {{ filter: brightness(1.12); }}
.torch-frame.dimmed rect {{ opacity: 0.22; }}
.torch-frame.match rect {{ filter: brightness(1.2); stroke: #fff; stroke-width: 1; }}
#probing-torch-tooltip {{
  position: fixed; display: none; pointer-events: none; z-index: 1000;
  background: rgba(15,20,28,0.96); border: 1px solid var(--border); border-radius: 10px;
  padding: 10px 12px; font-size: 12px; line-height: 1.45; max-width: 360px;
  box-shadow: 0 12px 40px rgba(0,0,0,0.45);
}}
#probing-torch-tooltip .tt-title {{ font-weight: 600; color: var(--text); margin-bottom: 4px; }}
#probing-torch-tooltip .tt-row {{ color: var(--muted); }}
#probing-torch-tooltip .tt-row span {{ color: var(--text); }}
</style>
</head>
<body>
<div class="probing-torch-module" id="probing-torch-module-root">
  <header class="torch-header">
    <h1>{title}</h1>
    <p class="torch-subtitle">{subtitle}</p>
  </header>
  <div class="torch-toolbar">
    <input class="torch-search" id="torch-search" type="search" placeholder="Filter modules…" autocomplete="off"/>
    <div class="torch-phases" id="torch-phases"></div>
    <div class="torch-stats" id="torch-stats"></div>
  </div>
  <div class="torch-crumbs" id="torch-crumbs"></div>
  <div class="torch-body">
    <svg id="probing-torch-svg" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Module performance">
      <g id="probing-torch-canvas"></g>
    </svg>
  </div>
  <div id="probing-torch-tooltip"></div>
  <script type="application/json" id="probing-torch-data">{payload_str}</script>
  <script>{TORCH_MODULE_JS}</script>
</div>
</body>
</html>"#
        )
    }
}

fn frames_to_json(frames: &[PlacedFrame], torch: bool) -> Vec<serde_json::Value> {
    frames
        .iter()
        .map(|f| {
            let mut obj = json!({
                "id": f.id,
                "parent": f.parent,
                "name": f.name,
                "value": f.value,
                "x": f.x,
                "y": f.y,
                "w": f.w,
                "d": f.depth,
            });
            if torch {
                let phase = phase_for_frame(f, frames);
                let module_path = module_path_for(f, frames);
                obj["phase"] = json!(phase);
                obj["modulePath"] = json!(module_path);
            }
            obj
        })
        .collect()
}

fn phase_for_frame(frame: &PlacedFrame, frames: &[PlacedFrame]) -> String {
    let mut cur = Some(frame.id);
    while let Some(id) = cur {
        let f = &frames[id];
        if f.depth == 1 {
            return f.name.clone();
        }
        cur = f.parent;
    }
    "other".to_string()
}

fn module_path_for(frame: &PlacedFrame, frames: &[PlacedFrame]) -> String {
    let mut parts = Vec::new();
    let mut cur = Some(frame.id);
    while let Some(id) = cur {
        let f = &frames[id];
        if f.depth >= 2 {
            parts.push(f.name.clone());
        }
        cur = f.parent;
    }
    parts.reverse();
    parts.join(".")
}

const CLASSIC_FLAMEGRAPH_JS: &str = r##"
(function () {
  const data = JSON.parse(document.getElementById('probing-fg-data').textContent);
  const svg = document.getElementById('probing-fg-svg');
  const canvas = document.getElementById('probing-fg-canvas');
  const tooltip = document.getElementById('probing-fg-tooltip');
  const fh = data.frameHeight || 18;
  const frames = data.frames || [];
  if (!frames.length) return;

  const byId = Object.fromEntries(frames.map((f) => [f.id, f]));
  const children = {};
  frames.forEach((f) => {
    if (f.parent != null) {
      if (!children[f.parent]) children[f.parent] = [];
      children[f.parent].push(f.id);
    }
  });

  function descendants(id) {
    const out = new Set([id]);
    (children[id] || []).forEach((c) => descendants(c).forEach((x) => out.add(x)));
    return out;
  }

  let zoomId = frames[0].id;

  function color(name, depth) {
    let h = 0;
    for (let i = 0; i < name.length; i++) h = (h * 37 + name.charCodeAt(i)) >>> 0;
    const r = (205 + (h % 55)) % 256;
    const g = (40 + ((h >> 8) % 120)) % 256;
    const b = (30 + ((h >> 16) % 80) + depth * 4) % 256;
    return `rgb(${r},${g},${b})`;
  }

  function showTip(f) {
    const pct = data.total ? ((100 * f.value) / data.total).toFixed(2) : "0";
    tooltip.textContent = `${f.name}\n${f.value} ${data.countName} (${pct}%)`;
    tooltip.style.display = "block";
  }

  function hideTip() { tooltip.style.display = "none"; }

  document.addEventListener("mousemove", (e) => {
    if (tooltip.style.display === "block") {
      tooltip.style.left = `${e.clientX + 12}px`;
      tooltip.style.top = `${e.clientY + 12}px`;
    }
  });

  function render() {
    const root = byId[zoomId];
    if (!root) return;
    const zoomSet = descendants(zoomId);
    const visible = frames.filter((f) => zoomSet.has(f.id));
    const maxD = Math.max(...visible.map((f) => f.d));
    const H = (maxD + 1) * fh + 2;
    const W = data.width;
    svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
    canvas.replaceChildren();

    visible.forEach((f) => {
      const rx = ((f.x - root.x) / root.w) * W;
      const rw = (f.w / root.w) * W;
      if (rw < 0.5) return;

      const g = document.createElementNS("http://www.w3.org/2000/svg", "g");
      g.setAttribute("class", "fg-frame");

      const rect = document.createElementNS("http://www.w3.org/2000/svg", "rect");
      rect.setAttribute("x", String(rx));
      rect.setAttribute("y", String(f.d * fh));
      rect.setAttribute("width", String(rw));
      rect.setAttribute("height", String(fh - 1));
      rect.setAttribute("fill", color(f.name, f.d));
      rect.setAttribute("stroke", "#1f2937");
      rect.setAttribute("stroke-width", "0.25");
      g.appendChild(rect);

      if (rw > 36) {
        const text = document.createElementNS("http://www.w3.org/2000/svg", "text");
        text.setAttribute("x", String(rx + 3));
        text.setAttribute("y", String(f.d * fh + fh * 0.72));
        text.setAttribute("fill", "#111827");
        text.setAttribute("font-size", "11");
        text.textContent = f.name.length * 7 > rw - 6
          ? f.name.slice(0, Math.floor((rw - 6) / 7)) + "…"
          : f.name;
        g.appendChild(text);
      }

      g.addEventListener("mouseenter", () => showTip(f));
      g.addEventListener("mouseleave", hideTip);
      g.addEventListener("click", (e) => { e.stopPropagation(); zoomId = f.id; render(); });
      canvas.appendChild(g);
    });
  }

  svg.addEventListener("click", () => { zoomId = frames[0].id; render(); });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") { zoomId = frames[0].id; render(); }
  });
  render();
})();
"##;

const TORCH_MODULE_JS: &str = r##"
(function () {
  const data = JSON.parse(document.getElementById('probing-torch-data').textContent);
  const svg = document.getElementById('probing-torch-svg');
  const canvas = document.getElementById('probing-torch-canvas');
  const tooltip = document.getElementById('probing-torch-tooltip');
  const searchEl = document.getElementById('torch-search');
  const phasesEl = document.getElementById('torch-phases');
  const statsEl = document.getElementById('torch-stats');
  const crumbsEl = document.getElementById('torch-crumbs');
  const fh = data.frameHeight || 32;
  const frames = data.frames || [];
  if (!frames.length) return;

  const byId = Object.fromEntries(frames.map((f) => [f.id, f]));
  const children = {};
  frames.forEach((f) => {
    if (f.parent != null) {
      if (!children[f.parent]) children[f.parent] = [];
      children[f.parent].push(f.id);
    }
  });

  function descendants(id) {
    const out = new Set([id]);
    (children[id] || []).forEach((c) => descendants(c).forEach((x) => out.add(x)));
    return out;
  }

  function ancestors(id) {
    const out = [];
    let cur = byId[id];
    while (cur) {
      out.push(cur);
      cur = cur.parent != null ? byId[cur.parent] : null;
    }
    return out.reverse();
  }

  const phases = ["all"];
  frames.forEach((f) => {
    if (f.d === 1 && f.name !== "all" && !phases.includes(f.name)) phases.push(f.name);
  });

  let zoomId = frames[0].id;
  let phaseFilter = "all";
  let searchQuery = "";
  const zoomStack = [frames[0].id];

  function formatDuration(ns) {
    if (ns >= 1e9) return (ns / 1e9).toFixed(2) + " s";
    if (ns >= 1e6) return (ns / 1e6).toFixed(2) + " ms";
    if (ns >= 1e3) return (ns / 1e3).toFixed(1) + " µs";
    return ns + " ns";
  }

  function phaseColor(phase, depth) {
    const base = {
      forward: [59, 130, 246],
      step: [245, 158, 11],
      backward: [168, 85, 247],
    }[phase] || [100, 116, 139];
    const fade = Math.min(depth * 6, 40);
    return `rgb(${base[0] + fade}, ${base[1] + fade}, ${base[2] + fade})`;
  }

  function labelFor(f) {
    if (f.d >= 2 && f.modulePath) return f.modulePath;
    if (f.d === 1) return f.name === "forward" ? "Forward pass" : f.name === "step" ? "Optimizer step" : f.name;
    return f.name;
  }

  function matchesSearch(f) {
    if (!searchQuery) return true;
    const q = searchQuery.toLowerCase();
    return (f.name && f.name.toLowerCase().includes(q))
      || (f.modulePath && f.modulePath.toLowerCase().includes(q));
  }

  function renderPhaseButtons() {
    phasesEl.replaceChildren();
    phases.forEach((p) => {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "torch-phase-btn" + (phaseFilter === p ? " active" : "");
      btn.dataset.phase = p;
      btn.textContent = p === "all" ? "All phases" : p === "forward" ? "Forward" : p === "step" ? "Optimizer" : p;
      btn.addEventListener("click", () => {
        phaseFilter = p;
        if (p !== "all") {
          const phaseNode = frames.find((f) => f.d === 1 && f.name === p);
          if (phaseNode) {
            zoomId = phaseNode.id;
            zoomStack.length = 0;
            zoomStack.push(frames[0].id, phaseNode.id);
          }
        } else {
          zoomId = frames[0].id;
          zoomStack.length = 0;
          zoomStack.push(frames[0].id);
        }
        renderPhaseButtons();
        render();
      });
      phasesEl.appendChild(btn);
    });
  }

  function renderStats(root, visible) {
    const viewTotal = visible.reduce((s, f) => s + (f.d === root.d ? 0 : 0), 0);
    let leafSum = 0;
    visible.forEach((f) => {
      if (!children[f.id] || children[f.id].length === 0) leafSum += f.value;
    });
    const scope = root.value;
    const pct = data.total ? ((100 * scope) / data.total).toFixed(1) : "0";
    statsEl.innerHTML = `
      <span class="torch-stat">View <strong>${formatDuration(scope)}</strong></span>
      <span class="torch-stat">Share <strong>${pct}%</strong></span>
      <span class="torch-stat">Modules <strong>${visible.filter((f) => f.d >= 2).length}</strong></span>
    `;
  }

  function renderCrumbs() {
    const chain = ancestors(zoomId);
    crumbsEl.replaceChildren();
    chain.forEach((f, i) => {
      if (i > 0) crumbsEl.append(" › ");
      const btn = document.createElement("button");
      btn.type = "button";
      btn.textContent = labelFor(f);
      btn.addEventListener("click", () => {
        zoomId = f.id;
        zoomStack.length = 0;
        chain.slice(0, i + 1).forEach((x) => zoomStack.push(x.id));
        render();
      });
      crumbsEl.appendChild(btn);
    });
  }

  function showTip(f, root) {
    const pctTotal = data.total ? ((100 * f.value) / data.total).toFixed(2) : "0";
    const pctView = root.value ? ((100 * f.value) / root.value).toFixed(2) : "0";
    tooltip.innerHTML = `
      <div class="tt-title">${labelFor(f)}</div>
      <div class="tt-row">Duration: <span>${formatDuration(f.value)}</span></div>
      <div class="tt-row">Of view: <span>${pctView}%</span> · Of total: <span>${pctTotal}%</span></div>
      ${f.modulePath ? `<div class="tt-row">Module: <span>${f.modulePath}</span></div>` : ""}
      <div class="tt-row">Phase: <span>${f.phase || "—"}</span></div>
    `;
    tooltip.style.display = "block";
  }

  function hideTip() { tooltip.style.display = "none"; }

  document.addEventListener("mousemove", (e) => {
    if (tooltip.style.display === "block") {
      tooltip.style.left = `${Math.min(e.clientX + 14, window.innerWidth - 280)}px`;
      tooltip.style.top = `${e.clientY + 14}px`;
    }
  });

  function render() {
    const root = byId[zoomId];
    if (!root) return;
    const zoomSet = descendants(zoomId);
    let visible = frames.filter((f) => zoomSet.has(f.id));
    if (phaseFilter !== "all") {
      visible = visible.filter((f) => f.phase === phaseFilter || f.d <= 1);
    }
    const maxD = Math.max(...visible.map((f) => f.d), 0);
    const H = (maxD + 1) * fh + 12;
    const W = data.width;
    svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
    canvas.replaceChildren();
    renderStats(root, visible);
    renderCrumbs();

    const anySearch = searchQuery.length > 0;

    visible.forEach((f) => {
      if (f.d === 0) return;
      const rx = ((f.x - root.x) / root.w) * W;
      const rw = (f.w / root.w) * W;
      if (rw < 1) return;

      const g = document.createElementNS("http://www.w3.org/2000/svg", "g");
      g.setAttribute("class", "torch-frame");
      const matched = matchesSearch(f);
      if (anySearch && !matched) g.classList.add("dimmed");
      if (anySearch && matched) g.classList.add("match");

      const rect = document.createElementNS("http://www.w3.org/2000/svg", "rect");
      const pad = 2;
      rect.setAttribute("x", String(rx + pad));
      rect.setAttribute("y", String(f.d * fh + pad));
      rect.setAttribute("width", String(Math.max(0, rw - pad * 2)));
      rect.setAttribute("height", String(fh - pad * 2 - 2));
      rect.setAttribute("rx", "6");
      rect.setAttribute("fill", phaseColor(f.phase || "other", f.d));
      rect.setAttribute("stroke", "rgba(255,255,255,0.08)");
      rect.setAttribute("stroke-width", "1");
      g.appendChild(rect);

      const label = f.d >= 2 ? f.name : labelFor(f);
      if (rw > 48 && f.d >= 1) {
        const text = document.createElementNS("http://www.w3.org/2000/svg", "text");
        text.setAttribute("x", String(rx + 10));
        text.setAttribute("y", String(f.d * fh + fh * 0.62));
        text.setAttribute("fill", "#f8fafc");
        text.setAttribute("font-size", "12");
        text.setAttribute("font-weight", f.d === 1 ? "600" : "500");
        const maxChars = Math.floor((rw - 16) / 7);
        text.textContent = label.length > maxChars ? label.slice(0, maxChars) + "…" : label;
        g.appendChild(text);
      }

      g.addEventListener("mouseenter", () => showTip(f, root));
      g.addEventListener("mouseleave", hideTip);
      g.addEventListener("click", (e) => {
        e.stopPropagation();
        zoomId = f.id;
        zoomStack.push(f.id);
        render();
      });
      canvas.appendChild(g);
    });
  }

  searchEl.addEventListener("input", () => {
    searchQuery = searchEl.value.trim();
    render();
  });

  svg.addEventListener("click", () => {
    zoomId = frames[0].id;
    zoomStack.length = 0;
    zoomStack.push(frames[0].id);
    phaseFilter = "all";
    renderPhaseButtons();
    render();
  });

  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      if (zoomStack.length > 1) {
        zoomStack.pop();
        zoomId = zoomStack[zoomStack.length - 1];
      } else {
        zoomId = frames[0].id;
        phaseFilter = "all";
        renderPhaseButtons();
      }
      render();
    }
  });

  renderPhaseButtons();
  render();
})();
"##;

pub fn empty_html(message: &str) -> String {
    empty_html_styled(message, false)
}

pub fn empty_torch_html(message: &str) -> String {
    empty_html_styled(message, true)
}

fn empty_html_styled(message: &str, torch: bool) -> String {
    let msg = encode_text(message);
    if torch {
        format!(
            r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"/><title>Module performance</title>
<style>
body{{margin:0;height:100vh;display:flex;align-items:center;justify-content:center;
background:#0b0f14;color:#8b9bb4;font-family:ui-sans-serif,system-ui,sans-serif;font-size:14px}}
</style></head><body><p>{msg}</p></body></html>"#
        )
    } else {
        format!(
            r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"/><title>Flamegraph</title>
<style>body{{font-family:system-ui,sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;background:#f5f5f5;color:#666}}</style>
</head><body><p>{msg}</p></body></html>"#
        )
    }
}

fn parse_folded_line(line: &str) -> Option<(Vec<String>, u64)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let (stack, count_str) = line.rsplit_once(' ')?;
    let count = count_str.parse::<u64>().ok()?;
    if count == 0 {
        return None;
    }
    let frames: Vec<String> = stack
        .split(';')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect();
    if frames.is_empty() {
        return None;
    }
    Some((frames, count))
}

fn insert_path(node: &mut Node, path: &[String], value: u64) {
    node.value += value;
    if path.is_empty() {
        return;
    }
    let head = &path[0];
    if let Some(child) = node.children.iter_mut().find(|c| c.name == *head) {
        insert_path(child, &path[1..], value);
    } else {
        let mut child = Node {
            name: head.clone(),
            value: 0,
            children: Vec::new(),
        };
        insert_path(&mut child, &path[1..], value);
        node.children.push(child);
    }
}

struct LayoutCtx<'a> {
    frame_height: f64,
    frames: &'a mut Vec<PlacedFrame>,
    next_id: &'a mut usize,
}

fn layout_node(
    node: &Node,
    parent_id: Option<usize>,
    depth: usize,
    x: f64,
    y: f64,
    width: f64,
    ctx: &mut LayoutCtx<'_>,
) -> usize {
    let id = *ctx.next_id;
    *ctx.next_id += 1;

    ctx.frames.push(PlacedFrame {
        id,
        parent: parent_id,
        name: node.name.clone(),
        value: node.value,
        x,
        y,
        w: width,
        depth,
    });

    if node.value == 0 || width < MIN_RENDER_WIDTH {
        return id;
    }

    let mut sorted: Vec<&Node> = node.children.iter().collect();
    sorted.sort_by(|a, b| b.value.cmp(&a.value).then_with(|| a.name.cmp(&b.name)));

    let scale = width / node.value as f64;
    let mut child_x = x;
    for child in sorted {
        let child_w = child.value as f64 * scale;
        if child_w >= MIN_RENDER_WIDTH {
            layout_node(
                child,
                Some(id),
                depth + 1,
                child_x,
                y + ctx.frame_height,
                child_w,
                ctx,
            );
            child_x += child_w;
        }
    }

    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_folded_line_splits_stack_and_count() {
        let (path, n) = parse_folded_line("forward;model;conv 5000000").unwrap();
        assert_eq!(path, ["forward", "model", "conv"]);
        assert_eq!(n, 5_000_000);
    }

    #[test]
    fn render_classic_html_includes_script() {
        let fg = Flamegraph::from_folded_lines(&[
            "forward;model 10000000".to_string(),
            "forward;model;conv 3000000".to_string(),
        ])
        .unwrap();
        let html = fg.render_html(&FlamegraphOptions::default());
        assert!(html.contains("probing-fg-data"));
        assert!(html.contains("probing-flamegraph-root"));
    }

    #[test]
    fn render_torch_module_ui() {
        let fg = Flamegraph::from_folded_lines(&[
            "forward;model 10000000".to_string(),
            "step;Adam 2000000".to_string(),
        ])
        .unwrap();
        let html = fg.render_html(&FlamegraphOptions {
            title: "Modules".to_string(),
            count_name: "ns".to_string(),
            kind: FlamegraphKind::TorchModule,
            subtitle: "Test subtitle".to_string(),
            metric: None,
        });
        assert!(html.contains("probing-torch-module"));
        assert!(html.contains("torch-search"));
        assert!(html.contains("\"phase\":\"forward\""));
        assert!(html.contains("Test subtitle"));
    }

    #[test]
    fn json_payload_roundtrip_fields() {
        let fg = Flamegraph::from_folded_lines(&["forward;model 10".to_string()]).unwrap();
        let json = fg.json_payload(&FlamegraphOptions {
            title: "T".to_string(),
            count_name: "ns".to_string(),
            kind: FlamegraphKind::TorchModule,
            subtitle: "S".to_string(),
            metric: Some("duration".to_string()),
        });
        assert!(json.contains("\"profile\":\"torch-module\""));
        assert!(json.contains("\"subtitle\":\"S\""));
        assert!(json.contains("\"phase\":\"forward\""));
    }
}
