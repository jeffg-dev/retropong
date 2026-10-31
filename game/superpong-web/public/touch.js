// Taps and holds on the game canvas from a touchscreen. Mouse and pen input
// keep the page's click handling. A tap reports where it landed as fractions
// of the canvas; the core decides what is drawn there.
export const HOLD_MS = 500;
// A touch that travels further than this (CSS pixels) is neither.
export const SLOP_PX = 16;

export class TouchGestures {
  constructor(
    target,
    {
      onTap = () => {},
      onHold = () => {},
      onTouch = () => {},
      enabled = () => true,
      timers = globalThis,
    } = {},
  ) {
    this.target = target;
    this.onTap = onTap;
    this.onHold = onHold;
    this.onTouch = onTouch;
    this.enabled = enabled;
    this.timers = timers;
    this.touch = null;
    this.onDown = (event) => {
      if (event.pointerType !== "touch" || !event.isPrimary) return;
      this.onTouch();
      this.cancel();
      if (!this.enabled()) return;
      const touch = {
        id: event.pointerId,
        x: event.clientX,
        y: event.clientY,
        held: false,
      };
      touch.timer = this.timers.setTimeout(() => {
        touch.held = true;
        if (this.enabled()) this.onHold();
      }, HOLD_MS);
      this.touch = touch;
    };
    this.onMove = (event) => {
      const touch = this.touch;
      if (!touch || event.pointerId !== touch.id || touch.held) return;
      if (
        Math.hypot(event.clientX - touch.x, event.clientY - touch.y) > SLOP_PX
      )
        this.cancel();
    };
    this.onUp = (event) => {
      const touch = this.touch;
      if (!touch || event.pointerId !== touch.id) return;
      this.cancel();
      if (touch.held || !this.enabled()) return;
      const box = this.target.getBoundingClientRect();
      if (box.width > 0 && box.height > 0)
        this.onTap(
          (event.clientX - box.left) / box.width,
          (event.clientY - box.top) / box.height,
        );
    };
    this.onCancel = (event) => {
      if (this.touch && event.pointerId === this.touch.id) this.cancel();
    };
    // A long press would otherwise open the browser's menu for the canvas.
    this.onContextMenu = (event) => {
      if (this.touch) event.preventDefault();
    };
    target.addEventListener("pointerdown", this.onDown);
    target.addEventListener("pointermove", this.onMove);
    target.addEventListener("pointerup", this.onUp);
    target.addEventListener("pointercancel", this.onCancel);
    target.addEventListener("contextmenu", this.onContextMenu);
  }

  cancel() {
    if (this.touch) this.timers.clearTimeout(this.touch.timer);
    this.touch = null;
  }

  dispose() {
    this.cancel();
    this.target.removeEventListener("pointerdown", this.onDown);
    this.target.removeEventListener("pointermove", this.onMove);
    this.target.removeEventListener("pointerup", this.onUp);
    this.target.removeEventListener("pointercancel", this.onCancel);
    this.target.removeEventListener("contextmenu", this.onContextMenu);
  }
}
