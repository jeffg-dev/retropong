import test from "node:test";
import assert from "node:assert/strict";
import {
  axisPosition,
  decodeHidReport,
  selectGamepad,
  ControllerInput,
} from "../public/controller.js";

class Device extends EventTarget {
  vendorId = 0x1209;
  productId = 1;
  productName = "RetroController";
  opened = false;
  async open() {
    this.opened = true;
  }
  async close() {
    this.opened = false;
  }
}
function event(target, type, values) {
  target.dispatchEvent(Object.assign(new Event(type), values));
}
function harness(devices = []) {
  const hid = new EventTarget();
  hid.getDevices = async () => devices;
  hid.requestDevice = async () => devices;
  const connected = [],
    errors = [];
  let disconnected = 0;
  const nav = { hid, getGamepads: () => [] };
  const input = new ControllerInput({
    navigator: nav,
    eventTarget: new EventTarget(),
    onConnect: (state) => connected.push(state),
    onDisconnect: () => disconnected++,
    onError: (error) => errors.push(error),
  });
  return {
    input,
    hid,
    nav,
    connected,
    errors,
    get disconnected() {
      return disconnected;
    },
  };
}

test("axes preserve direction and full travel without dead zones", () => {
  assert.deepEqual(
    [-1, -0.02, 0, 0.02, 1].map(axisPosition),
    [0, 0.49, 0.5, 0.51, 1],
  );
  assert.equal(axisPosition(NaN), 0.5);
});
test("HID little endian report maps two 12-bit axes and button bits", () => {
  assert.deepEqual(decodeHidReport(Uint8Array.of(0, 0, 255, 15, 3)), {
    left: 0,
    right: 1,
    buttons: 3,
  });
  const bytes = Uint8Array.of(99, 255, 15, 0, 0, 2, 99);
  assert.deepEqual(decodeHidReport(bytes.subarray(1, 6)), {
    left: 1,
    right: 0,
    buttons: 2,
  });
  assert.equal(decodeHidReport(new DataView(new ArrayBuffer(4))), null);
});
test("gamepad prefers RetroController and ignores disconnected devices", () => {
  const generic = { id: "Xbox", axes: [0, 0] };
  const retro = { id: "1209-0001-RetroController", axes: [-1, 1] };
  assert.equal(selectGamepad([null, generic, retro]), retro);
  assert.equal(
    selectGamepad([{ ...retro, connected: false }, generic]),
    generic,
  );
});
test("already attached gamepad connects on init and disconnects exactly once", async () => {
  const h = harness();
  h.nav.getGamepads = () => [
    { id: "RetroController", axes: [-1, 1], buttons: [{ pressed: true }] },
  ];
  await h.input.init();
  assert.deepEqual(h.input.poll(), {
    id: "gamepad:0:RetroController",
    left: 0,
    right: 1,
    buttons: 1,
  });
  assert.equal(h.connected.length, 1);
  h.nav.getGamepads = () => [];
  h.input.poll();
  h.input.poll();
  assert.equal(h.disconnected, 1);
  h.input.dispose();
});
test("permitted HID remains connected before first report and resumes after gamepad loss", async () => {
  const device = new Device();
  const h = harness([device]);
  await h.input.init();
  assert.equal(h.input.poll().left, 0.5);
  event(device, "inputreport", {
    device,
    reportId: 0,
    data: new DataView(Uint8Array.of(0, 0, 255, 15, 1).buffer),
  });
  assert.equal(h.input.poll().right, 1);
  h.nav.getGamepads = () => [{ id: "RetroController", axes: [1, -1] }];
  assert.equal(h.input.poll().left, 1);
  h.nav.getGamepads = () => [];
  assert.equal(h.input.poll().left, 0);
  assert.equal(h.disconnected, 0);
  event(h.hid, "disconnect", { device });
  assert.equal(h.input.poll(), null);
  assert.equal(h.disconnected, 1);
  h.input.dispose();
});
test("already opened HID and permission chooser work", async () => {
  const device = new Device();
  device.opened = true;
  const h = harness([device]);
  assert.equal(await h.input.requestDevice(), true);
  assert.equal(h.input.poll().id, "hid:RetroController");
  h.input.dispose();
  assert.equal(h.input.poll(), null);
});
test("disconnect while opening cannot install a stale device", async () => {
  const device = new Device();
  let finish;
  device.open = () =>
    new Promise((resolve) => {
      finish = () => {
        device.opened = true;
        resolve();
      };
    });
  const h = harness([device]);
  const pending = h.input.init();
  await new Promise((resolve) => setImmediate(resolve));
  event(h.hid, "disconnect", { device });
  finish();
  await pending;
  assert.equal(h.input.poll(), null);
  assert.equal(device.opened, false);
  h.input.dispose();
});
test("denied HID permission is reported without throwing", async () => {
  const h = harness();
  h.hid.requestDevice = async () => {
    throw new Error("Permission denied");
  };
  assert.equal(await h.input.requestDevice(), false);
  assert.equal(h.errors.length, 1);
  h.input.dispose();
});
