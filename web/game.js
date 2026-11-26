import { Terminal } from "./vendor/xterm.mjs";
import { FitAddon } from "./vendor/addon-fit.mjs";
import init, { KeyPhase, PointerAction, WebGame } from "./ratrun.js";

const host = document.querySelector("#terminal");
const status = document.querySelector("#status");
const frameMs = 1000 / 60;

function modifiers(event) {
  // Matches input_event::KeyModifiers.
  return (event.shiftKey ? 1 : 0) | (event.ctrlKey ? 2 : 0)
    | (event.altKey ? 4 : 0) | (event.metaKey ? 8 : 0);
}

async function start() {
  await init();
  await document.fonts.ready;
  // Use the DOM renderer. The WebGL addon can move entries in its glyph
  // atlas while assembling a frame of changing truecolor block glyphs.
  const terminal = new Terminal({
    fontFamily: 'Menlo, Consolas, "DejaVu Sans Mono", monospace',
    customGlyphs: true,
    minimumContrastRatio: 1,
    scrollback: 0,
    disableStdin: true,
    theme: { background: "#090a0d", foreground: "#e1d8cc" },
  });
  terminal.open(host);
  const fit = new FitAddon();
  terminal.loadAddon(fit);

  function fitGrid() {
    const fontSize = Math.min(20, Math.floor(host.clientWidth / 76), Math.floor(host.clientHeight / 54));
    terminal.options.fontSize = Math.max(10, fontSize);
    fit.fit();
  }

  fitGrid();
  const game = new WebGame(terminal.cols, terminal.rows);
  terminal.onResize(({ cols, rows }) => game.resize(cols, rows));
  terminal.write("\x1b[?25l\x1b[?7l"); // Hide the text cursor and disable auto-wrap.
  const screen = host.querySelector(".xterm-screen");
  status.hidden = true;
  terminal.focus();

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
    const col = Math.floor((event.clientX - bounds.left) * terminal.cols / bounds.width);
    const row = Math.floor((event.clientY - bounds.top) * terminal.rows / bounds.height);
    if (action !== PointerAction.Up && (col < 0 || row < 0 || col >= terminal.cols || row >= terminal.rows)) return;
    game.pointer(action, button,
      Math.min(terminal.cols - 1, Math.max(0, col)),
      Math.min(terminal.rows - 1, Math.max(0, row)),
      modifiers(event));
  }

  screen.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    terminal.focus();
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
  const timings = { wasm: [], write: [], cadence: [] };
  let skipped = 0;
  if (profile !== null) {
    game.start_profile(profile === "full");
    window.ratrunProfile = () => {
      const stages = JSON.parse(game.profile());
      const result = { ...stages, cols: terminal.cols, rows: terminal.rows, skipped };
      for (const [key, values] of Object.entries(timings)) result[key] = values.splice(0);
      skipped = 0;
      return result;
    };
  }

  function measure(name, end = performance.now()) {
    const samples = timings[name];
    samples.push(performance.measure(name, { start: `ratrun-${name}`, end }).duration);
    performance.clearMarks(`ratrun-${name}`);
    performance.clearMeasures(name);
    if (samples.length === 1200) samples.splice(0, 600);
  }

  let writing = false;
  const decoder = new TextDecoder();
  let next = 0;
  let previous;
  function frame(now) {
    if (now + 0.5 < next) {
      requestAnimationFrame(frame);
      return;
    }
    next = next ? Math.max(next + frameMs, now + 0.5) : now + frameMs;
    if (!writing && resizePending) {
      fitGrid();
      resizePending = false;
    }
    if (profile !== null) {
      if (previous !== undefined) {
        performance.mark("ratrun-cadence", { startTime: previous });
        measure("cadence", now);
      }
      previous = now;
      if (writing) skipped++;
      performance.mark("ratrun-wasm");
    }
    const bytes = game.frame(!writing);
    if (profile !== null) measure("wasm");
    host.style.cursor = game.crosshair() ? "crosshair" : "default";
    if (bytes.length) {
      writing = true;
      if (profile !== null) performance.mark("ratrun-write");
      // Decode the complete frame before passing it to xterm. Its byte-input
      // decoder can drop a glyph such as — across an internal chunk boundary.
      terminal.write(decoder.decode(bytes), () => {
        writing = false;
        if (profile !== null) measure("write");
        if (debugFrames && !stopped && window.ratrunDebugFrame) {
          window.ratrunDebugFrame(terminal, JSON.parse(game.debug_frame()));
        }
      });
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
