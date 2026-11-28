import { useState } from "preact/hooks";
import { CODE_RE } from "../router";
import { loadPref, savePref, type KeyValue } from "../storage";

export const SWATCHES: Array<[color: string, name: string]> = [
  ["#e4572e", "Red"],
  ["#f3a712", "Amber"],
  ["#29a35a", "Green"],
  ["#2e86de", "Blue"],
  ["#8e44ad", "Purple"],
  ["#e84393", "Pink"],
  ["#16a3a3", "Teal"],
  ["#5d6d7e", "Slate"],
];

export const MAX_NAME = 24;

/** Room codes are four letters; anything else typed is dropped. */
export const cleanCode = (raw: string) => raw.toUpperCase().replace(/[^A-Z]/g, "").slice(0, 4);

export function JoinForm({ initialCode, busy, error, storage, onJoin }: {
  initialCode: string;
  busy: boolean;
  error: string | null;
  storage: KeyValue;
  onJoin: (code: string, name: string, color: string) => void;
}) {
  const [code, setCode] = useState(() => cleanCode(initialCode));
  const [name, setName] = useState(() => (loadPref(storage, "name") ?? "").slice(0, MAX_NAME));
  const [color, setColor] = useState(() => {
    const saved = loadPref(storage, "color");
    return SWATCHES.some(([c]) => c === saved) ? saved! : SWATCHES[0]![0];
  });
  const ready = CODE_RE.test(code) && name.trim().length > 0 && !busy;

  return (
    <main class="join">
      <h1 class="brand">Hostable</h1>
      <form
        class="join-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (!ready) return;
          const n = name.trim().slice(0, MAX_NAME);
          savePref(storage, "name", n);
          savePref(storage, "color", color);
          onJoin(code, n, color);
        }}
      >
        <label class="field">
          <span>Room code</span>
          <input
            name="code"
            class="code-input"
            value={code}
            inputMode="text"
            autoCapitalize="characters"
            autoComplete="off"
            autoCorrect="off"
            spellcheck={false}
            maxLength={4}
            placeholder="ABCD"
            required
            onInput={(e) => setCode(cleanCode((e.currentTarget as HTMLInputElement).value))}
          />
        </label>
        <label class="field">
          <span>Your name</span>
          <input
            name="name"
            value={name}
            maxLength={MAX_NAME}
            autoComplete="nickname"
            enterKeyHint="go"
            required
            onInput={(e) => setName((e.currentTarget as HTMLInputElement).value.slice(0, MAX_NAME))}
          />
        </label>
        <fieldset class="swatches">
          <legend>Colour</legend>
          {SWATCHES.map(([c, label]) => (
            <label key={c} class="swatch" style={{ background: c }}>
              <input
                type="radio"
                name="color"
                value={c}
                checked={color === c}
                aria-label={label}
                onChange={() => setColor(c)}
              />
            </label>
          ))}
        </fieldset>
        {error ? (
          <p class="t-warning" role="alert">
            {error}
          </p>
        ) : null}
        <button type="submit" class="btn btn-primary btn-join" disabled={!ready}>
          {busy ? "Joining…" : "Join"}
        </button>
      </form>
    </main>
  );
}
