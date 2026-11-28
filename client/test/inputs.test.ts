import { describe, test } from "node:test";
import assert from "node:assert/strict";
import {
  canConfirm,
  clampText,
  keepAllowed,
  MAX_STROKE_POINTS,
  MAX_STROKES,
  StrokeRecorder,
  togglePick,
  TouchTracker,
  type TouchValue,
} from "../src/inputs";
import { accepts } from "./accepts";

describe("multi-pick rules", () => {
  test("toggle adds up to max and removes on a second tap", () => {
    let s: string[] = [];
    s = togglePick(s, "a", 2);
    s = togglePick(s, "b", 2);
    assert.deepEqual(s, ["a", "b"]);
    assert.deepEqual(togglePick(s, "c", 2), ["a", "b"]);
    assert.deepEqual(togglePick(s, "a", 2), ["b"]);
  });

  test("confirm needs min..max picks", () => {
    assert.equal(canConfirm([], 1, 3), false);
    assert.equal(canConfirm(["a"], 1, 3), true);
    assert.equal(canConfirm(["a", "b", "c"], 1, 3), true);
    assert.equal(canConfirm(["a", "b", "c", "d"], 1, 3), false);
    assert.equal(canConfirm([], 0, 2), true);
    assert.equal(canConfirm(["a"], 2, 2), false);
  });

  test("picks of options that went away or got disabled are dropped", () => {
    assert.deepEqual(keepAllowed(["c", "a", "gone"], ["a", "b", "c"]), ["a", "c"]);
  });
});

describe("text", () => {
  test("clamps by characters, not UTF-16 units", () => {
    assert.equal(clampText("héllo wörld", 5), "héllo");
    assert.equal(clampText("😀😀😀", 2), "😀😀");
    assert.equal(clampText("ok", 10), "ok");
    assert.equal(Array.from(clampText("x".repeat(5000), 99999)).length, 2000);
  });
});

describe("touchpad tracker", () => {
  function tracker() {
    let t = 0;
    const out: TouchValue[] = [];
    const tr = new TouchTracker((v) => out.push(v), () => t);
    return { tr, out, at: (ms: number) => (t = ms) };
  }

  test("a quick still touch is a tap", () => {
    const { tr, out, at } = tracker();
    tr.down(100, 100, 200, 200);
    at(120);
    tr.up(101, 100);
    assert.deepEqual(out, [{ tap: true }]);
  });

  test("a long press without movement sends nothing", () => {
    const { tr, out, at } = tracker();
    tr.down(100, 100, 200, 200);
    at(900);
    tr.up(100, 100);
    assert.deepEqual(out, []);
  });

  test("moves are fractions of the pad, clamped, at most 20 a second", () => {
    const { tr, out, at } = tracker();
    tr.down(0, 0, 200, 100);
    at(10);
    tr.move(50, 10); // first move goes out at once
    at(20);
    tr.move(70, 10); // throttled, accumulated
    at(40);
    tr.move(90, 30); // still within 50 ms
    at(60);
    tr.move(100, 30); // flushes everything since the last send
    at(70);
    tr.move(900, -900); // big jump: clamped, but throttled
    at(80);
    tr.up(900, -900);
    assert.deepEqual(out, [
      { dx: 0.25, dy: 0.1 },
      { dx: 0.25, dy: 0.2 },
      { dx: 1, dy: -1 },
    ]);
    const ui = { type: "touchpad", id: "t" } as const;
    for (const v of out) assert.ok(accepts(ui, { widget: "t", value: v }));
  });

  test("zero-size pads never produce NaN", () => {
    const { tr, out, at } = tracker();
    tr.down(0, 0, 0, 0);
    at(100);
    tr.move(0.5, 0.25);
    tr.up(0.5, 0.25);
    assert.deepEqual(out, [{ dx: 0.5, dy: 0.25 }]);
  });
});

describe("stroke recorder", () => {
  test("clamps and rounds points into 0..1", () => {
    const r = new StrokeRecorder();
    r.begin(-0.5, 0.123456);
    r.add(2, 0.5);
    r.end();
    assert.deepEqual(r.toValue(), [[[0, 0.123], [1, 0.5]]]);
  });

  test("skips points that barely move, and undo/clear work", () => {
    const r = new StrokeRecorder();
    r.begin(0.5, 0.5);
    assert.equal(r.add(0.5001, 0.5), false);
    r.end();
    r.begin(0.1, 0.1);
    r.end();
    r.undo();
    assert.equal(r.strokes.length, 1);
    r.clear();
    assert.deepEqual(r.toValue(), []);
  });

  test("caps strokes, points per stroke, and total size under 16 KB", () => {
    const r = new StrokeRecorder();
    for (let s = 0; s < 100; s++) {
      r.begin(Math.random(), Math.random());
      for (let p = 0; p < 700; p++) r.add(Math.random(), Math.random());
      r.end();
    }
    const v = r.toValue();
    assert.ok(v.length <= MAX_STROKES);
    assert.ok(v.every((s) => s.length <= MAX_STROKE_POINTS));
    const msg = JSON.stringify({ type: "input", widget: "art", value: v });
    assert.ok(msg.length < 16 * 1024, `message is ${msg.length} bytes`);
    assert.ok(accepts({ type: "draw", id: "art" }, { widget: "art", value: v }));

    const many = new StrokeRecorder();
    for (let s = 0; s < 100; s++) {
      many.begin(0.5, 0.5);
      many.end();
    }
    assert.equal(many.toValue().length, MAX_STROKES);
  });
});
