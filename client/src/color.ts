/** Colours a game or player may use. Anything else falls back to grey. */
export const FALLBACK_COLOR = "#888888";

const HEX = /^#(?:[0-9a-f]{3}|[0-9a-f]{6})$/i;

const NAMED = new Set([
  "black", "white", "gray", "grey", "silver", "red", "maroon", "orange", "gold",
  "yellow", "olive", "lime", "green", "teal", "cyan", "aqua", "blue", "navy",
  "purple", "violet", "magenta", "fuchsia", "pink", "brown", "tan", "beige",
  "ivory", "khaki", "coral", "salmon", "crimson", "indigo", "turquoise",
  "orchid", "plum", "chocolate", "sienna", "darkgreen", "darkblue", "darkred",
  "lightblue", "lightgreen", "lightgray", "lightgrey", "darkgray", "darkgrey",
  "skyblue", "steelblue", "forestgreen", "seagreen", "slategray", "slategrey",
  "wheat", "transparent",
]);

/**
 * Returns `input` if it is a `#rgb` / `#rrggbb` colour or a known colour
 * name, otherwise `fallback`. Keeps game-supplied strings out of style
 * attributes and canvas state.
 */
export function sanitizeColor(input: unknown, fallback: string = FALLBACK_COLOR): string {
  if (typeof input !== "string") return fallback;
  const s = input.trim();
  if (HEX.test(s)) return s.toLowerCase();
  const lower = s.toLowerCase();
  if (NAMED.has(lower)) return lower;
  return fallback;
}

/** Like {@link sanitizeColor} but an empty or bad colour means "none". */
export function optionalColor(input: unknown): string | null {
  const c = sanitizeColor(input, "");
  return c === "" ? null : c;
}
