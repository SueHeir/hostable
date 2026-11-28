import type { Scene } from "../protocol/Scene";
import type { Shape } from "../protocol/Shape";
import { optionalColor, sanitizeColor } from "../color";

/** The parts of `CanvasRenderingContext2D` the scene renderer uses. */
export interface Ctx2D {
  fillStyle: unknown;
  strokeStyle: unknown;
  lineWidth: number;
  font: string;
  textAlign: string;
  textBaseline: string;
  setTransform(a: number, b: number, c: number, d: number, e: number, f: number): void;
  clearRect(x: number, y: number, w: number, h: number): void;
  fillRect(x: number, y: number, w: number, h: number): void;
  strokeRect(x: number, y: number, w: number, h: number): void;
  beginPath(): void;
  closePath(): void;
  moveTo(x: number, y: number): void;
  lineTo(x: number, y: number): void;
  arc(x: number, y: number, r: number, a0: number, a1: number): void;
  fill(): void;
  stroke(): void;
  fillText(text: string, x: number, y: number, maxWidth?: number): void;
}

export interface Fit {
  scale: number;
  offsetX: number;
  offsetY: number;
}

/** Scale a `width` x `height` scene to fit a box, centred, keeping its aspect. */
export function fitScene(scene: Pick<Scene, "width" | "height">, boxW: number, boxH: number): Fit {
  const sw = scene.width > 0 ? scene.width : 1;
  const sh = scene.height > 0 ? scene.height : 1;
  const scale = Math.max(0, Math.min(boxW / sw, boxH / sh));
  return { scale, offsetX: (boxW - sw * scale) / 2, offsetY: (boxH - sh * scale) / 2 };
}

/** Corners of a pointy-top hexagon with circumradius `r`, starting at the top. */
export function hexCorners(x: number, y: number, r: number): Array<[number, number]> {
  const out: Array<[number, number]> = [];
  for (let k = 0; k < 6; k++) {
    const a = ((60 * k - 90) * Math.PI) / 180;
    out.push([x + r * Math.cos(a), y + r * Math.sin(a)]);
  }
  return out;
}

const finite = (...v: number[]) => v.every(Number.isFinite);
const MAX_LABEL = 200;

/**
 * Draw a table scene into a `boxW` x `boxH` CSS-pixel box (y down), scaled to
 * fit and centred. `dpr` is the device pixel ratio of the backing canvas.
 */
export function drawScene(ctx: Ctx2D, scene: Scene, boxW: number, boxH: number, dpr = 1): Fit {
  const fit = fitScene(scene, boxW, boxH);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, boxW, boxH);
  ctx.setTransform(dpr * fit.scale, 0, 0, dpr * fit.scale, dpr * fit.offsetX, dpr * fit.offsetY);
  const bg = optionalColor(scene.background);
  if (bg) {
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, scene.width, scene.height);
  }
  for (const shape of scene.shapes) drawShape(ctx, shape, fit.scale);
  return fit;
}

function paint(ctx: Ctx2D, fill: string, stroke: string, scale: number) {
  ctx.fillStyle = sanitizeColor(fill);
  ctx.fill();
  const s = optionalColor(stroke);
  if (s) {
    ctx.strokeStyle = s;
    ctx.lineWidth = scale > 0 ? 1.5 / scale : 1;
    ctx.stroke();
  }
}

function label(ctx: Ctx2D, text: string, x: number, y: number, size: number, maxWidth: number) {
  if (!text) return;
  ctx.fillStyle = "#111111";
  ctx.font = `600 ${Math.max(1, size).toFixed(2)}px system-ui, sans-serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(text.slice(0, MAX_LABEL), x, y, maxWidth);
}

export function drawShape(ctx: Ctx2D, s: Shape, scale = 1): void {
  switch (s.kind) {
    case "hex": {
      if (!finite(s.x, s.y, s.r) || s.r <= 0) return;
      ctx.beginPath();
      hexCorners(s.x, s.y, s.r).forEach(([px, py], i) => (i ? ctx.lineTo(px, py) : ctx.moveTo(px, py)));
      ctx.closePath();
      paint(ctx, s.fill, s.stroke, scale);
      label(ctx, s.label, s.x, s.y, s.r * 0.5, s.r * 1.6);
      return;
    }
    case "circle": {
      if (!finite(s.x, s.y, s.r) || s.r <= 0) return;
      ctx.beginPath();
      ctx.arc(s.x, s.y, s.r, 0, Math.PI * 2);
      ctx.closePath();
      paint(ctx, s.fill, s.stroke, scale);
      label(ctx, s.label, s.x, s.y, s.r * 0.7, s.r * 1.8);
      return;
    }
    case "rect": {
      if (!finite(s.x, s.y, s.w, s.h) || s.w <= 0 || s.h <= 0) return;
      ctx.beginPath();
      ctx.moveTo(s.x, s.y);
      ctx.lineTo(s.x + s.w, s.y);
      ctx.lineTo(s.x + s.w, s.y + s.h);
      ctx.lineTo(s.x, s.y + s.h);
      ctx.closePath();
      paint(ctx, s.fill, s.stroke, scale);
      label(ctx, s.label, s.x + s.w / 2, s.y + s.h / 2, Math.min(s.h * 0.4, s.w * 0.3), s.w * 0.95);
      return;
    }
    case "line": {
      if (!finite(s.x1, s.y1, s.x2, s.y2, s.width)) return;
      ctx.beginPath();
      ctx.moveTo(s.x1, s.y1);
      ctx.lineTo(s.x2, s.y2);
      ctx.strokeStyle = sanitizeColor(s.color);
      ctx.lineWidth = s.width > 0 ? s.width : 1;
      ctx.stroke();
      return;
    }
    case "text": {
      if (!finite(s.x, s.y, s.size) || !s.text) return;
      ctx.fillStyle = s.color ? sanitizeColor(s.color) : "#111111";
      ctx.font = `${Math.max(1, s.size).toFixed(2)}px system-ui, sans-serif`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(s.text.slice(0, MAX_LABEL), s.x, s.y);
      return;
    }
  }
}
