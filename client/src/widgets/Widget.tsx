import { useEffect, useState } from "preact/hooks";
import type { Widget } from "../protocol/Widget";
import type { ChoiceOption } from "../protocol/ChoiceOption";
import type { CardFace } from "../protocol/CardFace";
import type { Direction } from "../protocol/Direction";
import { imageSource } from "../assets";
import { optionalColor } from "../color";
import { clampText, MAX_TEXT_CHARS } from "../inputs";
import { Icon } from "./icons";
import { ConfirmButton, Group, PickHint, usePicks } from "./Picker";
import { Timer } from "./Timer";
import { Touchpad } from "./Touchpad";
import { Draw } from "./Draw";
import type { SendFn } from "./types";

export type { SendFn } from "./types";

/** Draws one node of the phone UI tree. Text is always text, never HTML. */
export function WidgetView({ w, send }: { w: Widget; send: SendFn }) {
  switch (w.type) {
    case "column":
      return <div class="w-column">{children(w.children, send)}</div>;
    case "row":
      return <div class="w-row">{children(w.children, send)}</div>;
    case "grid": {
      const cols = Math.max(1, Math.min(12, Math.floor(w.columns) || 1));
      return (
        <div class="w-grid" style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }}>
          {children(w.children, send)}
        </div>
      );
    }
    case "tabs":
      return <Tabs tabs={w.tabs} send={send} />;
    case "sheet":
      return <Sheet label={w.label} items={w.children} send={send} />;
    case "text":
      return <Text value={w.value} style={w.style ?? "body"} />;
    case "image":
      return <Image src={w.src} alt={w.alt} height={w.height} />;
    case "meter":
      return <Meter label={w.label} value={w.value} max={w.max} style={w.style ?? "normal"} />;
    case "timer":
      return <Timer endsInMs={w.ends_in_ms} totalMs={w.total_ms} label={w.label} />;
    case "button":
      return (
        <button
          type="button"
          class={`btn btn-${w.style ?? "normal"}`}
          disabled={!!w.disabled}
          onClick={() => send(w.id, null)}
        >
          {w.label}
        </button>
      );
    case "choice":
      return <Choice w={w} send={send} />;
    case "hand":
      return <Hand w={w} send={send} />;
    case "text_input":
      return <TextInput id={w.id} placeholder={w.placeholder} maxLen={w.max_len} submit={w.submit_label} send={send} />;
    case "dpad":
      return <Dpad id={w.id} send={send} />;
    case "touchpad":
      return <Touchpad id={w.id} send={send} />;
    case "draw":
      return <Draw id={w.id} prompt={w.prompt} send={send} />;
    default: {
      const unknown: never = w;
      void unknown;
      return null;
    }
  }
}

function keyOf(w: Widget, i: number): string {
  return "id" in w && w.id ? `${w.type}:${w.id}` : `${w.type}:${i}`;
}

function children(items: Widget[], send: SendFn) {
  return items.map((c, i) => <WidgetView key={keyOf(c, i)} w={c} send={send} />);
}

function Text({ value, style }: { value: string; style: string }) {
  if (style === "title") return <h2 class="t-title">{value}</h2>;
  const cls = style === "muted" ? "t-muted" : style === "warning" ? "t-warning" : "t-body";
  return <p class={cls}>{value}</p>;
}

function Image({ src, alt, height }: { src: string; alt: string; height?: number }) {
  const source = imageSource(src);
  if (!source) return null;
  const h = height && height > 0 ? Math.min(height, 2000) : undefined;
  if (source.kind === "icon") return <Icon name={source.name} label={alt} size={h ?? 32} />;
  return <img class="w-image" src={source.url} alt={alt} height={h} style={h ? { height: `${h}px` } : undefined} loading="lazy" />;
}

function Meter({ label, value, max, style }: { label: string; value: number; max: number; style: string }) {
  const pct = max > 0 ? Math.max(0, Math.min(100, (value / max) * 100)) : 0;
  return (
    <div class={`meter meter-${style}`}>
      <div class="meter-head">
        <span>{label}</span>
        <span>
          {value} / {max}
        </span>
      </div>
      <div class="meter-bar" role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={max} aria-valuenow={value}>
        <div class="meter-fill" style={{ width: `${pct.toFixed(1)}%` }} />
      </div>
    </div>
  );
}

