// The island. Ported from drip.html: all behaviour runs through applyHook(),
// which receives agent-neutral events from the Rust side. Claude Code's own
// payload shape stops at the backend adapter (src-tauri/src/agents/).

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./style.css";

type Mode = "hidden" | "peek" | "compact" | "expanded";
type Status = "working" | "approval" | "finished" | "error" | "idle";
type View = "activity" | "approval" | "finished" | "error";
type Choice = "allow" | "always" | "deny" | "ask";

interface Step { id: number; tool: string; target: string; status: "running" | "done" | "failed"; at: number; key?: string }
interface Approval { id: string; session: string; project: string; agent: string; tool: string; target: string; detail: string; canRemember: boolean; deadline: number; total: number }
interface State {
  mode: Mode; view: View; status: Status;
  session: string; project: string; started: number; finishedAt: number | null; lastActive: number;
  approval: Approval | null; queue: Approval[]; error: string | null; steps: Step[];
}
/** Mirrors `AgentEvent` in src-tauri/src/agents/mod.rs. */
interface AgentEvent {
  agent: string;
  session: string;
  kind: "session_start" | "session_end" | "prompt" | "tool_start" | "tool_end" | "tool_failed"
    | "permission" | "notice" | "turn_done" | "turn_failed";
  tool: string | null;
  target: string | null;
  cwd: string | null;
  needsDecision: boolean;
  key?: string;
  detail?: string;
  request?: { id: string; timeoutMs: number; canRemember: boolean };
}

const $ = <T extends HTMLElement = HTMLElement>(s: string) => document.querySelector(s) as T;
const island = $("#island");
const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;

/* ---------- State ---------- */
let uid = 0;
const S = {} as State;
const timers = new Set<number>();
const later = (fn: () => void, ms: number) => {
  const id = window.setTimeout(() => { timers.delete(id); fn(); }, ms);
  timers.add(id);
  return id;
};

function initial() {
  timers.forEach(clearTimeout); timers.clear();
  const now = Date.now();
  Object.assign(S, {
    mode: "hidden", view: "activity", status: "idle",
    session: "", project: "", started: now, finishedAt: null, lastActive: now,
    approval: null, queue: [], error: null, steps: [],
  } satisfies State);
}

/** The prototype's example session, shown in `tauri dev` so every view can be checked. */
function sample() {
  const now = Date.now();
  Object.assign(S, {
    mode: "expanded", view: "activity", status: "working",
    project: "weather-cli", started: now - 42000, lastActive: now,
    steps: [
      { id: ++uid, tool: "Edit", target: "forecast.ts", status: "running", at: now - 4000 },
      { id: ++uid, tool: "Search", target: '"parseHourly" in src', status: "done", at: now - 17000 },
      { id: ++uid, tool: "Read", target: "forecast.ts", status: "done", at: now - 29000 },
      { id: ++uid, tool: "Prompt", target: "Round hourly temps to one decimal", status: "done", at: now - 42000 },
    ],
  });
}

/** How each adapter's agent is named in copy. */
const AGENT_NAME: Record<string, string> = { "claude-code": "Claude" };

/* ---------- Steps ---------- */
// Tool labels and targets arrive ready to show; the backend adapter makes them.
const baseName = (p: unknown) => String(p ?? "").split(/[\\/]/).filter(Boolean).pop() || String(p ?? "");
function pushStep(step: Omit<Step, "id" | "at">) {
  S.steps.unshift({ id: ++uid, at: Date.now(), ...step });
  if (S.steps.length > 20) S.steps.pop();
}
function reveal(m: Mode) { if (S.mode === "hidden" || S.mode === "peek") S.mode = m; }

