import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

let binary;
try {
  binary = await readFile(new URL("../public/retropong.wasm", import.meta.url));
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
const options = {
  skip: binary ? false : "Run build.sh to test the compiled WASM artifact",
};
async function game() {
  const { instance } = await WebAssembly.instantiate(binary, {});
  instance.exports.init(3);
  return instance.exports;
}

test(
  "built WASM uses native launch, menu, countdown and full-range controller input",
  options,
  async () => {
    const g = await game();
    assert.equal(g.phase(), 0);
    assert.equal(g.volume(), 8);
    assert.equal(g.settings_snapshot() & (512 | 1024), 512 | 1024);
    g.advance(1 / 60, 0, 1, 64);
    assert.equal(g.phase(), 1);
    assert.equal(g.left_top(), 209);
    assert.equal(g.right_top(), 0);
    const frame = g.frame_number();
    g.advance(1 / 60, 0, 1, 1);
    assert.equal(g.menu_open(), 1);
    assert.equal(g.frame_number(), frame);
    const pointer = g.render();
    assert.equal(g.width(), 748);
    assert.equal(g.height(), 240);
    const rgba = new Uint8Array(g.memory.buffer, pointer, 748 * 240 * 4);
    assert.ok(rgba.some((v, i) => i % 4 !== 3 && v > 0));
    assert.ok(rgba.every((v, i) => i % 4 !== 3 || v === 255));
    g.advance(0, 0, 1, 64);
    assert.equal(g.menu_open(), 0);
    assert.equal(g.runs_console(), 0);
    for (let i = 0; i < 100; i++) g.advance(0.01, 1, 0, 64);
    assert.equal(
      g.frame_number(),
      frame,
      "Enter during countdown must not restart",
    );
    for (let i = 0; i < 110; i++) g.advance(0.01, 1, 0, 0);
    assert.equal(g.runs_console(), 1);
    assert.ok(g.frame_number() > frame);
    assert.equal(g.left_top(), 0);
    assert.equal(g.right_top(), 209);
  },
);

test(
  "built WASM maps RetroController buttons: left opens/activates, right cycles",
  options,
  async () => {
    const g = await game();
    g.advance(1 / 60, 0.5, 0.5, 64);
    const frame = g.frame_number();
    g.advance(0, 0.5, 0.5, 256);
    assert.equal(g.menu_open(), 0, "right does nothing while closed");
    g.advance(0, 0.5, 0.5, 128);
    assert.equal(g.menu_open(), 1, "left opens the menu");
    for (let i = 0; i < 8; i++) g.advance(0, 0.5, 0.5, 256);
    g.advance(0, 0.5, 0.5, 128);
    assert.equal(g.menu_open(), 0, "eight steps wrap back to RESUME");
    assert.equal(g.runs_console(), 0, "resuming counts down");
    assert.equal(g.frame_number(), frame, "left never restarts the match");
    g.advance(0, 0.5, 0.5, 128);
    assert.equal(g.menu_open(), 1, "left reopens during the countdown");
    for (let i = 0; i < 3; i++) g.advance(0, 0.5, 0.5, 256);
    g.advance(0, 0.5, 0.5, 128);
    assert.equal(g.crt_enabled(), 0, "left acts on the selected CRT entry");
  },
);

test(
  "built WASM native menu changes CRT and Exit Game signals the host",
  options,
  async () => {
    const g = await game();
    g.pause();
    for (let i = 0; i < 3; i++) g.advance(0, 0.5, 0.5, 8);
    g.advance(0, 0.5, 0.5, 64);
    assert.equal(g.crt_enabled(), 0);
    for (let i = 0; i < 4; i++) g.advance(0, 0.5, 0.5, 8);
    g.advance(0, 0.5, 0.5, 64);
    assert.equal(g.exit_requested(), 1);
  },
);

test(
  "built WASM settings roundtrip survives native menu changes, reload and replay",
  options,
  async () => {
    const { SettingsStore } = await import("../public/settings.js");
    let saved = null,
      writes = 0;
    const disk = {
      getItem: () => saved,
      setItem: (_, value) => {
        saved = value;
        writes++;
      },
    };
    const store = new SettingsStore(() => disk),
      g = await game();
    store.init(g);
    const wanted = 2 | (4 << 3) | 512 | 1024;
    assert.equal(g.restore_settings(wanted), 1);
    assert.equal(g.settings_snapshot(), wanted);
    for (const invalid of [0, 5, 1 | (11 << 3), 4099]) {
      assert.equal(g.restore_settings(invalid), 0);
      assert.equal(g.settings_snapshot(), wanted);
    }
    // Resume -> Game -> Color -> CRT -> Volume -> Left Direction.
    g.pause();
    for (let i = 0; i < 5; i++) g.advance(0, 0.2, 0.8, 8);
    g.advance(0, 0.2, 0.8, 64);
    const changed = wanted ^ 512;
    assert.equal(g.settings_snapshot(), changed);
    store.save(g);
    store.save(g);
    assert.equal(writes, 1);
    store.init(g);
    assert.equal(g.settings_snapshot(), changed);
    const reload = await game();
    new SettingsStore(() => disk).init(reload);
    assert.equal(reload.settings_snapshot(), changed);
    reload.advance(1 / 60, 0, 1, 64);
    assert.equal(reload.left_top(), 0);
    assert.equal(reload.right_top(), 0, "right inversion survives reload");
  },
);

test(
  "built WASM splash is immutable and leaves a paused match unchanged",
  options,
  async () => {
    const g = await game();
    g.advance(1 / 60, 0.3, 0.7, 64);
    g.pause();
    const frame = g.frame_number(),
      settings = g.settings_snapshot();
    const pixels = () =>
      Buffer.from(
        new Uint8Array(
          g.memory.buffer,
          g.render_splash(1),
          g.width() * g.height() * 4,
        ),
      );
    const first = pixels();
    for (let i = 0; i < 120; i++) assert.deepEqual(pixels(), first);
    assert.equal(g.frame_number(), frame);
    assert.equal(g.settings_snapshot(), settings);
    assert.equal(g.menu_open(), 1);
  },
);
