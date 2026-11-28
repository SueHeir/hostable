import type { Feedback } from "./protocol/Feedback";
import type { PhoneToRelay } from "./protocol/PhoneToRelay";
import type { RelayToPhone } from "./protocol/RelayToPhone";
import type { TableView } from "./protocol/TableView";
import type { Widget } from "./protocol/Widget";
import { clearSeat, loadSeat, saveSeat, type KeyValue } from "./storage";

/** What the relay says when a stored token no longer matches a seat. */
export const BAD_TOKEN = "That seat is no longer valid.";
export const NO_ROOM = "No game with that code.";
export const REMOVED = "You were removed from the game.";
export const GAME_ENDED = "The game has ended.";
export const OPENED_ELSEWHERE = "This seat was opened somewhere else.";

export type Status =
  /** Opening the socket or waiting for `joined` / `rejoined`. */
  | "connecting"
  /** Seated and talking to the host. */
  | "live"
  /** The socket dropped; trying again with backoff. */
  | "reconnecting"
  /** No usable seat: show the join form (with `error`, if any). */
  | "needs_join"
  /** Stopped for good, with the reason in `error`; `retry()` tries again. */
  | "ended";

export interface SeatInfo {
  seat: number | null;
  name: string;
  game: string;
}

export interface ConnState {
  code: string;
  status: Status;
  player: number | null;
  ui: Widget | null;
  uiVersion: number;
  table: TableView | null;
  tableVersion: number;
  hostAway: boolean;
  seat: SeatInfo | null;
  /** Why the form is showing or the connection stopped. */
  error: string | null;
  /** A passing warning from the relay, e.g. "slow down". */
  notice: string | null;
}

/** The parts of `WebSocket` the connection uses. */
export interface SocketLike {
  readonly readyState: number;
  send(data: string): void;
  close(): void;
  onopen: ((ev: unknown) => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
  onclose: ((ev: unknown) => void) | null;
  onerror: ((ev: unknown) => void) | null;
}

export interface ConnDeps {
  /** Phone socket URL from `config.json`. */
  url: string;
  storage: KeyValue;
  openSocket?: (url: string) => SocketLike;
  setTimer?: (fn: () => void, ms: number) => unknown;
  clearTimer?: (handle: unknown) => void;
  onFeedback?: (kind: Feedback) => void;
}

/** Host-only inputs re-sent after every `joined` / `rejoined`. */
export type Hello = () => Array<[widget: string, value: unknown]>;

const OPEN = 1;

/** Delay before reconnect attempt `n` (0-based): 0.5 s doubling to 10 s. */
export function backoffMs(attempt: number): number {
  return Math.min(10_000, 500 * 2 ** Math.min(attempt, 10));
}

/**
 * One phone's link to a room: joins or rejoins, keeps the seat token, shows
 * the newest screen, and reconnects with backoff when the socket drops.
 * Exactly one socket is open at a time.
 */
export class Connection {
  readonly code: string;
  hello: Hello = () => [];
  private st: ConnState;
  private readonly deps: Required<Omit<ConnDeps, "onFeedback">> & Pick<ConnDeps, "onFeedback">;
  private listeners = new Set<(s: ConnState) => void>();
  private socket: SocketLike | null = null;
  private first: PhoneToRelay | null = null;
  private handshaken = false;
  private gotError = false;
  private attempt = 0;
  private timer: unknown = null;
  private stopped = false;

  constructor(code: string, deps: ConnDeps) {
    this.code = code.trim().toUpperCase();
    this.deps = {
      url: deps.url,
      storage: deps.storage,
      onFeedback: deps.onFeedback,
      openSocket: deps.openSocket ?? ((url) => new WebSocket(url) as unknown as SocketLike),
      setTimer: deps.setTimer ?? ((fn, ms) => setTimeout(fn, ms)),
      clearTimer: deps.clearTimer ?? ((h) => clearTimeout(h as ReturnType<typeof setTimeout>)),
    };
    this.st = {
      code: this.code,
      status: "needs_join",
      player: null,
      ui: null,
      uiVersion: -1,
      table: null,
      tableVersion: -1,
      hostAway: false,
      seat: null,
      error: null,
      notice: null,
    };
  }

  get state(): ConnState {
    return this.st;
  }

  /** Taking a new seat and not yet answered. */
  get joining(): boolean {
    return this.first?.type === "join" && !this.handshaken;
  }