/* ---------- The state machine ---------- */
function applyHook(e: AgentEvent) {
  S.lastActive = Date.now();
  settledElsewhere(e);
  // The card on screen belongs to one session. Meanwhile other sessions only
  // add steps, and their own permission requests wait in line behind it.
  if (S.approval && e.session !== S.approval.session) {
    if (e.kind === "permission") queueCard(e); else otherSession(e);
    render();
    return;
  }
  // Otherwise the island follows whichever session spoke last.
  if (e.session !== S.session || !S.project) {
    S.session = e.session;
    if (e.cwd) S.project = baseName(e.cwd);
  }
  switch (e.kind) {
    case "session_start":
      S.project = baseName(e.cwd); S.started = Date.now(); S.finishedAt = null;
      S.steps = []; S.status = "idle"; S.view = "activity"; S.error = null;
      reveal("compact"); if (S.mode === "expanded") S.mode = "compact";
      break;
    case "prompt":
      if (S.status === "idle" || S.status === "finished" || S.status === "error") { S.started = Date.now(); S.finishedAt = null; }
      S.status = "working"; S.view = "activity";
      pushStep({ tool: "Prompt", target: e.target ?? "", status: "done" });
      reveal("compact");
      break;
    case "tool_start":
      if (S.status !== "approval") S.status = "working";
      if (S.view === "finished" || S.view === "error") S.view = "activity";
      pushStep({ tool: e.tool ?? "Tool", target: e.target ?? "", status: "running", key: e.key });
      reveal("compact");
      break;
    case "tool_end":
    case "tool_failed": {
      const s = S.steps.find(x => x.key && x.key === e.key) || S.steps.find(x => x.status === "running");
      if (s) s.status = e.kind === "tool_end" ? "done" : "failed";
      break;
    }
    case "permission":
      if (S.approval) queueCard(e); else { const c = card(e); if (c) showCard(c); }
      break;
    case "notice":
      pushStep({ tool: e.tool ?? "Note", target: e.target ?? "", status: "done" });
      break;
    case "turn_done":
      S.steps.forEach(s => { if (s.status === "running") s.status = "done"; });
      S.status = "finished"; S.finishedAt = Date.now(); S.view = "finished"; S.mode = "expanded";
      later(() => { if (S.status === "finished" && S.mode === "expanded" && S.view === "finished") { S.mode = "compact"; render(); } }, 10000);
      break;
    case "turn_failed":
      S.steps.forEach(s => { if (s.status === "running") s.status = "failed"; });
      S.status = "error"; S.error = e.detail || "Request failed"; S.view = "error"; S.mode = "expanded";
      break;
    case "session_end":
      S.status = "idle"; S.mode = "hidden";
      break;
    default:
      return;
  }
  render();
}

/** Steps from a session other than the one whose card is on screen. */
function otherSession(e: AgentEvent) {
  const where = e.cwd ? `${baseName(e.cwd)}: ` : "";
  if (e.kind === "prompt") pushStep({ tool: "Prompt", target: where + (e.target ?? ""), status: "done" });
  if (e.kind === "tool_start") pushStep({ tool: e.tool ?? "Tool", target: where + (e.target ?? ""), status: "running", key: e.key });
  if (e.kind === "notice") pushStep({ tool: e.tool ?? "Note", target: where + (e.target ?? ""), status: "done" });
  if (e.kind === "tool_end" || e.kind === "tool_failed") {
    const s = S.steps.find(x => x.key && x.key === e.key);
    if (s) s.status = e.kind === "tool_end" ? "done" : "failed";
  }
  if (where && (e.kind === "turn_done" || e.kind === "turn_failed" || e.kind === "session_end")) {
    S.steps.forEach(s => { if (s.status === "running" && s.target.startsWith(where)) s.status = e.kind === "turn_failed" ? "failed" : "done"; });
  }
}

/* ---------- Permission cards ---------- */
function card(e: AgentEvent): Approval | null {
  if (!e.needsDecision || !e.request) return null;
  const total = e.request.timeoutMs || 108000;
  return {
    id: e.request.id, session: e.session, project: baseName(e.cwd) || S.project, agent: e.agent,
    tool: e.tool ?? "a tool", target: e.target ?? "", detail: e.detail ?? e.target ?? "",
    canRemember: e.request.canRemember, deadline: Date.now() + total, total,
  };
}

function showCard(a: Approval) {
  S.approval = a; S.session = a.session; S.project = a.project;
  S.status = "approval"; S.view = "approval"; S.mode = "expanded";
  armApproval();
}

/** A second request waits its turn instead of pushing the first one back to its terminal. */
function queueCard(e: AgentEvent) {
  const c = card(e);
  if (c) S.queue.push(c);
}

