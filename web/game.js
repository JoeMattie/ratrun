import { CanvasRenderer } from "./canvas.js";
import init, { block_masks, KeyPhase, PointerAction, TouchPhase, WebGame } from "./ratrun.js";

const host = document.querySelector("#terminal");
const status = document.querySelector("#status");
const plate = document.querySelector("#portrait");
const frameMs = 1000 / 60;

// A coarse primary pointer is a thumb, not a mouse: phones and tablets get the
// touch grid (see canvas.js), the landscape gate and the first-touch setup.
const coarse = window.matchMedia("(pointer: coarse)");
const portrait = window.matchMedia("(orientation: portrait)");
let gated = false;
let finished = false;
let onGate = () => {};
let onPlate = () => {};

// Portrait on a touch device has no room for the arena and the on-screen
// controls, so the plate covers the grid and the touch stream stops. The gate
// is live from the first byte, before the wasm has loaded.
function updateGate() {
  const next = !finished && coarse.matches && portrait.matches;
  if (next === gated) return;
  gated = next;
  plate.hidden = !gated;
  if (gated) onGate();
}

updateGate();
for (const query of [coarse, portrait]) query.addEventListener("change", updateGate);
window.addEventListener("orientationchange", updateGate);

// Tapping the plate is the first gesture on a phone that is not playable yet.
// It takes fullscreen and asks for the landscape lock, which is the only way
// Android will turn the page for the player; the grid stays stopped.
plate.addEventListener("pointerdown", (event) => {
  event.preventDefault();
  onPlate();
});

// Cache-first once installed, so a home-screen run opens from the cache. A
// service worker needs a secure origin; localhost is the web/build.sh preview.
function registerWorker() {
  if (!("serviceWorker" in navigator)) return;
  const { protocol, hostname } = location;
  if (protocol !== "https:" && hostname !== "localhost" && hostname !== "127.0.0.1") return;
  navigator.serviceWorker.register("./sw.js", { scope: "./" }).catch(() => {});
}

function modifiers(event) {
  // Matches input_event::KeyModifiers.
  return (event.shiftKey ? 1 : 0) | (event.ctrlKey ? 2 : 0)
    | (event.altKey ? 4 : 0) | (event.metaKey ? 8 : 0);
}

