import { ControllerInput } from "./controller.js";
import { AudioPlayer } from "./audio.js";
import { createRenderer } from "./renderer.js";
import { SettingsStore } from "./settings.js";

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

function setState(value) {
  state = value;
  root.dataset.state = value;
  canvas.setAttribute(
    "aria-label",
    value === "playing"
      ? "RetroPong. Enter to start. Escape for menu. Arrow keys navigate, Enter selects."
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
    if (pressed & (1 << 9)) commands |= 1;
    if (pressed & (1 << 1)) commands |= 2;
    if (pressed & (1 << 12)) commands |= 4;
    if (pressed & (1 << 13)) commands |= 8;
    if (pressed & (1 << 14)) commands |= 16;
    if (pressed & (1 << 15)) commands |= 32;
    if (pressed & 1 && core.menu_open()) commands |= 64;
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
