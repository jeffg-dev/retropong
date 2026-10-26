const VID = 0x1209;
const PID = 0x0001;
const clamp = (value, low, high) => Math.min(high, Math.max(low, value));

export function axisPosition(value) {
  return Number.isFinite(value) ? (clamp(value, -1, 1) + 1) / 2 : 0.5;
}

export function decodeHidReport(data) {
  if (!data || data.byteLength < 5) return null;
  const view =
    data instanceof DataView
      ? data
      : new DataView(data.buffer, data.byteOffset, data.byteLength);
  return {
    left: clamp(view.getUint16(0, true), 0, 4095) / 4095,
    right: clamp(view.getUint16(2, true), 0, 4095) / 4095,
    buttons: view.getUint8(4),
  };
}

const isRetroController = (pad) => /retro.?controller|1209.*0001/i.test(pad.id);

export function selectGamepad(gamepads) {
  const candidates = Array.from(gamepads || []).filter(
    (pad) => pad && pad.connected !== false && pad.axes?.length >= 2,
  );
  return candidates.find(isRetroController) || candidates[0] || null;
}

// Core advance() command bits for newly pressed gamepad buttons. South and
// East (a RetroController's left/right buttons) go to the core as select and
// next, which the menu maps as each press arrives.
export function padCommands(pressed) {
  let commands = 0;
  if (pressed & (1 << 9)) commands |= 1;
  if (pressed & (1 << 12)) commands |= 4;
  if (pressed & (1 << 13)) commands |= 8;
  if (pressed & (1 << 14)) commands |= 16;
  if (pressed & (1 << 15)) commands |= 32;
  if (pressed & 1) commands |= 128;
  if (pressed & 2) commands |= 256;
  return commands;
}

export class ControllerInput {
  constructor({
    onConnect = () => {},
    onDisconnect = () => {},
    onError = () => {},
    navigator: nav = globalThis.navigator,
    eventTarget = globalThis.window,
  } = {}) {
    this.nav = nav;
    this.eventTarget = eventTarget;
    this.onConnect = onConnect;
    this.onDisconnect = onDisconnect;
    this.onError = onError;
    this.device = null;
    this.hidState = { left: 0.5, right: 0.5, buttons: 0 };
    this.activeId = null;
    this.disposed = false;
    this.initialized = false;
    this.generation = 0;
    this.pending = new Map();
    this.onGamepadEvent = () => this.poll();
    this.onHidConnect = (event) => {
      if (this.isController(event.device) && !this.device)
        void this.openDevice(event.device);
    };
    this.onHidDisconnect = (event) => {
      // Invalidate an open operation even if it has not completed yet.
      this.pending.delete(event.device);
      if (this.device === event.device) {
        this.generation++;
        this.device.removeEventListener("inputreport", this.onReport);
        this.device = null;
        this.poll();
      }
    };
    this.onReport = (event) => {
      if (event.device && event.device !== this.device) return;
      if (event.reportId !== undefined && event.reportId !== 0) return;
      const state = decodeHidReport(event.data);
      if (state) this.hidState = state;
    };
  }

  isController(device) {
    return device?.vendorId === VID && device?.productId === PID;
  }

  async init() {
    if (this.initialized || this.disposed) return;
    this.initialized = true;
    this.eventTarget?.addEventListener("gamepadconnected", this.onGamepadEvent);
    this.eventTarget?.addEventListener(
      "gamepaddisconnected",
      this.onGamepadEvent,
    );
    this.nav?.hid?.addEventListener("connect", this.onHidConnect);
    this.nav?.hid?.addEventListener("disconnect", this.onHidDisconnect);
    this.poll();
    try {
      const devices = (await this.nav?.hid?.getDevices?.()) || [];
      const device = devices.find((device) => this.isController(device));
      if (device && !this.disposed) await this.openDevice(device);
    } catch (error) {
      if (!this.disposed) this.onError(error);
    }
  }

  async requestDevice() {
    if (this.disposed) return false;
    if (!this.nav?.hid?.requestDevice) {
      this.onError(
        new Error(
          "Direct USB access requires a browser with WebHID support, such as Chrome or Edge.",
        ),
      );
      return false;
    }
    try {
      const devices = await this.nav.hid.requestDevice({
        filters: [{ vendorId: VID, productId: PID }],
      });
      if (!devices.length || this.disposed) return false;
      return await this.openDevice(devices[0]);
    } catch (error) {
      if (!this.disposed && error.name !== "NotFoundError") this.onError(error);
      return false;
    }
  }

  async openDevice(device) {
    if (this.disposed) return false;
    if (this.device === device) return true;
    if (this.pending.has(device)) return this.pending.get(device);
    const generation = this.generation;
    const operation = (async () => {
      try {
        await Promise.resolve();
        if (!device.opened) await device.open();
        if (
          this.disposed ||
          generation !== this.generation ||
          !this.pending.has(device)
        ) {
          if (device.opened) await device.close().catch(() => {});
          return false;
        }
        if (this.device && this.device !== device) {
          this.device.removeEventListener("inputreport", this.onReport);
          void this.device.close().catch(() => {});
        }
        this.device = device;
        this.hidState = { left: 0.5, right: 0.5, buttons: 0 };
        device.addEventListener("inputreport", this.onReport);
        this.poll();
        return true;
      } catch (error) {
        if (!this.disposed) this.onError(error);
        return false;
      } finally {
        this.pending.delete(device);
      }
    })();
    this.pending.set(device, operation);
    return operation;
  }

  poll() {
    if (this.disposed) return null;
    let pad = null;
    try {
      pad = selectGamepad(this.nav?.getGamepads?.());
    } catch {
      // Gamepad access can be unavailable in an embedded or restricted context.
    }
    const state = pad
      ? {
          left: axisPosition(pad.axes[0]),
          right: axisPosition(pad.axes[1]),
          id: `gamepad:${pad.index ?? 0}:${pad.id}`,
          buttons: Array.from(pad.buttons || []).reduce(
            (bits, button, index) =>
              index < 31 && (button.pressed || button.value > 0.5)
                ? bits | (1 << index)
                : bits,
            0,
          ),
        }
      : this.device
        ? {
            ...this.hidState,
            id: `hid:${this.device.productName || "RetroController"}`,
          }
        : null;
    if (state && this.activeId !== state.id) {
      this.activeId = state.id;
      this.onConnect(state);
    } else if (!state && this.activeId !== null) {
      this.activeId = null;
      this.onDisconnect();
    }
    return state;
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.generation++;
    this.eventTarget?.removeEventListener(
      "gamepadconnected",
      this.onGamepadEvent,
    );
    this.eventTarget?.removeEventListener(
      "gamepaddisconnected",
      this.onGamepadEvent,
    );
    this.nav?.hid?.removeEventListener("connect", this.onHidConnect);
    this.nav?.hid?.removeEventListener("disconnect", this.onHidDisconnect);
    if (this.device) {
      this.device.removeEventListener("inputreport", this.onReport);
      void this.device.close().catch(() => {});
      this.device = null;
    }
    this.activeId = null;
  }
}
