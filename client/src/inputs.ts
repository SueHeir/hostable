// Shapes of the values input widgets send, kept free of the DOM so they can
// be tested on their own. Each matches `Widget::accepts` in crates/protocol.

/** Longest text any input may hold (`MAX_TEXT_CHARS` in the protocol). */
export const MAX_TEXT_CHARS = 2000;

/** Cut `text` to at most `maxLen` characters (code points, not UTF-16 units). */
export function clampText(text: string, maxLen: number): string {
  const limit = Math.max(0, Math.min(Math.floor(maxLen), MAX_TEXT_CHARS));
  const chars = Array.from(text);
  return chars.length <= limit ? text : chars.slice(0, limit).join("");
}

// ---- choice and hand picks ----

/** Whether a choice or hand sends one value at once (`max` 1) or a list. */
export const isMulti = (max: number) => max > 1;

/** Tap on `value` in a multi-pick: add it if there is room, or remove it. */
export function togglePick(selected: readonly string[], value: string, max: number): string[] {
  if (selected.includes(value)) return selected.filter((v) => v !== value);
  if (selected.length >= max) return [...selected];
  return [...selected, value];
}

/** The Confirm button of a multi-pick is enabled only for min..max picks. */
export function canConfirm(selected: readonly string[], min: number, max: number): boolean {
  return selected.length >= min && selected.length <= max;
}

/** Picks that are still enabled options, in the order the options are shown. */
export function keepAllowed(selected: readonly string[], allowed: readonly string[]): string[] {
  return allowed.filter((v) => selected.includes(v));
}

// ---- touchpad ----

export type TouchValue = { dx: number; dy: number } | { tap: true };

/** At most 20 moves a second. */
export const TOUCH_INTERVAL_MS = 50;
const TAP_MAX_MS = 300;
const TAP_MAX_TRAVEL = 0.03;

const clampUnit = (x: number) => Math.max(-1, Math.min(1, Math.round(x * 1000) / 1000));

/**
 * Turns pointer positions on a pad into `{dx, dy}` moves (fractions of the
 * pad, -1..1, at most one per {@link TOUCH_INTERVAL_MS}) and quick touches
 * that barely move into `{tap: true}`.
 */
export class TouchTracker {
  private last: { x: number; y: number } | null = null;
  private w = 1;
  private h = 1;
  private accX = 0;
  private accY = 0;
  private travel = 0;
  private startedAt = 0;
  private lastSentAt = -Infinity;
  private movedSent = false;

  constructor(
    private readonly emit: (value: TouchValue) => void,
    private readonly now: () => number = () => Date.now(),
  ) {}

  down(x: number, y: number, width: number, height: number): void {
    this.last = { x, y };
    this.w = width > 0 ? width : 1;
    this.h = height > 0 ? height : 1;
    this.accX = this.accY = this.travel = 0;
    this.movedSent = false;
    this.startedAt = this.now();
  }

  move(x: number, y: number): void {
    if (!this.last) return;
    const dx = (x - this.last.x) / this.w;
    const dy = (y - this.last.y) / this.h;
    this.last = { x, y };
    if (!Number.isFinite(dx) || !Number.isFinite(dy)) return;
    this.accX += dx;
    this.accY += dy;
    this.travel += Math.hypot(dx, dy);
    if (this.now() - this.lastSentAt >= TOUCH_INTERVAL_MS) this.flush();
  }

  up(x: number, y: number): void {
    if (!this.last) return;
    this.move(x, y);
    const quick = this.now() - this.startedAt <= TAP_MAX_MS;
    if (!this.movedSent && quick && this.travel <= TAP_MAX_TRAVEL) {
      this.emit({ tap: true });
    } else {
      this.flush();
    }
    this.last = null;
  }

  cancel(): void {
    this.last = null;
  }

  private flush(): void {
    const dx = clampUnit(this.accX);
    const dy = clampUnit(this.accY);
    if (dx === 0 && dy === 0) return;
    if (this.travel <= TAP_MAX_TRAVEL && !this.movedSent) return;
    this.accX = this.accY = 0;
    this.lastSentAt = this.now();
    this.movedSent = true;
    this.emit({ dx, dy });
  }
}

// ---- drawing ----

export const MAX_STROKES = 64;
export const MAX_STROKE_POINTS = 512;
/**
 * All strokes together, so the message stays under the relay's 16 KB cap
 * (each point is about 14 bytes of JSON at 3 decimals).
 */
export const MAX_TOTAL_POINTS = 1000;
/** Points closer than this (a fraction of the canvas) to the last one are skipped. */
const MIN_STEP = 0.004;

export type Point = [number, number];

const unit = (v: number) => (Number.isFinite(v) ? Math.max(0, Math.min(1, Math.round(v * 1000) / 1000)) : 0);

/** Collects strokes in 0..1 canvas coordinates within the protocol's caps. */
export class StrokeRecorder {
  strokes: Point[][] = [];
  private drawing = false;

  get totalPoints(): number {
    return this.strokes.reduce((n, s) => n + s.length, 0);
  }

  get full(): boolean {
    return this.strokes.length >= MAX_STROKES || this.totalPoints >= MAX_TOTAL_POINTS;
  }

  /** Start a stroke at a normalised point; ignored once the caps are reached. */
  begin(x: number, y: number): boolean {
    if (this.full) return false;
    this.strokes.push([[unit(x), unit(y)]]);
    this.drawing = true;
    return true;
  }

  add(x: number, y: number): boolean {
    if (!this.drawing) return false;
    const stroke = this.strokes[this.strokes.length - 1]!;
    if (stroke.length >= MAX_STROKE_POINTS || this.totalPoints >= MAX_TOTAL_POINTS) return false;
    const p: Point = [unit(x), unit(y)];
    const last = stroke[stroke.length - 1]!;
    if (Math.hypot(p[0] - last[0], p[1] - last[1]) < MIN_STEP) return false;
    stroke.push(p);
    return true;
  }

  end(): void {
    this.drawing = false;
  }

  undo(): void {
    this.drawing = false;
    this.strokes.pop();
  }

  clear(): void {
    this.drawing = false;
    this.strokes = [];
  }

  /** The value a draw widget sends: `[[[x, y], ...], ...]`. */
  toValue(): Point[][] {
    return this.strokes.map((s) => s.map(([x, y]) => [x, y] as Point));
  }
}
