import type { TableView } from "./protocol/TableView";
import type { StoredSeat } from "./storage";

export interface GameSeat {
  seat: number;
  name: string;
  claimed: boolean;
  connected: boolean;
}

/** `GET /api/games/{id}`. */
export interface GameInfo {
  id: string;
  pack: string;
  name: string;
  code: string;
  pace: string;
  started: boolean;
  finished: unknown;
  paused: unknown;
  seats: GameSeat[];
  table: TableView | null;
}

export class ApiError extends Error {
  constructor(readonly status: number, message: string) {
    super(message);
  }
}

async function json<T>(res: Response): Promise<T> {
  if (!res.ok) {
    let text = "";
    try {
      text = await res.text();
    } catch {
      // No body.
    }
    throw new ApiError(res.status, text || res.statusText || `HTTP ${res.status}`);
  }
  return (await res.json()) as T;
}

export async function getGame(api: string, id: string, fetchFn: typeof fetch = fetch): Promise<GameInfo> {
  const g = await json<GameInfo>(await fetchFn(`${api}/games/${encodeURIComponent(id)}`, { cache: "no-store" }));
  return { ...g, seats: Array.isArray(g.seats) ? g.seats : [], table: g.table ?? null };
}

export async function claimSeat(api: string, id: string, seat: number, fetchFn: typeof fetch = fetch): Promise<StoredSeat> {
  const res = await fetchFn(`${api}/games/${encodeURIComponent(id)}/claim`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ seat }),
  });
  if (res.status === 409) throw new ApiError(409, "Someone already took that seat.");
  return json<StoredSeat>(res);
}

/** A human line for `finished` / `paused`, whatever shape the host gives. */
export function describe(v: unknown): string | null {
  if (v === null || v === undefined || v === false) return null;
  if (typeof v === "string") return v;
  if (typeof v === "object") {
    const o = v as Record<string, unknown>;
    for (const k of ["summary", "reason", "message"]) if (typeof o[k] === "string") return o[k] as string;
  }
  return "";
}