function Tabs({ tabs, send }: { tabs: { label: string; children: Widget[] }[]; send: SendFn }) {
  const [open, setOpen] = useState(0);
  const active = Math.min(open, Math.max(0, tabs.length - 1));
  const tab = tabs[active];
  return (
    <div class="tabs">
      <div class="tab-list" role="tablist">
        {tabs.map((t, i) => (
          <button
            key={i}
            type="button"
            role="tab"
            class="tab"
            aria-selected={i === active}
            onClick={() => setOpen(i)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div class="tab-panel" role="tabpanel" aria-label={tab?.label}>
        {tab ? children(tab.children, send) : null}
      </div>
    </div>
  );
}

function Sheet({ label, items, send }: { label: string; items: Widget[]; send: SendFn }) {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (!open) return;
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, [open]);
  return (
    <>
      <button type="button" class="btn sheet-open" aria-expanded={open} onClick={() => setOpen(true)}>
        {label}
      </button>
      {open ? (
        <div class="sheet-layer">
          <div class="sheet-backdrop" onClick={() => setOpen(false)} />
          <div class="sheet" role="dialog" aria-modal="true" aria-label={label}>
            <div class="sheet-head">
              <h2 class="t-title">{label}</h2>
              <button type="button" class="btn sheet-close" onClick={() => setOpen(false)}>
                Close
              </button>
            </div>
            <div class="w-column">{children(items, send)}</div>
          </div>
        </div>
      ) : null}
    </>
  );
}

type ChoiceWidget = Extract<Widget, { type: "choice" }>;
type HandWidget = Extract<Widget, { type: "hand" }>;

function Choice({ w, send }: { w: ChoiceWidget; send: SendFn }) {
  const items = w.options.map((o) => ({ value: o.value, disabled: !!o.disabled }));
  const picks = usePicks(w.id, items, w.min, w.max, send);
  const style = w.style ?? "list";
  return (
    <Group label={w.label} class={`choice choice-${style}`}>
      <div class="options">
        {w.options.map((o) => (
          <OptionButton key={o.value} o={o} multi={picks.multi} picked={picks.isPicked(o.value)} onTap={() => picks.tap(o.value)} />
        ))}
      </div>
      {picks.multi ? (
        <>
          <PickHint min={w.min} max={w.max} count={picks.selected.length} />
          <ConfirmButton enabled={picks.canConfirm} onConfirm={picks.confirm} />
        </>
      ) : null}
    </Group>
  );
}

function OptionButton({ o, multi, picked, onTap }: { o: ChoiceOption; multi: boolean; picked: boolean; onTap: () => void }) {
  const img = o.image ? imageSource(o.image) : null;
  return (
    <button
      type="button"
      class={`option${picked ? " picked" : ""}`}
      disabled={!!o.disabled}
      aria-pressed={multi ? picked : undefined}
      onClick={onTap}
    >
      {img?.kind === "asset" ? <img src={img.url} alt="" class="option-img" /> : null}
      {img?.kind === "icon" ? <Icon name={img.name} label="" size={28} /> : null}
      <span class="option-label">{o.label}</span>
      {o.detail ? <span class="option-detail">{o.detail}</span> : null}
    </button>
  );
}

function Hand({ w, send }: { w: HandWidget; send: SendFn }) {
  const items = w.cards.map((c) => ({ value: c.value, disabled: !!c.disabled }));
  const picks = usePicks(w.id, items, w.min, w.max, send);
  const n = w.cards.length;
  const fan = n > 1 && n <= 7;
  return (
    <Group label="Your hand" class="hand">
      <div class={`cards${fan ? " fan" : ""}`}>
        {w.cards.map((c, i) => (
          <Card
            key={c.value}
            c={c}
            tilt={fan ? (i - (n - 1) / 2) * 4 : 0}
            multi={picks.multi}
            picked={picks.isPicked(c.value)}
            onTap={() => picks.tap(c.value)}
          />
        ))}
      </div>
      {picks.multi ? (
        <>
          <PickHint min={w.min} max={w.max} count={picks.selected.length} />
          <ConfirmButton enabled={picks.canConfirm} onConfirm={picks.confirm} />
        </>
      ) : null}
    </Group>
  );
}

function Card({ c, tilt, multi, picked, onTap }: { c: CardFace; tilt: number; multi: boolean; picked: boolean; onTap: () => void }) {
  const frame = optionalColor(c.color);
  const img = c.image ? imageSource(c.image) : null;
  const style: Record<string, string> = {};
  if (frame) style.borderColor = frame;
  if (tilt) style.transform = `rotate(${tilt.toFixed(1)}deg)`;
  return (
    <button
      type="button"
      class={`card${picked ? " picked" : ""}`}
      style={style}
      disabled={!!c.disabled}
      aria-pressed={multi ? picked : undefined}
      onClick={onTap}
    >
      {c.corner ? <span class="card-corner">{c.corner}</span> : null}
      <span class="card-title">{c.title}</span>
      {img?.kind === "asset" ? <img src={img.url} alt="" class="card-img" /> : null}
      {img?.kind === "icon" ? <Icon name={img.name} label="" size={36} /> : null}
      {c.text ? <span class="card-text">{c.text}</span> : null}
    </button>
  );
}

let inputSerial = 0;

function TextInput({ id, placeholder, maxLen, submit, send }: {
  id: string;
  placeholder: string;
  maxLen: number;
  submit?: string;
  send: SendFn;
}) {
  const [value, setValue] = useState("");
  const [fieldId] = useState(() => `ti-${++inputSerial}`);
  const limit = Math.max(0, Math.min(maxLen, MAX_TEXT_CHARS));
  return (
    <form
      class="text-input"
      onSubmit={(e) => {
        e.preventDefault();
        send(id, clampText(value, limit));
        setValue("");
      }}
    >
      <label class="sr-only" for={fieldId}>
        {placeholder || "Your answer"}
      </label>
      <input
        id={fieldId}
        type="text"
        value={value}
        placeholder={placeholder}
        maxLength={limit}
        autoComplete="off"
        onInput={(e) => setValue(clampText((e.currentTarget as HTMLInputElement).value, limit))}
      />
      <button type="submit" class="btn btn-primary" disabled={value.length === 0}>
        {submit || "Send"}
      </button>
    </form>
  );
}

const ARROWS: Array<[Direction, string, string]> = [
  ["up", "Up", "▲"],
  ["left", "Left", "◀"],
  ["right", "Right", "▶"],
  ["down", "Down", "▼"],
];

function Dpad({ id, send }: { id: string; send: SendFn }) {
  return (
    <div class="dpad" role="group" aria-label="Direction pad">
      {ARROWS.map(([dir, label, glyph]) => (
        <button key={dir} type="button" class={`btn dpad-${dir}`} aria-label={label} onClick={() => send(id, dir)}>
          {glyph}
        </button>
      ))}
    </div>
  );
}
