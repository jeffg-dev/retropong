import { ControllerInput, padCommands } from "./controller.js";
import { AudioPlayer } from "./audio.js";
import { createRenderer } from "./renderer.js";
import { SettingsStore } from "./settings.js";
import { TouchGestures } from "./touch.js";

const settings = new SettingsStore();

const root = document.getElementById("console");
const canvas = document.getElementById("game");
const audio = new AudioPlayer();
let core, renderer, loading, audioLoading;
let previousTime = 0,
  previousButtons = 0,
  commands = 0;
let initialized = false,
  exited = false,
  exitKnobs = null;
let state = "waiting";
// Launch hints name the input last used: touch, else the keyboard.
let touchHints = Boolean(globalThis.matchMedia?.("(pointer: coarse)").matches);

function setState(value) {
  state = value;
  root.dataset.state = value;
  canvas.setAttribute(
    "aria-label",
    value === "playing"
      ? "RetroPong. Enter or tap to start. Escape or hold for menu. Arrow keys navigate, Enter selects; tap a menu entry to change it."
      : "RETROPONG. CONNECT CONTROLLER & TURN KNOB",
  );
}
function resetClock() {
  previousTime = 0;
  commands = 0;
}
function pause() {
  core?.pause();
  audio.stop();
  resetClock();
}
function report(error) {
  console.error("RetroPong:", error);
}
function setTouchHints(on) {
  touchHints = on;
  core?.set_touch_hints(+on);
}

async function loadGame() {
  if (core) return;
  if (!loading)
    loading = (async () => {
      const response = await fetch("./retropong.wasm");
      if (!response.ok)
        throw new Error(`Game download failed (${response.status})`);
      const { instance } = await WebAssembly.instantiate(
        await response.arrayBuffer(),
        {},
      );
      renderer ||= await createRenderer(canvas);
      core = instance.exports;
      core.set_touch_hints(+touchHints);
      settings.init(core);
    })().catch((error) => {
      loading = null;
      report(error);
    });
  return loading;
}
function startConnected() {
  if (!core || state === "playing" || exited) return;
  if (!initialized) {
    settings.init(core);
    initialized = true;
  }
  audioLoading ||= audio.load();
  resetClock();
  setState("playing");
  canvas.focus({ preventScroll: true });
}
function disconnected() {
  previousButtons = 0;
  if (initialized) pause();
  exited = false;
  exitKnobs = null;
  setState("waiting");
}
const input = new ControllerInput({
  onDisconnect: disconnected,
  onError: report,
});

function exitGame(pad) {
  exited = true;
  initialized = false;
  exitKnobs = [pad.left, pad.right];
  audio.stop();
  resetClock();
  setState("waiting");
}
function connect() {
  void audio.unlock();
  const pad = input.poll();
  if (pad) {
    exited = false;
    startConnected();
  } else if (navigator.hid) void input.requestDevice();
  if (!core) void loadGame();
}
canvas.addEventListener("click", () => {
  if (state === "playing") void audio.unlock();
  else connect();
});
// While playing, a tap is Enter or Esc, or a menu entry; a hold is Esc.
new TouchGestures(canvas, {
  enabled: () => state === "playing" && Boolean(core),
  onTouch: () => {
    if (!touchHints) setTouchHints(true);
  },
  onTap: (x, y) => {
    void audio.unlock();
    core.tap(x, y);
  },
  onHold: () => {
    commands |= 1;
  },
});
const keyCommands = {
  Escape: 1,
  ArrowUp: 4,
  ArrowDown: 8,
  ArrowLeft: 16,
  ArrowRight: 32,
  Enter: 64,
};
document.addEventListener("keydown", (event) => {
  if (event.repeat) {
    if (keyCommands[event.key]) event.preventDefault();
    return;
  }
  if (touchHints) setTouchHints(false);
  if (state !== "playing") {
    if (event.key === "Enter") {
      event.preventDefault();
      connect();
    }
    return;
  }
  void audio.unlock();
  const command = keyCommands[event.key];
  if (command) {
    event.preventDefault();
    commands |= command;
  }
});
document.addEventListener("visibilitychange", () => {
  resetClock();
  if (document.hidden && state === "playing") pause();
});
window.addEventListener("pagehide", (event) => {
  pause();
  if (!event.persisted) input.dispose();
});
window.addEventListener("pageshow", resetClock);

function frame(now) {
  requestAnimationFrame(frame);
  const pad = input.poll();
  if (!core || document.hidden) {
    resetClock();
    return;
  }
  if (
    exited &&
    pad &&
    exitKnobs &&
    (Math.abs(pad.left - exitKnobs[0]) > 0.03 ||
      Math.abs(pad.right - exitKnobs[1]) > 0.03)
  )
    exited = false;
  if (pad && !exited && state !== "playing") startConnected();
  const delta = previousTime ? Math.min((now - previousTime) / 1000, 0.25) : 0;
  previousTime = now;
  let pointer;
  if (state === "playing" && pad) {
    const pressed = pad.buttons & ~previousButtons;
    previousButtons = pad.buttons;
    commands |= padCommands(pressed);
    const running = core.runs_console();
    const sound = core.advance(delta, pad.left, pad.right, commands);
    commands = 0;
    settings.save(core);
    if (running && !core.runs_console()) audio.stop();
    audio.volume = core.volume();
    audio.play(sound);
    if (core.exit_requested()) {
      exitGame(pad);
    }
    root.dataset.menu = core.menu_open() ? "open" : "closed";
  }
  try {
    pointer = state === "playing" ? core.render() : core.render_splash(delta);
    const width = core.width(),
      height = core.height();
    renderer.draw(
      new Uint8Array(core.memory.buffer, pointer, width * height * 4),
      width,
      height,
      state !== "playing" || Boolean(core.crt_enabled()),
    );
  } catch (error) {
    pause();
    report(error);
  }
}
// Load the native font/palette raster for the static connection screen.
void loadGame();
if (window.isSecureContext) void input.init();
requestAnimationFrame(frame);