async function start() {
  registerWorker();
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
  coarse.addEventListener("change", () => { resizePending = true; });

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

  // Fullscreen, the landscape lock and the wake lock all start from the first
  // trusted touch: the one gesture every mobile browser grants.
  let touched = false;
  let fullscreenAsked = false;
  let wake = null;

  function requestFullscreen() {
    if (fullscreenAsked) return Promise.resolve();
    fullscreenAsked = true;
    const element = document.documentElement;
    const request = element.requestFullscreen ?? element.webkitRequestFullscreen;
    if (!request) return Promise.resolve();
    try {
      return Promise.resolve(request.call(element, { navigationUI: "hide" })).catch(() => {});
    } catch (error) {
      return Promise.resolve();
    }
  }

  // The platform may refuse (iOS Safari has neither routine fullscreen nor an
  // orientation lock); the plate and the grid carry on either way.
  function lockLandscape() {
    const orientation = window.screen.orientation;
    if (!orientation || typeof orientation.lock !== "function") return;
    try {
      Promise.resolve(orientation.lock("landscape")).catch(() => {});
    } catch (error) {
      // Unsupported.
    }
  }

  // Only a phone holds the display on, and only after the touch that started
  // fullscreen; a desktop browser is left to its own sleep settings.
  async function keepAwake() {
    if (!touched || wake || document.hidden || !("wakeLock" in navigator)) return;
    try {
      wake = await navigator.wakeLock.request("screen");
      wake.addEventListener("release", () => { wake = null; });
    } catch (error) {
      wake = null;
    }
  }

  function releaseWake() {
    const lock = wake;
    wake = null;
    if (lock) Promise.resolve(lock.release()).catch(() => {});
  }

  function firstTouch(event) {
    if (!event.isTrusted) return;
    game.unlock_audio();
    if (touched) return;
    touched = true;
    requestFullscreen().then(lockLandscape);
    keepAwake();
  }

  document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
      releaseWake();
      blur();
    } else {
      focus();
      keepAwake();
    }
  });

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

  // Touch goes to Rust with fractional cells, so a thumb slides inside one cell
  // and two fingers can pinch. Rust owns the stick, the buttons, taps, swipes
  // and the pinch; the page never turns a touch into a mouse click.
  const fingers = new Map();

  function cell(event) {
    const bounds = screen.getBoundingClientRect();
    if (!bounds.width || !bounds.height) return null;
    // Half a cell inside each edge keeps every value inside Rust's 0..cols even
    // when a thumb lands in the margin around the grid.
    return {
      col: Math.min(canvas.cols - 0.5,
        Math.max(0, (event.clientX - bounds.left) * canvas.cols / bounds.width)),
      row: Math.min(canvas.rows - 0.5,
        Math.max(0, (event.clientY - bounds.top) * canvas.rows / bounds.height)),
    };
  }

  function touch(event, phase) {
    if (stopped || gated) return;
    const at = cell(event);
    if (!at) return;
    if (phase === TouchPhase.Start || phase === TouchPhase.Move) fingers.set(event.pointerId, at);
    else fingers.delete(event.pointerId);
    game.touch(phase, event.pointerId, at.col, at.row);
  }

  function cancelTouches() {
    if (stopped) return;
    for (const [id, at] of fingers) game.touch(TouchPhase.Cancel, id, at.col, at.row);
    fingers.clear();
  }

  onGate = cancelTouches;
  onPlate = () => {
    touched = true;
    requestFullscreen().then(lockLandscape);
    keepAwake();
  };

  host.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    if (event.pointerType === "touch") {
      // No button, no capture of our own and no focus: the browser already
      // holds an implicit capture for the touch, and focusing anything is what
      // raises a soft keyboard.
      firstTouch(event);
      touch(event, TouchPhase.Start);
      return;
    }
    host.focus();
    if (event.isTrusted) game.unlock_audio();
    host.setPointerCapture(event.pointerId);
    pointer(event, PointerAction.Down, event.button);
  });
  host.addEventListener("pointerup", (event) => {
    if (event.pointerType === "touch") touch(event, TouchPhase.End);
    else pointer(event, PointerAction.Up, event.button);
  });
  host.addEventListener("pointercancel", (event) => {
    if (event.pointerType === "touch") {
      touch(event, TouchPhase.Cancel);
      return;
    }
    blur();
    focus();
  });
  host.addEventListener("pointermove", (event) => {
    if (event.pointerType === "touch") {
      // A finger that never touched the glass is not a contact.
      if (fingers.has(event.pointerId)) touch(event, TouchPhase.Move);
      return;
    }
    const button = event.buttons & 1 ? 0 : event.buttons & 2 ? 2 : event.buttons & 4 ? 1 : -1;
    pointer(event, PointerAction.Move, button);
  });

  // No long-press callout, no context menu, no text selection, no browser
  // pinch. `touch-action: none` in the page handles the rest.
  document.addEventListener("contextmenu", (event) => event.preventDefault());
  for (const type of ["gesturestart", "gesturechange", "gestureend"]) {
    document.addEventListener(type, (event) => event.preventDefault(), { passive: false });
  }
  document.addEventListener("dblclick", (event) => event.preventDefault(), { passive: false });

  host.addEventListener("wheel", (event) => {
    event.preventDefault();
    if (event.deltaY) pointer(event, event.deltaY < 0 ? PointerAction.ScrollUp : PointerAction.ScrollDown);
  }, { passive: false });

  function unload() {
    releaseWake();
    cancelTouches();
    const exit = document.exitFullscreen ?? document.webkitExitFullscreen;
    if (document.fullscreenElement && exit) {
      Promise.resolve().then(() => exit.call(document)).catch(() => {});
    }
  }
  window.addEventListener("pagehide", unload);

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
      cancelTouches();
      stopped = true;
      finished = true;
      plate.hidden = true;
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
