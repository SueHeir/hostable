import { useState } from "preact/hooks";
import type { ComponentChildren } from "preact";
import { canConfirm, isMulti, keepAllowed, togglePick } from "../inputs";
import type { SendFn } from "./types";

export interface PickItem {
  value: string;
  disabled: boolean;
}

/**
 * Pick logic shared by `choice` and `hand`: with `max` 1 a tap sends the
 * value at once; otherwise taps toggle and Confirm sends the list once
 * `min..max` items are picked.
 */
export function usePicks(id: string, items: PickItem[], min: number, max: number, send: SendFn) {
  const [picked, setPicked] = useState<string[]>([]);
  const allowed = items.filter((i) => !i.disabled).map((i) => i.value);
  const selected = keepAllowed(picked, allowed);
  const multi = isMulti(max);
  return {
    multi,
    selected,
    isPicked: (v: string) => selected.includes(v),
    tap(v: string) {
      if (!allowed.includes(v)) return;
      if (multi) setPicked(togglePick(selected, v, max));
      else send(id, v);
    },
    canConfirm: canConfirm(selected, min, max),
    confirm() {
      if (!canConfirm(selected, min, max)) return;
      send(id, selected);
      setPicked([]);
    },
  };
}

export function PickHint({ min, max, count }: { min: number; max: number; count: number }) {
  const want = min === max ? `Pick ${max}` : min <= 0 ? `Pick up to ${max}` : `Pick ${min} to ${max}`;
  return (
    <p class="t-muted pick-hint" aria-live="polite">
      {want} · {count} picked
    </p>
  );
}

export function ConfirmButton({ enabled, onConfirm }: { enabled: boolean; onConfirm: () => void }) {
  return (
    <button type="button" class="btn btn-primary confirm" disabled={!enabled} onClick={onConfirm}>
      Confirm
    </button>
  );
}

export function Group({ label, class: cls, children }: { label?: string; class: string; children: ComponentChildren }) {
  return (
    <div class={cls} role="group" aria-label={label || undefined}>
      {label ? <p class="group-label">{label}</p> : null}
      {children}
    </div>
  );
}
