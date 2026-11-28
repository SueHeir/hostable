// @hostable/sdk: the TypeScript side of the Hostable rules ABI.
//
// A game is an object with the same calls as the Rust `Rules` trait. Call
// `defineGame(game)` once at the top level of your rules file; it installs
// the dispatcher the QuickJS rules host calls.
//
// Rules must be deterministic: use `Rng` seeded from `setup`'s seed and keep
// it in your state. `Math.random` and `Date` are frozen in the sandbox.

import type { Widget } from "../../../client/src/protocol/Widget";
import type { TableView } from "../../../client/src/protocol/TableView";
import type { PhoneInput } from "../../../client/src/protocol/PhoneInput";
import type { Feedback } from "../../../client/src/protocol/Feedback";

export type { Widget, TableView, PhoneInput, Feedback };
export type { ButtonStyle } from "../../../client/src/protocol/ButtonStyle";
export type { ChoiceOption } from "../../../client/src/protocol/ChoiceOption";
export type { Scene } from "../../../client/src/protocol/Scene";
export type { Shape } from "../../../client/src/protocol/Shape";
export type { PlayerPanel } from "../../../client/src/protocol/PlayerPanel";

export const RULES_API_VERSION = 1;

export type SeatId = number;

export interface Seat {
  id: SeatId;
  name: string;
  color?: string;
}

export type Event =
  | { type: "sound"; name: string }
  | { type: "toast"; text: string }
  | { type: "feedback"; seat: SeatId; kind: Feedback }
  | { type: "animate"; what: unknown };

export type OnTimeout = { type: "rules" } | { type: "input"; input: PhoneInput };

export interface Wait {
  seat: SeatId;
  prompt?: string;
  live_timeout_ms: number;
  async_timeout_ms: number;
  on_timeout?: OnTimeout;
  reaction?: boolean;
}

export interface Results {
  /** Seats from first place to last; ties share an inner list. */
  ranking: SeatId[][];
  summary: string;
}

export interface Outcome<S> {
  state: S;
  events?: Event[];
  waiting_on?: Wait[];
  finished?: Results | null;
}

export interface Game<S> {
  seats: { min: number; max: number };
  setup(seats: Seat[], seed: number): Outcome<S>;
  onAction(state: S, seat: SeatId, input: PhoneInput): Outcome<S>;
  onTimeout?(state: S, seat: SeatId, nowMs: number): Outcome<S>;
  tableView(state: S): TableView;
  phoneView(state: S, seat: SeatId): Widget;
  canRespond?(state: S, seat: SeatId): boolean;
}

/** An error the host shows as a game error (the game pauses). */
export class GameError extends Error {}

type Request =
  | { call: "info" }
  | { call: "setup"; seats: Seat[]; seed: number }
  | { call: "on_action"; state: unknown; seat: SeatId; input: PhoneInput }
  | { call: "on_timeout"; state: unknown; seat: SeatId; now_ms: number }
  | { call: "table_view"; state: unknown }
  | { call: "phone_view"; state: unknown; seat: SeatId }
  | { call: "can_respond"; state: unknown; seat: SeatId };

/** Handle one ABI request. Exported for tests; `defineGame` wires it up. */
export function dispatch<S>(game: Game<S>, text: string): string {
  try {
    const req = JSON.parse(text) as Request;
    return JSON.stringify({ ok: handle(game, req) });
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    const kind = e instanceof GameError ? "game" : "crashed";
    return JSON.stringify({ err: { kind, message } });
  }
}

function handle<S>(game: Game<S>, req: Request): unknown {
  switch (req.call) {
    case "info":
      return { min_seats: game.seats.min, max_seats: game.seats.max, api: RULES_API_VERSION };
    case "setup":
      return game.setup(req.seats, req.seed);
    case "on_action":
      return game.onAction(req.state as S, req.seat, req.input);
    case "on_timeout":
      return game.onTimeout ? game.onTimeout(req.state as S, req.seat, req.now_ms) : { state: req.state };
    case "table_view":
      return game.tableView(req.state as S);
    case "phone_view":
      return game.phoneView(req.state as S, req.seat);
    case "can_respond":
      return game.canRespond ? game.canRespond(req.state as S, req.seat) : true;
  }
  throw new GameError("unknown call");
}

declare const globalThis: {
  __hostable_dispatch?: (text: string) => string;
  __hostable_log?: (text: string) => void;
};

/** Register the game with the rules host. Call exactly once. */
export function defineGame<S>(game: Game<S>): Game<S> {
  globalThis.__hostable_dispatch = (text: string) => dispatch(game, text);
  return game;
}

/** A line in the host's log. */
export function log(message: string): void {
  if (globalThis.__hostable_log) globalThis.__hostable_log(message);
}

/**
 * SplitMix64, bit-for-bit the same as the Rust SDK's `Rng`, kept in your
 * state as a decimal string so it survives JSON.
 */
export class Rng {
  state: bigint;
  constructor(seed: number | bigint | string) {
    const mask = (1n << 64n) - 1n;
    this.state = (BigInt(seed) ^ 0x9e3779b97f4a7c15n) & mask;
  }
  static restore(saved: string): Rng {
    const r = new Rng(0);
    r.state = BigInt(saved);
    return r;
  }
  save(): string {
    return this.state.toString();
  }
  nextU64(): bigint {
    const mask = (1n << 64n) - 1n;
    this.state = (this.state + 0x9e3779b97f4a7c15n) & mask;
    let z = this.state;
    z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & mask;
    z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & mask;
    return z ^ (z >> 31n);
  }
  /** A number in 0..n-1. */
  below(n: number): number {
    return Number(((this.nextU64() >> 32n) * BigInt(n)) >> 32n);
  }
}

// ---- widget helpers ----

export const ui = {
  column: (...children: Widget[]): Widget => ({ type: "column", children }),
  row: (...children: Widget[]): Widget => ({ type: "row", children }),
  grid: (columns: number, children: Widget[]): Widget => ({ type: "grid", columns, children }),
  text: (value: string): Widget => ({ type: "text", value }),
  title: (value: string): Widget => ({ type: "text", value, style: "title" }),
  muted: (value: string): Widget => ({ type: "text", value, style: "muted" }),
  button: (id: string, label: string, style: "normal" | "primary" | "danger" | "huge" = "normal", disabled = false): Widget =>
    ({ type: "button", id, label, style, disabled }),
};
