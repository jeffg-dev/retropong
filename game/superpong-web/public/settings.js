// Browser storage is optional. The native menu remains the source of truth.
export const SETTINGS_KEY = "retropong.settings";
export function decodeSettings(value) {
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    value.version !== 1 ||
    !Number.isInteger(value.game) ||
    value.game < 1 ||
    value.game > 4 ||
    !Number.isInteger(value.volume) ||
    value.volume < 0 ||
    value.volume > 10 ||
    !["color", "crt", "invert_left", "invert_right"].every(
      (key) => typeof value[key] === "boolean",
    )
  )
    return null;
  return (
    value.game |
    (value.volume << 3) |
    (+value.color << 7) |
    (+value.crt << 8) |
    (+value.invert_left << 9) |
    (+value.invert_right << 10)
  );
}
export function encodeSettings(bits) {
  return {
    version: 1,
    game: bits & 7,
    volume: (bits >> 3) & 15,
    color: Boolean(bits & 128),
    crt: Boolean(bits & 256),
    invert_left: Boolean(bits & 512),
    invert_right: Boolean(bits & 1024),
  };
}
export class SettingsStore {
  constructor(getStorage = () => globalThis.localStorage) {
    this.getStorage = getStorage;
    this.saved = null;
    try {
      this.saved = decodeSettings(
        JSON.parse(getStorage().getItem(SETTINGS_KEY)),
      );
    } catch {}
  }
  init(core) {
    core.init(3);
    if (this.saved !== null) core.restore_settings(this.saved);
    // Establish a baseline without writing defaults on every page load.
    this.saved = core.settings_snapshot();
  }
  save(core) {
    const bits = core.settings_snapshot();
    if (bits === this.saved) return;
    this.saved = bits;
    try {
      this.getStorage().setItem(
        SETTINGS_KEY,
        JSON.stringify(encodeSettings(bits)),
      );
    } catch {}
  }
}
