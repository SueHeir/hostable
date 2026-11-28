/** A seat this browser holds in a room, kept after `joined` or a claim. */
export interface StoredSeat {
  code: string;
  player: number;
  player_token: string;
}

export interface KeyValue {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/** `localStorage`, or memory when it is missing or blocked (private mode). */
export function browserStorage(): KeyValue {
  try {
    const ls = globalThis.localStorage;
    const probe = "hostable:probe";
    ls.setItem(probe, "1");
    ls.removeItem(probe);
    return ls;
  } catch {
    return memoryStorage();
  }
}

export function memoryStorage(): KeyValue & { data: Map<string, string> } {
  const data = new Map<string, string>();
  return {
    data,
    getItem: (k) => data.get(k) ?? null,
    setItem: (k, v) => void data.set(k, v),
    removeItem: (k) => void data.delete(k),
  };
}

export const seatKey = (code: string) => `hostable:seat:${code.trim().toUpperCase()}`;

export function loadSeat(kv: KeyValue, code: string): StoredSeat | null {
  try {
    const raw = kv.getItem(seatKey(code));
    if (!raw) return null;
    const v = JSON.parse(raw) as Partial<StoredSeat>;
    if (typeof v.player_token !== "string" || !v.player_token || typeof v.player !== "number") {
      return null;
    }
    return { code: code.trim().toUpperCase(), player: v.player, player_token: v.player_token };
  } catch {
    return null;
  }
}

export function saveSeat(kv: KeyValue, seat: StoredSeat): void {
  const code = seat.code.trim().toUpperCase();
  try {
    kv.setItem(seatKey(code), JSON.stringify({ code, player: seat.player, player_token: seat.player_token }));
  } catch {
    // Storage full or blocked: the seat lasts for this page only.
  }
}

export function clearSeat(kv: KeyValue, code: string): void {
  try {
    kv.removeItem(seatKey(code));
  } catch {
    // Nothing to clear.
  }
}

export function loadPref(kv: KeyValue, name: string): string | null {
  try {
    return kv.getItem(`hostable:pref:${name}`);
  } catch {
    return null;
  }
}

export function savePref(kv: KeyValue, name: string, value: string): void {
  try {
    kv.setItem(`hostable:pref:${name}`, value);
  } catch {
    // Preferences are a convenience.
  }
}
