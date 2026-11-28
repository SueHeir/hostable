import { test } from "node:test";
import assert from "node:assert/strict";
import { drawScene, fitScene, hexCorners, type Ctx2D } from "../src/table/scene";
import type { Scene } from "../src/protocol/Scene";

type Call = [string, ...unknown[]];

/** A 2D context that records what is drawn, including style at fill/stroke time. */
function fakeCtx() {
  const calls: Call[] = [];
  const ctx: Ctx2D & { calls: Call[] } = {
    calls,
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 1,
    font: "",
    textAlign: "",
    textBaseline: "",
    setTransform: (...a) => void calls.push(["setTransform", ...a]),
    clearRect: (...a) => void calls.push(["clearRect", ...a]),
    fillRect: (...a) => void calls.push(["fillRect", ctx.fillStyle, ...a]),
    strokeRect: (...a) => void calls.push(["strokeRect", ...a]),
    beginPath: () => void calls.push(["beginPath"]),
    closePath: () => void calls.push(["closePath"]),
    moveTo: (...a) => void calls.push(["moveTo", ...a]),
    lineTo: (...a) => void calls.push(["lineTo", ...a]),
    arc: (...a) => void calls.push(["arc", ...a]),
    fill: () => void calls.push(["fill", ctx.fillStyle]),
    stroke: () => void calls.push(["stroke", ctx.strokeStyle, ctx.lineWidth]),
    fillText: (...a) => void calls.push(["fillText", ctx.fillStyle, ctx.font, ...a]),
  };
  return ctx;
}

const near = (a: number, b: number) => Math.abs(a - b) < 1e-9;

test("scale to fit keeps the aspect ratio and centres", () => {
  assert.deepEqual(fitScene({ width: 100, height: 50 }, 400, 400), { scale: 4, offsetX: 0, offsetY: 100 });
  assert.deepEqual(fitScene({ width: 100, height: 200 }, 300, 200), { scale: 1, offsetX: 100, offsetY: 0 });
  const zero = fitScene({ width: 0, height: 0 }, 50, 50);
  assert.ok(Number.isFinite(zero.scale));
});

test("pointy-top hex corners start at the top", () => {
  const c = hexCorners(10, 10, 5);
  assert.equal(c.length, 6);
  assert.ok(near(c[0]![0], 10) && near(c[0]![1], 5));
  assert.ok(near(c[3]![0], 10) && near(c[3]![1], 15));
  assert.ok(near(c[1]![1], 7.5));
});

test("draws every shape kind with sanitised colours under a fit transform", () => {
  const scene: Scene = {
    width: 200,
    height: 100,
    background: "#102030",
    shapes: [
      { kind: "hex", x: 50, y: 50, r: 20, fill: "#f3a712", stroke: "#000", label: "8" },
      { kind: "circle", x: 120, y: 50, r: 10, fill: "javascript:alert(1)", stroke: "", label: "" },
      { kind: "rect", x: 150, y: 10, w: 40, h: 30, fill: "green", stroke: "white", label: "Road" },
      { kind: "line", x1: 0, y1: 0, x2: 200, y2: 100, color: "red", width: 3 },
      { kind: "text", x: 100, y: 90, text: "Hello", size: 12, color: "url(x)" },
      { kind: "circle", x: Number.NaN, y: 0, r: 5, fill: "red", stroke: "", label: "" },
    ],
  };
  const ctx = fakeCtx();
  const fit = drawScene(ctx, scene, 400, 400, 2);
  assert.deepEqual(fit, { scale: 2, offsetX: 0, offsetY: 100 });
  const c = ctx.calls;
  assert.deepEqual(c[0], ["setTransform", 2, 0, 0, 2, 0, 0]);
  assert.deepEqual(c[1], ["clearRect", 0, 0, 400, 400]);
  assert.deepEqual(c[2], ["setTransform", 4, 0, 0, 4, 0, 200]);
  assert.deepEqual(c[3], ["fillRect", "#102030", 0, 0, 200, 100]);

  const moves = c.filter((x) => x[0] === "moveTo");
  const lines = c.filter((x) => x[0] === "lineTo");
  // Hex: 1 move + 5 lines; rect: 1 move + 3 lines; line: 1 move + 1 line.
  assert.equal(moves.length, 3);
  assert.equal(lines.length, 9);
  assert.deepEqual(moves[0]!.slice(1).map((v) => Math.round(v as number)), [50, 30]);

  const fills = c.filter((x) => x[0] === "fill").map((x) => x[1]);
  assert.deepEqual(fills, ["#f3a712", "#888888", "green"]);
  const arcs = c.filter((x) => x[0] === "arc");
  assert.equal(arcs.length, 1, "the NaN circle is skipped");
  assert.deepEqual(arcs[0]!.slice(1, 4), [120, 50, 10]);

  const strokes = c.filter((x) => x[0] === "stroke").map((x) => x.slice(1));
  assert.deepEqual(strokes[0], ["#000", 0.75]);
  assert.deepEqual(strokes[1], ["white", 0.75]);
  assert.deepEqual(strokes[2], ["red", 3]);
  assert.equal(strokes.length, 3, "an empty stroke draws no outline");

  const texts = c.filter((x) => x[0] === "fillText");
  assert.deepEqual(texts.map((t) => t[3]), ["8", "Road", "Hello"]);
  assert.equal(texts[2]![1], "#888888", "a bad text colour falls back");
  assert.match(texts[2]![2] as string, /^12\.00px/);
});

test("an empty background paints nothing behind the shapes", () => {
  const ctx = fakeCtx();
  drawScene(ctx, { width: 10, height: 10, background: "", shapes: [] }, 10, 10);
  assert.equal(ctx.calls.filter((x) => x[0] === "fillRect").length, 0);
});