/** Shows the next waiting card that still has time left. Returns false when there is none. */
function nextCard() {
  S.approval = null;
  S.queue = S.queue.filter(a => a.deadline > Date.now() + 1000);
  const next = S.queue.shift();
  if (next) { showCard(next); return true; }
  S.status = "working"; S.view = "activity";
  return false;
}

/** Hands a request back to the agent's own prompt: the relay prints nothing. */
const release = (a: Approval) => { if (a.id) invoke("decide", { id: a.id, choice: "ask" }).catch(() => {}); };

/**
 * Claude Code shows its own prompt while a hook waits, so a request can be
 * answered in the terminal first. Its session moving on settles the card: the
 * same tool finishing or failing, or the turn or session ending.
 */
function settledElsewhere(e: AgentEvent) {
  const ends = e.kind === "prompt" || e.kind === "turn_done" || e.kind === "turn_failed" || e.kind === "session_end" || e.kind === "session_start";
  const settles = (a: Approval) => a.session === e.session && (ends ||
    ((e.kind === "tool_end" || e.kind === "tool_failed") && e.tool === a.tool && (e.target ?? "") === a.target));
  S.queue = S.queue.filter(a => { if (!settles(a)) return true; release(a); return false; });
  if (S.approval && settles(S.approval)) {
    release(S.approval);
    pushStep({ tool: "Answered", target: "in the terminal", status: "done" });
    nextCard();
  }
}

function decide(choice: Choice, timedOut = false) {
  const a = S.approval; if (!a) return;
  // "ask" hands the question back to the terminal: Rust closes the pipe with nothing written.
  if (a.id) invoke("decide", { id: a.id, choice }).catch(() => {});
  if (choice === "deny") pushStep({ tool: "Denied", target: a.target, status: "failed" });
  if (timedOut) pushStep({ tool: "Asked", target: "in the terminal", status: "done" });
  S.lastActive = Date.now();
  if (!nextCard()) S.mode = "compact";
  render();
}

