"use strict";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// ---- state -----------------------------------------------------------------

let cfg = { poll_ms: 50, rules: [] };
let status = { connected: false, strip_count: 8, labels: {}, rules: [] };
let saveTimer = null;

const DB_MIN = -60;
const el = (id) => document.getElementById(id);

// ---- helpers ---------------------------------------------------------------

function stripCount() {
  return status.connected && status.strip_count ? status.strip_count : 8;
}

function stripName(i) {
  const label = status.labels && status.labels[i];
  return label && label.trim() ? `${i} · ${label}` : `Strip ${i}`;
}

function dbToPct(db) {
  const v = Math.max(DB_MIN, Math.min(0, db));
  return ((v - DB_MIN) / -DB_MIN) * 100;
}

function fmt(n, digits = 0) {
  return Number(n).toFixed(digits);
}

// find the live status for a given rule index + source strip
function sourceStatus(r, strip) {
  const rs = status.rules[r];
  if (!rs) return null;
  return rs.sources.find((s) => s.strip === strip) || null;
}

// ---- rendering -------------------------------------------------------------

function render() {
  const root = el("rules");
  root.innerHTML = "";
  cfg.rules.forEach((rule, r) => root.appendChild(ruleCard(rule, r)));
}

function ruleCard(rule, r) {
  const card = document.createElement("div");
  card.className = "card";
  card.dataset.rule = r;

  const head = document.createElement("div");
  head.className = "card-head";
  head.innerHTML = `<h2>Rule ${r + 1}</h2>`;
  const right = document.createElement("div");
  right.style.display = "flex";
  right.style.gap = "10px";
  right.style.alignItems = "center";
  right.appendChild(previewButton());
  const dot = document.createElement("span");
  dot.className = "ducking-dot";
  dot.id = `dot-${r}`;
  dot.textContent = "DUCKING";
  right.appendChild(dot);
  if (cfg.rules.length > 1) {
    const rm = document.createElement("button");
    rm.className = "icon-btn";
    rm.textContent = "✕";
    rm.title = "Remove rule";
    rm.onclick = () => { cfg.rules.splice(r, 1); render(); scheduleSave(); };
    right.appendChild(rm);
  }
  head.appendChild(right);
  card.appendChild(head);

  // Sources
  card.appendChild(subhead("Trigger sources — duck when any of these has sound"));
  rule.sources.forEach((src, si) => card.appendChild(sourceRow(rule, r, src, si)));
  const addSrc = document.createElement("button");
  addSrc.className = "link-btn";
  addSrc.textContent = "+ Add source";
  addSrc.onclick = () => {
    rule.sources.push({ strip: firstFreeStrip(rule), threshold_db: -40 });
    render(); scheduleSave();
  };
  card.appendChild(addSrc);

  // Targets
  card.appendChild(subhead("Targets — strips to duck"));
  card.appendChild(targetChips(rule));

  // Amount / timing sliders
  card.appendChild(slider("Duck amount", `${fmt(rule.duck_db, 1)} dB`, rule.duck_db, -40, 0, 0.5,
    (v) => { rule.duck_db = v; }, "How much quieter the target strips get while ducking. More negative = quieter."));
  card.appendChild(slider("Attack", `${fmt(rule.attack_ms)} ms`, rule.attack_ms, 0, 500, 5,
    (v) => { rule.attack_ms = v; }, "How quickly the volume drops once a trigger starts. Lower = snappier."));
  card.appendChild(slider("Release", `${fmt(rule.release_ms)} ms`, rule.release_ms, 0, 2000, 10,
    (v) => { rule.release_ms = v; }, "How slowly the volume rises back once the trigger stops. Higher = gentler."));
  card.appendChild(slider("Hold", `${fmt(rule.hold_ms)} ms`, rule.hold_ms, 0, 3000, 50,
    (v) => { rule.hold_ms = v; }, "Keep ducking this long after the trigger goes quiet, to ride over short gaps."));

  // Low-pass (Color) block
  card.appendChild(colorBlock(rule));

  return card;
}

function subhead(text) {
  const h = document.createElement("div");
  h.className = "subhead";
  h.textContent = text;
  return h;
}

// ---- hold-to-preview -------------------------------------------------------

let previewing = false;

// Force a duck (or stop) so the user can hear their settings without waiting for
// a real trigger. De-duped so the safety net handlers below are cheap.
function setPreview(on) {
  if (on === previewing) return;
  previewing = on;
  document.querySelectorAll(".preview-btn").forEach((b) => b.classList.toggle("active", on));
  invoke("set_preview", { on });
}

