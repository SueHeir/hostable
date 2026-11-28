import { useEffect, useState } from "preact/hooks";

/**
 * A countdown drawn from `ends_in_ms` at the moment it arrives. Purely
 * visual: the host enforces the deadline.
 */
export function Timer({ endsInMs, totalMs, label, now = Date.now }: {
  endsInMs: number;
  totalMs?: number;
  label?: string;
  now?: () => number;
}) {
  const [endAt, setEndAt] = useState(() => now() + Math.max(0, endsInMs));
  const [t, setT] = useState(now);
  useEffect(() => {
    const end = now() + Math.max(0, endsInMs);
    setEndAt(end);
    setT(now());
    const h = setInterval(() => {
      const n = now();
      setT(n);
      if (n >= end) clearInterval(h);
    }, 200);
    return () => clearInterval(h);
  }, [endsInMs]);
  const left = Math.max(0, endAt - t);
  const secs = Math.ceil(left / 1000);
  const total = totalMs && totalMs > 0 ? totalMs : 0;
  const frac = total ? Math.max(0, Math.min(1, left / total)) : null;
  return (
    <div class={`timer${left <= 3000 ? " timer-low" : ""}`} role="timer" aria-label={label || "Time left"}>
      <div class="timer-head">
        {label ? <span class="timer-label">{label}</span> : null}
        <span class="timer-secs">{secs}s</span>
      </div>
      {frac !== null ? (
        <div class="timer-bar">
          <div class="timer-fill" style={{ width: `${(frac * 100).toFixed(1)}%` }} />
        </div>
      ) : null}
    </div>
  );
}