/* ---------- Rendering ---------- */
const fmt = (ms: number) => { const s = Math.max(0, Math.round(ms / 1000)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`; };
const CHIP: Record<Status, string> = { working: "Working", approval: "Needs you", finished: "Done", error: "Error", idle: "Ready" };

function stepList(el: HTMLElement, steps: Step[]) {
  el.replaceChildren(...steps.map(s => {
    const li = document.createElement("li"); li.dataset.s = s.status;
    const dot = document.createElement("span"); dot.className = "dot";
    const tool = document.createElement("span"); tool.className = "tool"; tool.textContent = s.tool;
    const tgt = document.createElement("span"); tgt.className = "tgt mono"; tgt.textContent = s.target;
    const t = document.createElement("span"); t.className = "t"; t.dataset.at = String(s.at);
    li.append(dot, tool, tgt, t); return li;
  }));
}
function headNow() {
  if (S.status === "approval") return "Waiting for your answer";
  if (S.status === "finished") return `Finished in ${fmt((S.finishedAt ?? Date.now()) - S.started)}`;
  if (S.status === "error") return "Stopped with an error";
  if (S.status === "idle") return S.steps.length ? "Ready" : "Session started";
  const s = S.steps[0]; return s ? `${s.tool} · ${s.target}` : "Working";
}
function render() {
  island.dataset.mode = S.mode;
  island.dataset.status = S.status;
  $("#proj").textContent = S.project || "Claude Code";
  $("#now").textContent = headNow();
  $("#chip").textContent = CHIP[S.status];
  for (const v of ["activity", "approval", "finished", "error"]) $("#v-" + v).hidden = S.view !== v;

  if (S.view === "activity") stepList($("#steps"), S.steps.slice(0, 6));
  if (S.view === "approval" && S.approval) {
    const a = S.approval;
    const who = AGENT_NAME[a.agent] ?? "The agent";
    $("#ap-title").textContent = a.tool === "Run" ? `${who} wants to run a command`
      : (a.tool === "Edit" || a.tool === "Write" || a.tool === "Notebook") ? `${who} wants to change a file`
      : a.tool === "Fetch" ? `${who} wants to open a web page` : `${who} wants to use ${a.tool}`;
    $("#ap-cmd").textContent = a.detail;
    $("#b-always").hidden = !a.canRemember;
    $("#ap-note").hidden = !a.canRemember;
    const more = S.queue.length;
    $("#ap-more").textContent = more ? `${more} more ${more === 1 ? "request" : "requests"} waiting after this one` : "";
    $("#ap-more").hidden = !more;
  }
  if (S.view === "finished") {
    const done = S.steps.filter(s => s.status === "done");
    const files = new Set(done.filter(s => s.tool === "Edit" || s.tool === "Write").map(s => s.target)).size;
    const cmds = done.filter(s => s.tool === "Run").length;
    const failed = S.steps.filter(s => s.status === "failed").length;
    $("#fin-time").textContent = `Done in ${fmt((S.finishedAt || Date.now()) - S.started)}`;
    const steps = S.steps.filter(s => s.tool !== "Prompt").length;
    const stats: [number, string][] = [
      [steps, steps === 1 ? "step" : "steps"],
      [files, files === 1 ? "file changed" : "files changed"],
      [cmds, cmds === 1 ? "command" : "commands"],
    ];
    if (failed) stats.push([failed, "failed"]);
    $("#fin-stats").replaceChildren(...stats.map(([n, l]) => {
      const s = document.createElement("span"); const b = document.createElement("b");
      b.textContent = String(n); s.append(b, " " + l); return s;
    }));
    stepList($("#fin-steps"), S.steps.slice(0, 4));
  }
  if (S.view === "error") $("#err-msg").textContent = S.error;
  tickText();
  syncWindow();
}
function tickText() {
  const now = Date.now();
  if (S.view === "activity" && S.mode === "expanded") {
    $("#elapsed").textContent = S.status === "idle" ? "Waiting for a prompt" : `Working for ${fmt(now - S.started)}`;
    document.querySelectorAll<HTMLElement>("#steps .t").forEach(t => t.textContent = fmt(now - Number(t.dataset.at)) + " ago");
  }
  if (S.approval) {
    const left = S.approval.deadline - now;
    $("#ap-left").textContent = `${fmt(left)} to answer`;
    $("#ap-fill").style.transform = `scaleX(${Math.max(0, left / S.approval.total)})`;
    if (left <= 0) decide("ask", true);
  }
  if (S.mode === "compact" && (S.status === "idle" || S.status === "finished") && now - S.lastActive > 25000) { S.mode = "hidden"; render(); }
}

/* ---------- Window size follows the island ---------- */
// Room around the island for its shadow. The hidden strip is a little wider
// than the bar so the top-centre edge is easy to hit.
const SIDE = 14, BOTTOM = 26, STRIP_W = 160, STRIP_H = 6;
const WIDTH: Record<Exclude<Mode, "hidden">, number> = { peek: 66, compact: 370, expanded: 470 };
let win = { w: 0, h: 0 };
let shrinkTimer = 0;

function finalSize() {
  if (S.mode === "hidden") return { w: STRIP_W, h: STRIP_H };
  const h = S.mode === "peek" ? 48 : S.mode === "compact" ? 54 : Math.min(470, island.scrollHeight + 1);
  return { w: WIDTH[S.mode] + 2 * SIDE, h: h + BOTTOM };
}
function setWindow(w: number, h: number) {
  if (w === win.w && h === win.h) return;
  win = { w, h };
  invoke("set_island_size", { width: w, height: h }).catch(() => {});
}
/** Grow at once so the animation is never clipped; shrink once it has settled. */
function syncWindow() {
  const f = finalSize();
  if (f.w > win.w || f.h > win.h) setWindow(Math.max(f.w, win.w), Math.max(f.h, win.h));
  clearTimeout(shrinkTimer);
  shrinkTimer = window.setTimeout(() => { const g = finalSize(); setWindow(g.w, g.h); }, reduce ? 0 : 480);
}
new ResizeObserver(() => { if (S.mode === "expanded") syncWindow(); }).observe($(".body"));

/* ---------- Interaction ---------- */
$("#head").addEventListener("click", e => {
  e.stopPropagation(); S.lastActive = Date.now();
  S.mode = ({ hidden: "peek", peek: "compact", compact: "expanded", expanded: "compact" } as const)[S.mode];
  render();
});
island.addEventListener("click", () => { if (S.mode === "hidden") { S.mode = "peek"; render(); } });
// The hidden window is only the wake strip: the bar or the space beside it.
const wake = () => { if (S.mode === "hidden") { S.mode = "peek"; render(); } };
$("#hotzone").addEventListener("pointerenter", wake);
island.addEventListener("pointerenter", wake);
$("#hotzone").addEventListener("click", () => {
  if (S.mode === "hidden") S.mode = "peek"; else if (S.mode === "peek") S.mode = "compact";
  render();
});
island.addEventListener("pointerleave", () => { later(() => { if (S.mode === "peek") { S.mode = "hidden"; render(); } }, 900); });
document.addEventListener("keydown", e => {
  if (e.key !== "Escape") return;
  S.mode = S.mode === "expanded" ? "compact" : "hidden"; render();
});
/* ---------- Approval buttons ----------
   An answer only counts when it is unmistakably meant for the card on screen:
   focus is cleared when a card appears, presses in its first 400 ms are
   ignored, and a click counts only if the press started on that same button
   while this card was showing. Nothing left over from the previous card (a
   focused button, a late or repeated click) can answer the new one. */
const ARM_MS = 400;
const answerButtons = ["#b-allow", "#b-deny", "#b-always"].map(s => $<HTMLButtonElement>(s));
let armedAt = Infinity, armTimer = 0;
let pressed: { el: EventTarget | null; card: string } = { el: null, card: "" };

function armApproval() {
  (document.activeElement as HTMLElement | null)?.blur();
  pressed = { el: null, card: "" };
  armedAt = Date.now() + ARM_MS;
  answerButtons.forEach(b => (b.disabled = true));
  clearTimeout(armTimer);
  armTimer = window.setTimeout(() => answerButtons.forEach(b => (b.disabled = false)), ARM_MS);
}

function answer(choice: Choice, ev: MouseEvent) {
  const a = S.approval;
  const ok = a && ev.isTrusted && Date.now() >= armedAt && pressed.el === ev.currentTarget && pressed.card === a.id;
  pressed = { el: null, card: "" };
  if (ok) decide(choice);
}

for (const [b, choice] of [[answerButtons[0], "allow"], [answerButtons[1], "deny"], [answerButtons[2], "always"]] as const) {
  b.addEventListener("pointerdown", ev => {
    pressed = Date.now() >= armedAt && S.approval ? { el: ev.currentTarget, card: S.approval.id } : { el: null, card: "" };
  });
  b.addEventListener("click", ev => answer(choice, ev));
}

listen<AgentEvent>("hook", e => applyHook(e.payload));
// The relay went away or the 108 s ran out: Claude Code asks in the terminal.
listen<string>("permission-gone", e => {
  const queued = S.queue.length;
  S.queue = S.queue.filter(a => a.id !== e.payload);
  if (S.queue.length !== queued) { render(); return; }
  if (S.approval?.id !== e.payload) return;
  S.approval.id = "";
  decide("ask", true);
});
listen("show", () => { S.lastActive = Date.now(); if (S.mode === "hidden" || S.mode === "peek") S.mode = "compact"; render(); });

/* ---------- The droplet ---------- */
const cv = $<HTMLCanvasElement>("#drop"), ctx = cv.getContext("2d")!;
const C: Record<string, string> = {};
const cs = getComputedStyle(document.documentElement);
for (const k of ["drip", "warn", "ok", "bad", "eye", "eye-white"]) C[k] = cs.getPropertyValue("--" + k).trim();

let px = innerWidth / 2, py = 0;
addEventListener("pointermove", e => { px = e.clientX; py = e.clientY; });
let nextBlink = performance.now() + 2500, blinkUntil = 0;

function sizeCanvas() {
  const dpr = Math.min(3, devicePixelRatio || 1);
  cv.width = Math.round(40 * dpr); cv.height = Math.round(46 * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
}
sizeCanvas();
matchMedia(`(resolution: ${devicePixelRatio}dppx)`).addEventListener("change", sizeCanvas);

function draw(now: number) {
  const t = reduce ? 0 : now / 1000, w = 40, h = 46, st = S.status;
  ctx.clearRect(0, 0, w, h);
  const color = ({ approval: C.warn, finished: C.ok, error: C.bad } as Record<string, string>)[st] || C.drip;
  let cx = w / 2, cy = h * 0.64; const r = 11;
  if (!reduce) {
    if (st === "approval") cy -= Math.abs(Math.sin(t * 5)) * 3;
    if (st === "finished") cy -= Math.max(0, Math.sin(t * 3.2)) * 3.5;
    if (st === "error") cx += Math.sin(t * 38) * 0.7;
  }
  const speed = st === "working" ? 4 : 2;
  const sx = 1 + (reduce ? 0 : 0.05 * Math.sin(t * speed)), sy = 1 / sx;

  ctx.save(); ctx.translate(cx, cy); ctx.scale(sx, sy);
  ctx.beginPath();
  ctx.moveTo(0, -r * 1.8);
  ctx.bezierCurveTo(r * 0.35, -r * 1.2, r, -r * 0.6, r, 0);
  ctx.arc(0, 0, r, 0, Math.PI);
  ctx.bezierCurveTo(-r, -r * 0.6, -r * 0.35, -r * 1.2, 0, -r * 1.8);
  ctx.closePath();
  ctx.fillStyle = color; ctx.fill();
  const shade = ctx.createRadialGradient(-r * 0.3, -r * 0.4, r * 0.2, 0, 0, r * 1.5);
  shade.addColorStop(0, "rgba(255,255,255,0.22)"); shade.addColorStop(0.6, "rgba(255,255,255,0)"); shade.addColorStop(1, "rgba(0,0,0,0.28)");
  ctx.fillStyle = shade; ctx.fill();
  ctx.beginPath(); ctx.ellipse(-r * 0.48, -r * 0.55, r * 0.14, r * 0.28, -0.5, 0, Math.PI * 2);
  ctx.fillStyle = "rgba(255,255,255,0.55)"; ctx.fill();

  // Eyes
  const rect = cv.getBoundingClientRect();
  let lx = 0, ly = 0;
  if (rect.width) {
    const dx = px - (rect.left + rect.width / 2), dy = py - (rect.top + rect.height / 2);
    lx = dx / (Math.abs(dx) + 140); ly = dy / (Math.abs(dy) + 140);
  }
  if (st === "working" && !reduce) { lx = Math.sin(t * 1.6) * 0.9; ly = 0.3; }
  if (now > nextBlink) { blinkUntil = now + 130; nextBlink = now + 2500 + Math.random() * 3500; }
  const blink = now < blinkUntil ? 0.12 : 1;
  const ey = r * 0.08, ex = r * 0.38;
  ctx.lineCap = "round"; ctx.lineWidth = 1.6; ctx.strokeStyle = C.eye;
  for (const side of [-1, 1]) {
    const x = side * ex;
    if (st === "finished") { ctx.beginPath(); ctx.arc(x, ey + 1.5, r * 0.2, Math.PI * 1.15, Math.PI * 1.85); ctx.stroke(); continue; }
    if (st === "error") { const k = r * 0.15; ctx.beginPath(); ctx.moveTo(x - k, ey - k); ctx.lineTo(x + k, ey + k); ctx.moveTo(x + k, ey - k); ctx.lineTo(x - k, ey + k); ctx.stroke(); continue; }
    const big = st === "approval" ? 1.18 : 1;
    ctx.beginPath(); ctx.ellipse(x, ey, r * 0.21 * big, r * 0.27 * big * blink, 0, 0, Math.PI * 2);
    ctx.fillStyle = C["eye-white"]; ctx.fill();
    if (blink > 0.5) { ctx.beginPath(); ctx.arc(x + lx * r * 0.09, ey + ly * r * 0.1, r * 0.12 * big, 0, Math.PI * 2); ctx.fillStyle = C.eye; ctx.fill(); }
  }
  ctx.restore();
}

let lastText = 0;
function loop(now: number) {
  if (S.mode !== "hidden") draw(now);
  if (now - lastText > 250) { lastText = now; tickText(); }
  requestAnimationFrame(loop);
}

initial();
if (import.meta.env.DEV) sample();
render();
requestAnimationFrame(loop);
