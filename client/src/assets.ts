import { BASE } from "./base";

export type ImageSource = { kind: "asset"; url: string } | { kind: "icon"; name: string };

const ASSET = /^assets\/[A-Za-z0-9_\-][A-Za-z0-9_\-./]*$/;
const ICON = /^icon:([a-z0-9_-]{1,32})$/;

/**
 * Where a widget image comes from. Only paths inside the pack's `assets/`
 * folder (served by the host at `<base>assets/...`) and built-in icons are
 * allowed; anything else (URLs, `data:`, `..`) is refused with `null`.
 */
export function imageSource(src: unknown, base: string = BASE): ImageSource | null {
  if (typeof src !== "string" || src.length > 256) return null;
  const icon = ICON.exec(src);
  if (icon) return { kind: "icon", name: icon[1]! };
  if (!ASSET.test(src)) return null;
  if (src.split("/").some((part) => part === ".." || part === "." || part === "")) return null;
  return { kind: "asset", url: assetUrl(src, base) };
}

/** The one place that decides the URL of a pack asset. */
export function assetUrl(path: string, base: string = BASE): string {
  return base + path;
}
