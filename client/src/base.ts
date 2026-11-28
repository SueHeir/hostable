/** The path the app is served under, always ending in `/` (e.g. `/play/`). */
export const BASE: string = normalizeBase(import.meta.env?.BASE_URL ?? "/");

export function normalizeBase(base: string): string {
  let b = base || "/";
  if (!b.startsWith("/")) b = "/" + b;
  if (!b.endsWith("/")) b += "/";
  return b;
}
