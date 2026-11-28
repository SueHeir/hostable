import type { SocketLike } from "../src/connection";
import type { RelayToPhone } from "../src/protocol/RelayToPhone";

/** A WebSocket stand-in the test drives by hand. */
export class FakeSocket implements SocketLike {
  readyState = 0;
  sent: unknown[] = [];
  closed = false;
  onopen: ((ev: unknown) => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onclose: ((ev: unknown) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;

  constructor(readonly url: string) {}

  send(data: string): void {
    if (this.readyState !== 1) throw new Error("send before open");
    this.sent.push(JSON.parse(data));
  }

  close(): void {
    this.closed = true;
    this.readyState = 3;
  }

  open(): void {
    this.readyState = 1;
    this.onopen?.({});
  }

  receive(msg: RelayToPhone): void {
    this.onmessage?.({ data: JSON.stringify(msg) });
  }

  /** The server or network dropped the socket. */
  drop(): void {
    this.readyState = 3;
    this.onclose?.({});
  }
}

/** Collects every socket opened, and a manual timer queue. */
export function harness() {
  const sockets: FakeSocket[] = [];
  const timers: Array<{ fn: () => void; ms: number; id: number }> = [];
  let next = 1;
  return {
    sockets,
    timers,
    openSocket: (url: string) => {
      const s = new FakeSocket(url);
      sockets.push(s);
      return s;
    },
    setTimer: (fn: () => void, ms: number) => {
      const id = next++;
      timers.push({ fn, ms, id });
      return id;
    },
    clearTimer: (id: unknown) => {
      const i = timers.findIndex((t) => t.id === id);
      if (i >= 0) timers.splice(i, 1);
    },
    /** Run the oldest pending timer; returns its delay. */
    runTimer(): number {
      const t = timers.shift();
      if (!t) throw new Error("no timer pending");
      t.fn();
      return t.ms;
    },
    get last(): FakeSocket {
      const s = sockets[sockets.length - 1];
      if (!s) throw new Error("no socket");
      return s;
    },
  };
}
