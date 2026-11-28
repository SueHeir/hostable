import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import { StrokeRecorder } from "../inputs";
import type { SendFn } from "./types";

/** A canvas to draw on; Done sends the strokes in 0..1 coordinates. */
export function Draw({ id, prompt, send }: { id: string; prompt?: string; send: SendFn }) {
  const rec = useMemo(() => new StrokeRecorder(), [id]);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [, setTick] = useState(0);
  const changed = () => setTick((n) => n + 1);

  const redraw = () => {
    const c = canvas.current;
    const ctx = c?.getContext?.("2d");
    if (!c || !ctx) return;
    const rect = c.getBoundingClientRect();
    const dpr = globalThis.devicePixelRatio || 1;
    const w = Math.max(1, Math.round(rect.width * dpr));
    const h = Math.max(1, Math.round(rect.height * dpr));
    if (c.width !== w || c.height !== h) {
      c.width = w;
      c.height = h;
    }
    ctx.clearRect(0, 0, w, h);
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.lineWidth = 3 * dpr;
    ctx.strokeStyle = getComputedStyle(c).color || "#000";
    for (const stroke of rec.strokes) {
      ctx.beginPath();
      stroke.forEach(([x, y], i) => (i ? ctx.lineTo(x * w, y * h) : ctx.moveTo(x * w, y * h)));
      if (stroke.length === 1) ctx.lineTo(stroke[0]![0] * w + 0.5, stroke[0]![1] * h);
      ctx.stroke();
    }
  };
  useEffect(redraw);

  const point = (e: PointerEvent): [number, number] => {
    const r = canvas.current?.getBoundingClientRect();
    const w = r && r.width > 0 ? r.width : 1;
    const h = r && r.height > 0 ? r.height : 1;
    return [(e.clientX - (r?.left ?? 0)) / w, (e.clientY - (r?.top ?? 0)) / h];
  };

  return (
    <div class="draw">
      {prompt ? <p class="t-body">{prompt}</p> : null}
      <canvas
        ref={canvas}
        class="draw-canvas"
        aria-label={prompt ? `Drawing: ${prompt}` : "Drawing area"}
        onPointerDown={(e) => {
          e.preventDefault();
          try {
            (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
          } catch {
            // Capture is a nicety.
          }
          const [x, y] = point(e);
          if (rec.begin(x, y)) changed();
        }}
        onPointerMove={(e) => {
          const [x, y] = point(e);
          if (rec.add(x, y)) redraw();
        }}
        onPointerUp={() => {
          rec.end();
          changed();
        }}
        onPointerCancel={() => {
          rec.end();
          changed();
        }}
      />
      <div class="w-row draw-tools">
        <button type="button" class="btn" disabled={!rec.strokes.length} onClick={() => { rec.undo(); changed(); }}>
          Undo
        </button>
        <button type="button" class="btn" disabled={!rec.strokes.length} onClick={() => { rec.clear(); changed(); }}>
          Clear
        </button>
        <button
          type="button"
          class="btn btn-primary"
          disabled={!rec.strokes.length}
          onClick={() => {
            send(id, rec.toValue());
            rec.clear();
            changed();
          }}
        >
          Done
        </button>
      </div>
      {rec.full ? <p class="t-muted">That's as much as fits.</p> : null}
    </div>
  );
}
