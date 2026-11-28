import { BASE } from "./base";

export interface ClientConfig {
  /** The phone socket URL, e.g. `wss://example.org/play/ws/phone`. */
  relay: string;
  /** Base URL of the cloud HTTP API, e.g. `/play/api`. */
  api: string;
  /** Empty when Web Push isn't set up. */
  vapid_public_key: string;
}

interface Loc {
  protocol: string;
  host: string;
}

export function fallbackConfig(loc: Loc, base: string = BASE): ClientConfig {
  const scheme = loc.protocol === "https:" ? "wss" : "ws";
  return {
    relay: `${scheme}://${loc.host}${base}ws/phone`,
    api: `${base}api`,
    vapid_public_key: "",
  };
}

/** Reads `<base>config.json`, filling anything missing from the fallback. */
export async function loadConfig(
  fetchFn: typeof fetch = fetch,
  loc: Loc = location,
  base: string = BASE,
): Promise<ClientConfig> {
  const fallback = fallbackConfig(loc, base);
  try {
    const res = await fetchFn(`${base}config.json`, { cache: "no-store" });
    if (!res.ok) return fallback;
    const raw = (await res.json()) as Partial<ClientConfig>;
    const str = (v: unknown, d: string) => (typeof v === "string" && v ? v : d);
    return {
      relay: str(raw.relay, fallback.relay),
      api: str(raw.api, fallback.api).replace(/\/+$/, ""),
      vapid_public_key: typeof raw.vapid_public_key === "string" ? raw.vapid_public_key : "",
    };
  } catch {
    return fallback;
  }
}
