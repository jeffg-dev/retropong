import assert from "node:assert/strict";
import { after, test } from "node:test";
import { AudioPlayer } from "../public/audio.js";

const originalWindow = globalThis.window;
const originalFetch = globalThis.fetch;
class MockAudioContext {
  state = "suspended";
  destination = {};
  sources = [];
  createGain() {
    return { gain: { value: 0 }, connect() {} };
  }
  async decodeAudioData(data) {
    return data;
  }
  async resume() {
    this.state = "running";
  }
  createBufferSource() {
    const source = {
      started: false,
      stopped: false,
      connect() {},
      disconnect() {},
      start() {
        this.started = true;
      },
      stop() {
        this.stopped = true;
      },
    };
    this.sources.push(source);
    return source;
  }
}
globalThis.window = { AudioContext: MockAudioContext };
globalThis.fetch = async (path) => ({
  ok: true,
  arrayBuffer: async () => ({ path }),
});
after(() => {
  globalThis.window = originalWindow;
  globalThis.fetch = originalFetch;
});

test("volume uses the desktop squared gain curve, including initial volume", async () => {
  const player = new AudioPlayer();
  await player.load();
  assert.ok(Math.abs(player.gain.gain.value - 0.64) < 1e-12);
  player.volume = 5;
  assert.equal(player.gain.gain.value, 0.25);
  player.volume = 20;
  assert.equal(player.volume, 10);
  assert.equal(player.gain.gain.value, 1);
  player.volume = -1;
  assert.equal(player.gain.gain.value, 0);
});

test("play waits for audio unlock and each sound replaces the previous channel", async () => {
  const player = new AudioPlayer();
  await player.load();
  player.play(1);
  assert.equal(player.context.sources.length, 0);
  await player.unlock();
  player.play(1);
  const bounce = player.context.sources[0];
  assert.equal(bounce.started, true);
  assert.equal(bounce.buffer.path, "./sounds/bounce.wav");
  player.play(2 | 4 | 8);
  assert.equal(bounce.stopped, true);
  assert.equal(player.sources.size, 1);
  assert.deepEqual(
    player.context.sources.map((source) => source.stopped),
    [true, true, true, false],
  );
  assert.equal([...player.sources][0].buffer.path, "./sounds/attract_tick.wav");
  player.stop();
  assert.equal(player.sources.size, 0);
  assert.equal(player.context.sources.at(-1).stopped, true);
});

test("missing later sounds do not silence the last available sound in an event mask", async () => {
  const player = new AudioPlayer();
  await player.load();
  await player.unlock();
  player.buffers[3] = null;
  player.play(4 | 8);
  assert.equal(player.sources.size, 1);
  assert.equal([...player.sources][0].buffer.path, "./sounds/serve.wav");
});
