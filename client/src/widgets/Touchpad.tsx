import { useMemo, useRef } from "preact/hooks";
import { TouchTracker } from "../inputs";
import type { SendFn } from "./types";

const KEY_STEP = 0.1;
const KEYS: Record<string, [number, number]> = {
  ArrowUp: [0, -KEY_STEP],
  ArrowDown: [0, KEY_STEP],
  ArrowLeft: [-KEY_STEP, 0],
  ArrowRight: [KEY_STEP, 0],
};

/** Drag to move a cursor on the shared screen, tap to click. */
export function Touchpad({ id, send }: { id: string; send: SendFn }) {
  const tracker = useMemo(() => new TouchTracker((v) => send(id, v)), [id, send]);
  const ref = useRef<HTMLDivElement>(null);
  const size = () => {
    const r = ref.current?.getBoundingClientRect();
    return { w: r?.width ?? 0, h: r?.height ?? 0 };
  };
  return (
    <div
      ref={ref}
      class="touchpad"
      role="application"
      aria-label="Touchpad: drag to move, tap to select"
      tabIndex={0}
      onPointerDown={(e) => {
        const { w, h } = size();
        try {
          (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
        } catch {
          // Capture is a nicety.
        }
        tracker.down(e.clientX, e.clientY, w, h);
      }}
      onPointerMove={(e) => tracker.move(e.clientX, e.clientY)}
      onPointerUp={(e) => tracker.up(e.clientX, e.clientY)}
      onPointerCancel={() => tracker.cancel()}
      onKeyDown={(e) => {
        const step = KEYS[e.key];
        if (step) {
          e.preventDefault();
          send(id, { dx: step[0], dy: step[1] });
        } else if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          send(id, { tap: true });
        }
      }}
    >
      <span class="touchpad-hint" aria-hidden="true">Drag · Tap</span>
    </div>
  );
}
