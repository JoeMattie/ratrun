import { CanvasRenderer } from "./canvas.js";
import init, { block_masks, KeyPhase, PointerAction, WebGame } from "./ratrun.js";

const host = document.querySelector("#terminal");
const status = document.querySelector("#status");
const frameMs = 1000 / 60;

function modifiers(event) {
  // Matches input_event::KeyModifiers.
  return (event.shiftKey ? 1 : 0) | (event.ctrlKey ? 2 : 0)
    | (event.altKey ? 4 : 0) | (event.metaKey ? 8 : 0);
}

async function start() {
  const { memory } = await init();
  await document.fonts.ready;
  const screen = document.querySelector("#text");
  const canvas = new CanvasRenderer(document.querySelector("#picture"), screen, block_masks());

  function fitGrid() {
    return canvas.fit(host.clientWidth, host.clientHeight, devicePixelRatio);
  }

  fitGrid();
  const game = new WebGame(canvas.cols, canvas.rows);
  status.hidden = true;
  host.focus();

  let resizePending = false;
  const resizeObserver = new ResizeObserver(() => { resizePending = true; });
  resizeObserver.observe(host);

  // Keep the original key identity until keyup, even if modifiers change
  // before a release. The shared input layer handles binds and held keys.
  const held = new Map();
  let stopped = false;
  const shortcuts = new Set(["Tab", "Enter", "Escape", "Backspace", "Delete", "Insert",
    "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown",
    "Control", "Shift", "Alt"]);

  host.addEventListener("keydown", (event) => {
    if (stopped || event.isComposing || event.metaKey) return;
    if (!shortcuts.has(event.key) && [...event.key].length !== 1 && !/^F(?:[1-9]|1[0-2])$/.test(event.key)) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.isTrusted) game.unlock_audio();
    const key = held.get(event.code) ?? event.key;
    held.set(event.code, key);
    game.key(key, event.location, modifiers(event), event.repeat ? KeyPhase.Repeat : KeyPhase.Press);
  }, true);

  window.addEventListener("keyup", (event) => {
    if (stopped || !held.has(event.code)) return;
    event.preventDefault();
    game.key(held.get(event.code), event.location, modifiers(event), KeyPhase.Release);
    held.delete(event.code);
  }, true);

  function blur() {
    if (stopped) return;
    held.clear();
    game.focus(false);
  }

  function focus() {
    if (!stopped && host.contains(document.activeElement)) game.focus(true);
  }

  host.addEventListener("focusin", focus);
  host.addEventListener("focusout", blur);
  window.addEventListener("blur", blur);
  window.addEventListener("focus", focus);
  document.addEventListener("visibilitychange", () => document.hidden ? blur() : focus());

  function pointer(event, action, button = -1) {
    if (stopped) return;
    const bounds = screen.getBoundingClientRect();
    if (!bounds.width || !bounds.height) return;
    const col = Math.floor((event.clientX - bounds.left) * canvas.cols / bounds.width);
    const row = Math.floor((event.clientY - bounds.top) * canvas.rows / bounds.height);
    if (action !== PointerAction.Up && (col < 0 || row < 0 || col >= canvas.cols || row >= canvas.rows)) return;
    game.pointer(action, button,
      Math.min(canvas.cols - 1, Math.max(0, col)),
      Math.min(canvas.rows - 1, Math.max(0, row)),
      modifiers(event));
  }

  screen.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    host.focus();
    if (event.isTrusted) game.unlock_audio();
    screen.setPointerCapture(event.pointerId);
    pointer(event, PointerAction.Down, event.button);
  });
  screen.addEventListener("pointerup", (event) => pointer(event, PointerAction.Up, event.button));
  screen.addEventListener("pointercancel", () => { blur(); focus(); });
  screen.addEventListener("pointermove", (event) => {
    const button = event.buttons & 1 ? 0 : event.buttons & 2 ? 2 : event.buttons & 4 ? 1 : -1;
    pointer(event, PointerAction.Move, button);
  });
  host.addEventListener("contextmenu", (event) => event.preventDefault());
  screen.addEventListener("wheel", (event) => {
    event.preventDefault();
    if (event.deltaY) pointer(event, event.deltaY < 0 ? PointerAction.ScrollUp : PointerAction.ScrollDown);
  }, { passive: false });

  const params = new URLSearchParams(location.search);
  const profile = params.get("profile");
  const debugFrames = params.has("debug-frames");
  const timings = { wasm: [], canvas: [], cadence: [] };
  if (profile !== null) {
    game.start_profile(profile === "full");
    window.ratrunProfile = () => {
      const stages = JSON.parse(game.profile());
      const result = { ...stages, cols: canvas.cols, rows: canvas.rows };
      for (const [key, values] of Object.entries(timings)) result[key] = values.splice(0);
      return result;
    };
  }

  function measure(name, start, end = performance.now()) {
    const samples = timings[name];
    const label = `ratrun-${name}`;
    samples.push(performance.measure(label, { start, end }).duration);
    performance.clearMeasures(label);
    if (samples.length === 3600) samples.splice(0, 1800);
  }

  let next = 0;
  let previous;
  let crosshair = false;
  function frame(now) {
    if (now + 0.5 < next) {
      requestAnimationFrame(frame);
      return;
    }
    next = next ? Math.max(next + frameMs, now + 0.5) : now + frameMs;
    if (resizePending || devicePixelRatio !== canvas.pixelRatio) {
      if (fitGrid()) game.resize(canvas.cols, canvas.rows);
      resizePending = false;
    }
    let started;
    if (profile !== null) {
      if (previous !== undefined) measure("cadence", previous, now);
      previous = now;
      started = performance.now();
    }
    const length = game.frame();
    if (profile !== null) measure("wasm", started);
    const cursor = game.crosshair();
    if (crosshair !== cursor) {
      crosshair = cursor;
      host.style.cursor = cursor ? "crosshair" : "default";
    }
    if (length) {
      if (profile !== null) started = performance.now();
      const cells = game.cells_ptr(), text = game.text_ptr();
      canvas.paint(memory.buffer, cells, length, text);
      if (profile !== null) measure("canvas", started);
      if (debugFrames && window.ratrunDebugFrame) {
        window.ratrunDebugFrame(canvas, JSON.parse(game.debug_frame()));
      }
    }
    if (game.quit()) {
      stopped = true;
      resizeObserver.disconnect();
      game.free();
      delete window.ratrunProfile;
      const restart = document.createElement("button");
      restart.id = "restart";
      restart.textContent = "RESTART";
      restart.addEventListener("click", () => location.reload());
      status.replaceChildren(restart);
      status.hidden = false;
      restart.focus();
      return;
    }
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
}

start().catch((error) => {
  console.error(error);
  status.textContent = "RAT RUN FAILED TO LOAD";
  status.hidden = false;
});