function previewButton() {
  const btn = document.createElement("button");
  btn.className = "preview-btn";
  btn.textContent = "Preview";
  btn.title = "Hold to duck now using these settings";
  btn.addEventListener("pointerdown", (e) => { e.preventDefault(); setPreview(true); });
  return btn;
}

function sourceRow(rule, r, src, si) {
  const row = document.createElement("div");
  row.className = "source-row";

  const top = document.createElement("div");
  top.className = "source-top";
  top.appendChild(stripSelect(src.strip, (v) => { src.strip = v; render(); scheduleSave(); }));
  if (rule.sources.length > 1) {
    const rm = document.createElement("button");
    rm.className = "icon-btn";
    rm.textContent = "✕";
    rm.title = "Remove source";
    rm.onclick = () => { rule.sources.splice(si, 1); render(); scheduleSave(); };
    top.appendChild(rm);
  }
  row.appendChild(top);

  // live meter with threshold marker
  const meter = document.createElement("div");
  meter.className = "meter";
  const fill = document.createElement("div");
  fill.className = "meter-fill";
  fill.dataset.meter = `${r}:${src.strip}`;
  const mark = document.createElement("div");
  mark.className = "meter-mark";
  mark.dataset.mark = `${r}:${src.strip}`;
  mark.style.left = dbToPct(src.threshold_db) + "%";
  meter.appendChild(fill);
  meter.appendChild(mark);
  row.appendChild(meter);

  row.appendChild(slider("Threshold", `${fmt(src.threshold_db, 0)} dB`, src.threshold_db, -60, 0, 1,
    (v) => { src.threshold_db = v; mark.style.left = dbToPct(v) + "%"; },
    "Sound above this level on this input starts the duck. The marker on the meter shows where it sits."));

  return row;
}

function stripSelect(value, onChange) {
  const sel = document.createElement("select");
  for (let i = 0; i < stripCount(); i++) {
    const opt = document.createElement("option");
    opt.value = i;
    opt.textContent = stripName(i);
    if (i === value) opt.selected = true;
    sel.appendChild(opt);
  }
  sel.onchange = () => onChange(parseInt(sel.value, 10));
  return sel;
}

function targetChips(rule) {
  const wrap = document.createElement("div");
  wrap.className = "chips";
  const sources = new Set(rule.sources.map((s) => s.strip));
  for (let i = 0; i < stripCount(); i++) {
    const chip = document.createElement("button");
    chip.className = "chip" + (rule.targets.includes(i) ? " sel" : "");
    chip.textContent = stripName(i);
    if (sources.has(i)) chip.disabled = true;
    chip.onclick = () => {
      const idx = rule.targets.indexOf(i);
      if (idx >= 0) rule.targets.splice(idx, 1);
      else rule.targets.push(i);
      rule.targets.sort((a, b) => a - b);
      render(); scheduleSave();
    };
    wrap.appendChild(chip);
  }
  return wrap;
}

function slider(label, valText, value, min, max, step, onInput, hint) {
  const row = document.createElement("div");
  row.className = "slider-row";
  const lab = document.createElement("div");
  lab.className = "slabel";
  const name = document.createElement("span");
  name.textContent = label;
  const val = document.createElement("span");
  val.className = "val";
  val.textContent = valText;
  lab.appendChild(name); lab.appendChild(val);
  const input = document.createElement("input");
  input.type = "range";
  input.min = min; input.max = max; input.step = step; input.value = value;
  const unit = valText.replace(/^[-\d.]+\s*/, "");
  const digits = step < 1 ? 1 : 0;
  input.oninput = () => {
    const v = parseFloat(input.value);
    val.textContent = `${fmt(v, digits)} ${unit}`.trim();
    onInput(v);
  };
  input.onchange = () => scheduleSave();
  row.appendChild(lab); row.appendChild(input);
  if (hint) {
    const h = document.createElement("div");
    h.className = "hint";
    h.textContent = hint;
    row.appendChild(h);
  }
  return row;
}

function colorBlock(rule) {
  const block = document.createElement("div");
  block.className = "color-block" + (rule.color ? "" : " off");

  const toggle = document.createElement("label");
  toggle.className = "switch";
  const cb = document.createElement("input");
  cb.type = "checkbox";
  cb.checked = !!rule.color;
  const track = document.createElement("span");
  track.className = "track";
  const lab = document.createElement("span");
  lab.className = "switch-label";
  lab.textContent = "Low-pass while ducking (Color pad, hardware strips only)";
  toggle.appendChild(cb); toggle.appendChild(track); toggle.appendChild(lab);
  block.appendChild(toggle);

  const controls = document.createElement("div");
  controls.className = "color-controls";
  controls.appendChild(slider("Amount (left = more low-pass)", fmt(rule.color ? rule.color.x : -0.2, 2),
    rule.color ? rule.color.x : -0.2, -0.5, 0.5, 0.05, (v) => { if (rule.color) rule.color.x = v; },
    "Rolls off the highs while ducking, so the audio sits further back. Left = darker."));
  controls.appendChild(slider("Vertical", fmt(rule.color ? rule.color.y : 0, 2),
    rule.color ? rule.color.y : 0, 0, 1, 0.05, (v) => { if (rule.color) rule.color.y = v; },
    "The pad's second axis - a subtle tone shift. Usually left at 0."));
  block.appendChild(controls);

  cb.onchange = () => {
    rule.color = cb.checked ? { x: -0.2, y: 0.0 } : null;
    render(); scheduleSave();
  };
  return block;
}

