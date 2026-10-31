import test from "node:test";
import assert from "node:assert/strict";
import { HOLD_MS, SLOP_PX, TouchGestures } from "../public/touch.js";

class Canvas extends EventTarget {
  getBoundingClientRect() {
    return { left: 100, top: 50, width: 400, height: 300 };
  }
}
function harness({ enabled = true } = {}) {
  const canvas = new Canvas();
  const timers = {
    pending: new Map(),
    next: 1,
    setTimeout(callback, ms) {
      this.pending.set(this.next, { callback, ms });
      return this.next++;
    },
    clearTimeout(id) {
      this.pending.delete(id);
    },
    fire() {
      for (const [id, { callback }] of this.pending) {
        this.pending.delete(id);
        callback();
      }
    },
  };
  const seen = [];
  const state = { enabled };
  const gestures = new TouchGestures(canvas, {
    timers,
    enabled: () => state.enabled,
    onTap: (x, y) => seen.push(["tap", x, y]),
    onHold: () => seen.push(["hold"]),
    onTouch: () => seen.push(["touch"]),
  });
  const pointer = (type, x, y, values = {}) => {
    const event = Object.assign(new Event(type, { cancelable: true }), {
      pointerType: "touch",
      isPrimary: true,
      pointerId: 7,
      clientX: x,
      clientY: y,
      ...values,
    });
    canvas.dispatchEvent(event);
    return event;
  };
  return { canvas, timers, seen, state, gestures, pointer };
}

test("a short touch is a tap at its place on the canvas", () => {
  const { seen, pointer, timers } = harness();
  pointer("pointerdown", 300, 200);
  assert.equal([...timers.pending.values()][0].ms, HOLD_MS);
  pointer("pointermove", 305, 204);
  pointer("pointerup", 305, 204);
  assert.deepEqual(seen, [["touch"], ["tap", 0.5125, 0.5133333333333333]]);
  assert.equal(timers.pending.size, 0);
});

test("a held touch is a hold and not also a tap", () => {
  const { seen, pointer, timers } = harness();
  pointer("pointerdown", 300, 200);
  timers.fire();
  const menu = pointer("contextmenu", 300, 200);
  assert.ok(menu.defaultPrevented, "no browser menu on a long press");
  pointer("pointerup", 300, 200);
  assert.deepEqual(seen, [["touch"], ["hold"]]);
});

test("a dragged, cancelled or second touch does nothing", () => {
  const { seen, pointer, timers } = harness();
  pointer("pointerdown", 300, 200);
  pointer("pointermove", 300 + SLOP_PX + 1, 200);
  pointer("pointerup", 300 + SLOP_PX + 1, 200);
  pointer("pointerdown", 300, 200);
  pointer("pointercancel", 300, 200);
  pointer("pointerup", 300, 200);
  pointer("pointerdown", 300, 200, { isPrimary: false, pointerId: 8 });
  pointer("pointerup", 300, 200, { isPrimary: false, pointerId: 8 });
  assert.equal(timers.pending.size, 0);
  assert.deepEqual(seen, [["touch"], ["touch"]]);
});

test("mouse and pen input are left to the click handler", () => {
  const { seen, pointer, timers } = harness();
  for (const pointerType of ["mouse", "pen"]) {
    pointer("pointerdown", 300, 200, { pointerType });
    pointer("pointerup", 300, 200, { pointerType });
  }
  assert.equal(timers.pending.size, 0);
  assert.deepEqual(seen, []);
  assert.equal(pointer("contextmenu", 300, 200).defaultPrevented, false);
});

test("touches report touch input but act only while enabled", () => {
  const { seen, pointer, timers, state, gestures } = harness({
    enabled: false,
  });
  pointer("pointerdown", 300, 200);
  pointer("pointerup", 300, 200);
  assert.deepEqual(seen, [["touch"]]);
  state.enabled = true;
  pointer("pointerdown", 300, 200);
  state.enabled = false;
  timers.fire();
  pointer("pointerup", 300, 200);
  assert.deepEqual(seen, [["touch"], ["touch"]], "disabled before it is held");
  state.enabled = true;
  pointer("pointerdown", 300, 200);
  gestures.dispose();
  assert.equal(timers.pending.size, 0);
  pointer("pointerup", 300, 200);
  pointer("pointerdown", 300, 200);
  assert.deepEqual(seen, [["touch"], ["touch"], ["touch"]]);
});