  subscribe(fn: (s: ConnState) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Take a new seat. */
  join(name: string, color: string): void {
    this.stopped = false;
    this.first = { type: "join", code: this.code, name, color };
    this.attempt = 0;
    this.set({ status: "connecting", error: null, notice: null });
    this.connect();
  }

  /** Rejoin with the stored token. Returns false if there is none. */
  resume(): boolean {
    const seat = loadSeat(this.deps.storage, this.code);
    if (!seat) return false;
    this.stopped = false;
    this.first = { type: "rejoin", code: this.code, player_token: seat.player_token };
    this.attempt = 0;
    this.set({ status: this.st.ui ? "reconnecting" : "connecting", error: null, player: seat.player });
    this.connect();
    return true;
  }

  /** After `ended`: try the stored seat again, or fall back to the form. */
  retry(): void {
    if (!this.resume()) this.set({ status: "needs_join" });
  }

  /** Reconnect now instead of waiting out the backoff (e.g. back online). */
  wake(): void {
    if (this.st.status === "reconnecting" && !this.stopped && this.timer !== null) {
      this.deps.clearTimer(this.timer);
      this.timer = null;
      this.connect();
    }
  }

  /** Send `{"type":"input",widget,value}`. Dropped (false) while not live. */
  send(widget: string, value: unknown): boolean {
    const s = this.socket;
    if (!s || !this.handshaken || s.readyState !== OPEN) return false;
    const msg: PhoneToRelay = { type: "input", widget, value: value === undefined ? null : value };
    s.send(JSON.stringify(msg));
    return true;
  }

  close(): void {
    this.stopped = true;
    this.cancelTimer();
    this.drop();
  }

  private set(patch: Partial<ConnState>): void {
    this.st = { ...this.st, ...patch };
    for (const fn of this.listeners) fn(this.st);
  }

  private cancelTimer(): void {
    if (this.timer !== null) {
      this.deps.clearTimer(this.timer);
      this.timer = null;
    }
  }

  private drop(): void {
    const s = this.socket;
    this.socket = null;
    this.handshaken = false;
    if (s) {
      s.onopen = s.onmessage = s.onclose = s.onerror = null;
      try {
        s.close();
      } catch {
        // Already closed.
      }
    }
  }

  private connect(): void {
    this.cancelTimer();
    this.drop();
    if (!this.first) return;
    let s: SocketLike;
    try {
      s = this.deps.openSocket(this.deps.url);
    } catch {
      this.lost();
      return;
    }
    this.socket = s;
    this.gotError = false;
    s.onopen = () => {
      if (this.socket === s && this.first) s.send(JSON.stringify(this.first));
    };
    s.onmessage = (ev) => {
      if (this.socket !== s || typeof ev.data !== "string") return;
      let msg: RelayToPhone;
      try {
        msg = JSON.parse(ev.data) as RelayToPhone;
      } catch {
        return;
      }
      this.handle(msg);
    };
    s.onclose = () => {
      if (this.socket !== s) return;
      this.socket = null;
      this.handshaken = false;
      this.lost();
    };
    s.onerror = () => {
      // A close event always follows.
    };
  }

  private lost(): void {
    if (this.stopped) return;
    if (this.first?.type === "join") {
      if (!this.gotError) {
        this.set({
          status: "needs_join",
          error: "Couldn't reach the game. Check your connection and try again.",
        });
      }
      this.stopped = true;
      return;
    }
    const delay = backoffMs(this.attempt++);
    this.set({ status: "reconnecting" });
    this.timer = this.deps.setTimer(() => {
      this.timer = null;
      if (!this.stopped) this.connect();
    }, delay);
  }

  private seated(player: number): void {
    this.handshaken = true;
    this.attempt = 0;
    // Versions count per host session; start fresh on every new socket.
    // The relay re-sends `host_away` after this if the host is still gone.
    this.set({ status: "live", player, uiVersion: -1, tableVersion: -1, hostAway: false, error: null, notice: null });
    for (const [widget, value] of this.hello()) this.send(widget, value);
  }

  private handle(msg: RelayToPhone): void {
    switch (msg.type) {
      case "joined":
        saveSeat(this.deps.storage, { code: this.code, player: msg.player, player_token: msg.player_token });
        this.first = { type: "rejoin", code: this.code, player_token: msg.player_token };
        this.seated(msg.player);
        return;
      case "rejoined":
        this.seated(msg.player);
        return;
      case "host_away":
        this.set({ hostAway: true });
        return;
      case "host_back":
        this.set({ hostAway: false });
        return;
      case "error":
        this.error(msg.message);
        return;
      case "from_host": {
        const m = msg.message;
        switch (m.type) {
          case "ui":
            if (m.version < this.st.uiVersion) return;
            this.set({ ui: m.ui, uiVersion: m.version });
            return;
          case "table":
            if (m.version < this.st.tableVersion) return;
            this.set({ table: m.table, tableVersion: m.version });
            return;
          case "feedback":
            this.deps.onFeedback?.(m.kind);
            return;
          case "seat":
            this.set({ seat: { seat: m.seat, name: m.name, game: m.game } });
            return;
        }
      }
    }
  }

  private error(message: string): void {
    this.gotError = true;
    const forget = () => clearSeat(this.deps.storage, this.code);
    if (!this.handshaken) {
      // The relay closes the socket after a refused first message, and
      // wrong tries are rate-limited, so never retry on our own.
      this.stopped = true;
      this.cancelTimer();
      this.drop();
      if (this.first?.type === "join") {
        this.set({ status: "needs_join", error: message });
      } else if (message === BAD_TOKEN || message === NO_ROOM) {
        forget();
        this.first = null;
        this.set({ status: "needs_join", error: message, ui: null, table: null, player: null });
      } else {
        this.set({ status: "ended", error: message });
      }
      return;
    }
    if (message === REMOVED || message === BAD_TOKEN) {
      forget();
      this.stop();
      this.first = null;
      this.set({ status: "needs_join", error: message, ui: null, table: null, player: null });
    } else if (message === GAME_ENDED) {
      forget();
      this.stop();
      this.first = null;
      this.set({ status: "ended", error: message, hostAway: false });
    } else if (message === OPENED_ELSEWHERE) {
      this.stop();
      this.set({ status: "ended", error: message, hostAway: false });
    } else {
      this.set({ notice: message });
    }
  }

  private stop(): void {
    this.stopped = true;
    this.cancelTimer();
    this.drop();
  }
}
