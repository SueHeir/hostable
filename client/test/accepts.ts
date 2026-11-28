// A line-for-line port of `Widget::accepts` from crates/protocol/src/widget.rs,
// so tests can check that what the client sends is what the host accepts.
import type { Widget } from "../src/protocol/Widget";

const MAX_TEXT_CHARS = 2000;

function children(w: Widget): Widget[] {
  switch (w.type) {
    case "column":
    case "row":
    case "grid":
    case "sheet":
      return w.children;
    case "tabs":
      return w.tabs.flatMap((t) => t.children);
    default:
      return [];
  }
}

function inputs(w: Widget, out: Widget[] = []): Widget[] {
  switch (w.type) {
    case "button":
      if (!w.disabled) out.push(w);
      break;
    case "choice":
      if (w.options.some((o) => !o.disabled)) out.push(w);
      break;
    case "hand":
      if (w.cards.some((c) => !c.disabled)) out.push(w);
      break;
    case "text_input":
    case "dpad":
    case "touchpad":
    case "draw":
      out.push(w);
      break;
  }
  for (const c of children(w)) inputs(c, out);
  return out;
}

function inputId(w: Widget): string | undefined {
  return "id" in w ? w.id : undefined;
}

function picksOk(v: unknown, allowed: string[], min: number, max: number): boolean {
  if (max <= 1) return typeof v === "string" && allowed.includes(v);
  if (!Array.isArray(v)) return false;
  if (v.length < min || v.length > max) return false;
  const seen: string[] = [];
  for (const item of v) {
    if (typeof item === "string" && allowed.includes(item) && !seen.includes(item)) seen.push(item);
    else return false;
  }
  return true;
}

function touchOk(v: unknown): boolean {
  if (!v || typeof v !== "object" || Array.isArray(v)) return false;
  const o = v as Record<string, unknown>;
  const keys = Object.keys(o);
  if (o.tap === true && keys.length === 1) return true;
  const unit = (k: string) => typeof o[k] === "number" && (o[k] as number) >= -1 && (o[k] as number) <= 1;
  return keys.length === 2 && unit("dx") && unit("dy");
}

function strokesOk(v: unknown): boolean {
  if (!Array.isArray(v)) return false;
  return (
    v.length <= 64 &&
    v.every(
      (s) =>
        Array.isArray(s) &&
        s.length <= 512 &&
        s.every(
          (p) =>
            Array.isArray(p) &&
            p.length === 2 &&
            p.every((c) => typeof c === "number" && c >= 0 && c <= 1),
        ),
    )
  );
}

/** Whether the host would take `{widget, value}` (after a JSON round trip). */
export function accepts(ui: Widget, input: { widget: string; value: unknown }): boolean {
  const widget = inputs(ui).find((w) => inputId(w) === input.widget);
  if (!widget) return false;
  const v = JSON.parse(JSON.stringify(input.value ?? null)) as unknown;
  switch (widget.type) {
    case "button":
      return v === null;
    case "choice":
      return picksOk(v, widget.options.filter((o) => !o.disabled).map((o) => o.value), widget.min, widget.max);
    case "hand":
      return picksOk(v, widget.cards.filter((c) => !c.disabled).map((c) => c.value), widget.min, widget.max);
    case "text_input":
      return typeof v === "string" && Array.from(v).length <= Math.min(widget.max_len, MAX_TEXT_CHARS);
    case "dpad":
      return v === "up" || v === "down" || v === "left" || v === "right";
    case "touchpad":
      return touchOk(v);
    case "draw":
      return strokesOk(v);
    default:
      return false;
  }
}
