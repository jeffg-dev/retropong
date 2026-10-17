const SOUNDS = ["bounce", "point", "serve", "attract_tick"];

export class AudioPlayer {
  constructor() {
    this.context = null;
    this.gain = null;
    this.buffers = [];
    this.sources = new Set();
    this._volume = 8;
  }

  get volume() {
    return this._volume;
  }
  set volume(value) {
    const number = Number(value);
    this._volume = Number.isFinite(number)
      ? Math.max(0, Math.min(10, number))
      : 8;
    if (this.gain) this.gain.gain.value = (this._volume / 10) ** 2;
  }

  async load() {
    const AudioContext = window.AudioContext || window.webkitAudioContext;
    if (!AudioContext) return;
    if (!this.context) {
      this.context = new AudioContext();
      this.gain = this.context.createGain();
      this.gain.gain.value = (this._volume / 10) ** 2;
      this.gain.connect(this.context.destination);
    }
    this.buffers = await Promise.all(
      SOUNDS.map(async (name) => {
        try {
          const response = await fetch(`./sounds/${name}.wav`);
          if (!response.ok)
            throw new Error(`Sound request failed (${response.status})`);
          return await this.context.decodeAudioData(
            await response.arrayBuffer(),
          );
        } catch (error) {
          console.warn(`Sound ${name} unavailable.`, error);
          return null;
        }
      }),
    );
  }

  async unlock() {
    if (this.context?.state === "suspended") {
      try {
        await this.context.resume();
      } catch {
        /* Autoplay policy keeps audio muted. */
      }
    }
  }

  play(bitmask) {
    if (!this.context || this.context.state !== "running" || !this._volume)
      return;
    for (let index = 0; index < this.buffers.length; index++) {
      if (!(bitmask & (1 << index)) || !this.buffers[index]) continue;
      // Match the native single audio channel: later events replace earlier ones.
      this.stop();
      const source = this.context.createBufferSource();
      source.buffer = this.buffers[index];
      source.connect(this.gain);
      source.onended = () => {
        this.sources.delete(source);
        source.disconnect();
      };
      this.sources.add(source);
      source.start();
    }
  }

  stop() {
    for (const source of this.sources) {
      try {
        source.stop();
      } catch {
        /* A sound may already have finished. */
      }
      source.disconnect();
    }
    this.sources.clear();
  }
}