// ---- strip pickers ---------------------------------------------------------

function firstFreeStrip(rule) {
  const used = new Set([...rule.sources.map((s) => s.strip), ...rule.targets]);
  for (let i = 0; i < stripCount(); i++) if (!used.has(i)) return i;
  return 0;
}

function defaultRule() {
  return { sources: [{ strip: 0, threshold_db: -40 }], targets: [1], duck_db: -8, attack_ms: 60, release_ms: 350, hold_ms: 1000, color: null };
}

// ---- live status -----------------------------------------------------------

function applyStatus(s) {
  status = s;
  const conn = el("conn");
  if (s.connected) {
    conn.textContent = s.kind_name || "Connected";
    conn.className = "pill pill-ok";
  } else {
    conn.textContent = "Waiting for Voicemeeter…";
    conn.className = "pill pill-wait";
  }
  const err = el("error");
  if (s.error) { err.textContent = s.error; err.classList.remove("hidden"); }
  else err.classList.add("hidden");

  el("enabled").checked = !!s.enabled;

  // live meters + ducking dots
  (s.rules || []).forEach((rs, r) => {
    const dot = el(`dot-${r}`);
    if (dot) dot.classList.toggle("on", rs.ducked);
    rs.sources.forEach((src) => {
      const fill = document.querySelector(`.meter-fill[data-meter="${r}:${src.strip}"]`);
      if (fill) {
        fill.style.width = dbToPct(src.db) + "%";
        fill.classList.toggle("over", src.over);
      }
    });
  });
}

// ---- persistence -----------------------------------------------------------

function scheduleSave() {
  const status = el("save-status");
  status.textContent = "Saving…";
  status.className = "save-status";
  clearTimeout(saveTimer);
  saveTimer = setTimeout(save, 400);
}

async function save() {
  try {
    await invoke("save_config", { config: cfg });
    const s = el("save-status");
    s.textContent = "Saved ✓";
    s.className = "save-status ok";
  } catch (e) {
    const s = el("save-status");
    s.textContent = String(e);
    s.className = "save-status";
  }
}

// ---- update banner ---------------------------------------------------------

function showUpdate(info) {
  el("update-text").textContent = `Update ${info.version} available (you have ${info.current}).`;
  el("update-banner").classList.remove("hidden");
}

// ---- init ------------------------------------------------------------------

async function init() {
  cfg = await invoke("get_config");
  if (!cfg.rules || cfg.rules.length === 0) cfg.rules = [defaultRule()];

  const prefs = await invoke("get_prefs");
  el("enabled").checked = prefs.enabled;
  el("autostart").checked = prefs.autostart;

  try { applyStatus(await invoke("get_status")); } catch (_) {}
  render();

  el("enabled").onchange = (e) => invoke("set_enabled", { on: e.target.checked });
  el("autostart").onchange = async (e) => {
    try { await invoke("set_autostart", { on: e.target.checked }); }
    catch (err) { e.target.checked = !e.target.checked; alert(err); }
  };
  el("add-rule").onclick = () => { cfg.rules.push(defaultRule()); render(); scheduleSave(); };
  el("update-install").onclick = () => invoke("install_update");

  // Preview is a dead-man's switch: release on pointer-up anywhere, and whenever
  // the window is hidden to the tray mid-hold (pointerup would never fire).
  window.addEventListener("pointerup", () => setPreview(false));
  window.addEventListener("pointercancel", () => setPreview(false));
  window.addEventListener("blur", () => setPreview(false));
  document.addEventListener("visibilitychange", () => { if (document.hidden) setPreview(false); });

  await listen("engine:status", (ev) => applyStatus(ev.payload));
  await listen("updater:available", (ev) => showUpdate(ev.payload));

  // re-render when the strip count first becomes known so labels populate
  let hadCount = status.connected;
  await listen("engine:status", (ev) => {
    if (ev.payload.connected && !hadCount) { hadCount = true; render(); }
  });
}

window.addEventListener("DOMContentLoaded", init);
