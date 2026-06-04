// MM-06 dev UI (dev-ui-spec.md) — a throwaway control surface for auditioning
// the engine. Not the shipping UI: no styling polish, persistence, or patterns.
//
// Seven voices (all analog/synthesized), one QWERTY key each, a LEVEL slider and
// an Accent toggle per voice, plus a global strip (Master / Accent / Drive /
// Variance / Seed). Everything routes through the typed Bridge to the worklet.

import "./styles.css";
import { Bridge } from "./bridge";
import {
  VOICES,
  defaultKit,
  replay,
  type KitState,
} from "./state";
import {
  GLOBAL_MASTER_LEVEL,
  GLOBAL_ACCENT_LEVEL,
  GLOBAL_DRIVE,
  GLOBAL_VARIANCE_DEPTH,
  GLOBAL_SEED,
  type VoiceId,
} from "./protocol";

const kit: KitState = defaultKit();
let bridge: Bridge | null = null;

// Per-voice panel element refs, for fire feedback.
const panelEls = new Map<VoiceId, HTMLElement>();
// Keys currently held, to suppress auto-repeat machine-gunning.
const heldKeys = new Set<string>();

const keyToVoice = new Map<string, VoiceId>();
for (const v of VOICES) keyToVoice.set(v.key.toLowerCase(), v.id);

/** rAF-throttled sender: coalesces rapid slider input to one send per frame. */
function rafThrottle(fn: (value: number) => void): (value: number) => void {
  let pending: number | null = null;
  let scheduled = false;
  return (value: number) => {
    pending = value;
    if (scheduled) return;
    scheduled = true;
    requestAnimationFrame(() => {
      scheduled = false;
      if (pending !== null) fn(pending);
    });
  };
}

function fireVoice(id: VoiceId): void {
  if (!bridge) return;
  bridge.trigger(id, kit.accents[id]);
}

function flashPanel(id: VoiceId): void {
  const el = panelEls.get(id);
  if (!el) return;
  el.classList.add("fired");
  setTimeout(() => el.classList.remove("fired"), 90);
}

// --- DOM construction --------------------------------------------------------

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  Object.assign(node, props);
  for (const c of children) node.append(c);
  return node;
}

function buildGlobals(): HTMLElement {
  const strip = el("section", { className: "globals" });
  strip.append(el("h2", { textContent: "Globals" }));

  const slider = (
    labelText: string,
    initial: number,
    onInput: (v: number) => void,
  ): HTMLElement => {
    const wrap = el("label", { className: "ctrl" });
    const valEl = el("span", { className: "val", textContent: initial.toFixed(2) });
    const input = el("input", {
      type: "range",
      min: "0",
      max: "1",
      step: "0.01",
      value: String(initial),
    });
    const throttled = rafThrottle(onInput);
    input.addEventListener("input", () => {
      const v = Number(input.value);
      valEl.textContent = v.toFixed(2);
      throttled(v);
    });
    wrap.append(el("span", { className: "lbl", textContent: labelText }), input, valEl);
    return wrap;
  };

  strip.append(
    slider("Master", kit.master, (v) => {
      kit.master = v;
      bridge?.setGlobal(GLOBAL_MASTER_LEVEL, v);
    }),
    slider("Accent", kit.accentLevel, (v) => {
      kit.accentLevel = v;
      bridge?.setGlobal(GLOBAL_ACCENT_LEVEL, v);
    }),
    slider("Drive", kit.drive, (v) => {
      kit.drive = v;
      bridge?.setGlobal(GLOBAL_DRIVE, v);
    }),
    slider("Variance", kit.variance, (v) => {
      kit.variance = v;
      bridge?.setGlobal(GLOBAL_VARIANCE_DEPTH, v);
    }),
  );

  // Seed integer field (text input — guarded against triggering voices).
  const seedWrap = el("label", { className: "ctrl seed-row" });
  const seedInput = el("input", {
    type: "number",
    min: "0",
    step: "1",
    value: String(kit.seed),
    className: "seed",
  });
  seedInput.addEventListener("change", () => {
    const v = Math.max(0, Math.floor(Number(seedInput.value) || 0));
    kit.seed = v;
    bridge?.setGlobal(GLOBAL_SEED, v);
  });
  seedWrap.append(el("span", { className: "lbl", textContent: "Seed" }), seedInput);
  strip.append(seedWrap);

  return strip;
}

