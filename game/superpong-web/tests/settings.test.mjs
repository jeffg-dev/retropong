import { test } from "node:test";
import assert from "node:assert/strict";
import {
  SettingsStore,
  SETTINGS_KEY,
  decodeSettings,
  encodeSettings,
} from "../public/settings.js";

function storage(initial = null) {
  let raw = initial,
    writes = 0;
  return {
    getItem: () => raw,
    setItem(key, value) {
      assert.equal(key, SETTINGS_KEY);
      raw = value;
      writes++;
    },
    get raw() {
      return raw;
    },
    get writes() {
      return writes;
    },
  };
}
function core() {
  let bits;
  return {
    init() {
      bits = 1987;
    },
    restore_settings(value) {
      bits = value;
    },
    settings_snapshot: () => bits,
  };
}
test("settings survive reload and replay and write only on changes", () => {
  const disk = storage(),
    g = core(),
    store = new SettingsStore(() => disk);
  store.init(g);
  store.save(g);
  assert.equal(disk.writes, 0);
  const bits = 4 | (3 << 3) | 512 | 1024;
  g.restore_settings(bits);
  store.save(g);
  store.save(g);
  assert.equal(disk.writes, 1);
  store.init(g);
  assert.equal(g.settings_snapshot(), bits);
  const reloaded = new SettingsStore(() => disk),
    next = core();
  reloaded.init(next);
  assert.equal(next.settings_snapshot(), bits);
  reloaded.save(next);
  assert.equal(disk.writes, 1);
});
test("corrupt, partial, wrong-version and out-of-range documents use defaults", () => {
  const valid = encodeSettings(1987);
  for (const raw of [
    "{",
    "null",
    "[]",
    "{}",
    JSON.stringify({ ...valid, version: 2 }),
    JSON.stringify({ ...valid, game: 0 }),
    JSON.stringify({ ...valid, game: 5 }),
    JSON.stringify({ ...valid, volume: 11 }),
    JSON.stringify({ ...valid, volume: 1.5 }),
    JSON.stringify({ ...valid, color: 1 }),
    JSON.stringify({ ...valid, invert_left: null }),
  ]) {
    const disk = storage(raw),
      g = core();
    new SettingsStore(() => disk).init(g);
    assert.equal(g.settings_snapshot(), 1987);
    assert.equal(disk.writes, 0);
  }
  assert.equal(decodeSettings(valid), 1987);
});
test("blocked storage reads, writes and missing storage do not interrupt play or replay", () => {
  for (const getStorage of [
    () => {
      throw Error("blocked");
    },
    () => undefined,
    () => ({
      getItem: () => null,
      setItem: () => {
        throw Error("quota");
      },
    }),
  ]) {
    const store = new SettingsStore(getStorage),
      g = core();
    store.init(g);
    g.restore_settings(643);
    store.save(g);
    store.init(g);
    assert.equal(g.settings_snapshot(), 643);
  }
});

test("new sessions use clockwise up and preserve saved counterclockwise preferences", () => {
  const fresh = core();
  new SettingsStore(() => storage()).init(fresh);
  assert.equal(encodeSettings(fresh.settings_snapshot()).invert_left, true);
  assert.equal(encodeSettings(fresh.settings_snapshot()).invert_right, true);
  for (const bits of [451, 451 | 512, 451 | 1024]) {
    const disk = storage(JSON.stringify(encodeSettings(bits)));
    const restored = core();
    new SettingsStore(() => disk).init(restored);
    assert.equal(restored.settings_snapshot(), bits);
    assert.equal(disk.writes, 0);
  }
});