function buildVoicePanel(v: (typeof VOICES)[number]): HTMLElement {
  const panel = el("section", { className: "voice" });
  panelEls.set(v.id, panel);

  panel.append(
    el("div", { className: "head" },
      el("span", { className: "name", textContent: `${v.label} — ${v.name}` }),
      el("kbd", { textContent: v.key }),
    ),
  );

  // LEVEL slider (normalized 0..1, default 0.5).
  const levelWrap = el("label", { className: "ctrl" });
  const valEl = el("span", { className: "val", textContent: kit.levels[v.id].toFixed(2) });
  const level = el("input", {
    type: "range",
    min: "0",
    max: "1",
    step: "0.01",
    value: String(kit.levels[v.id]),
  });
  const throttled = rafThrottle((value) => bridge?.setLevel(v.id, value));
  level.addEventListener("input", () => {
    const value = Number(level.value);
    kit.levels[v.id] = value;
    valEl.textContent = value.toFixed(2);
    throttled(value);
  });
  levelWrap.append(el("span", { className: "lbl", textContent: "LEVEL" }), level, valEl);
  panel.append(levelWrap);

  // Accent toggle.
  const accentBtn = el("button", { className: "accent", textContent: "Accent" });
  accentBtn.addEventListener("click", () => {
    kit.accents[v.id] = !kit.accents[v.id];
    accentBtn.classList.toggle("on", kit.accents[v.id]);
  });

  // Trigger button (mouse equivalent of the key).
  const trigBtn = el("button", { className: "trigger", textContent: `Trigger (${v.key})` });
  trigBtn.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    fireVoice(v.id);
    flashPanel(v.id);
  });

  const row = el("div", { className: "btnrow" }, accentBtn, trigBtn);
  panel.append(row);
  return panel;
}

function buildUI(): void {
  const app = document.getElementById("app")!;
  app.innerHTML = "";

  app.append(buildGlobals());

  const grid = el("div", { className: "grid" });
  for (const v of VOICES) grid.append(buildVoicePanel(v));
  app.append(grid);

  const hint = el("p", { className: "hint" });
  hint.textContent =
    "Keys Q W E R T Y U trigger BD SD LT HT CY OH CH. " +
    "Placeholder audio: every voice plays the same short bleep for now — real TR-606 " +
    "synthesis comes later. LEVEL / Master / Drive / Accent are live; Variance / Seed are inert.";
  app.append(hint);
}

// --- audio bootstrap (user gesture) ------------------------------------------

function buildStartOverlay(): void {
  const overlay = el("div", { className: "overlay" });
  const btn = el("button", { className: "start", textContent: "▶ Start audio" });
  const status = el("p", { className: "status", textContent: "" });
  btn.addEventListener("click", async () => {
    btn.disabled = true;
    status.textContent = "starting engine…";
    try {
      bridge = await Bridge.create({
        onVoiceFired: (voice) => flashPanel(voice as VoiceId),
        onXRun: () => console.warn("MM-06: XRun (process overran)"),
      });
      replay(bridge, kit);
      overlay.remove();
    } catch (err) {
      console.error(err);
      status.textContent = `failed: ${String(err)}`;
      btn.disabled = false;
    }
  });
  overlay.append(
    el("h1", { textContent: "MM-06" }),
    el("p", { textContent: "TR-606 emulation — fully analog, no samples." }),
    btn,
    status,
  );
  document.body.append(overlay);
}

// --- keyboard ----------------------------------------------------------------

function isTextInput(target: EventTarget | null): boolean {
  const t = target as HTMLElement | null;
  return !!t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA");
}

window.addEventListener("keydown", (e) => {
  if (isTextInput(e.target)) return; // don't fire while typing a seed
  const key = e.key.toLowerCase();
  const voice = keyToVoice.get(key);
  if (voice === undefined) return;
  if (heldKeys.has(key)) return; // suppress auto-repeat
  heldKeys.add(key);
  e.preventDefault();
  fireVoice(voice);
  flashPanel(voice);
});

window.addEventListener("keyup", (e) => {
  heldKeys.delete(e.key.toLowerCase());
});

// --- go ----------------------------------------------------------------------

buildUI();
buildStartOverlay();
