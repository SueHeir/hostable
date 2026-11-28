import { createContext } from "preact";
import { useContext, useEffect, useState } from "preact/hooks";
import type { ClientConfig } from "./config";
import { Connection, type ConnState, type SocketLike } from "./connection";
import { vibrate } from "./feedback";
import type { KeyValue } from "./storage";

/** What every page needs: config, storage, and the page's one connection. */
export class AppEnv {
  private current: Connection | null = null;

  constructor(
    readonly config: ClientConfig,
    readonly storage: KeyValue,
    private readonly openSocket?: (url: string) => SocketLike,
    readonly fetchFn: typeof fetch = (...args) => fetch(...args),
  ) {}

  /** The open connection for `code`, if there is one. */
  existing(code: string): Connection | null {
    return this.current && this.current.code === code.toUpperCase() ? this.current : null;
  }

  /** A fresh connection for `code`, closing any other (one socket per page). */
  connect(code: string): Connection {
    this.current?.close();
    this.current = new Connection(code, {
      url: this.config.relay,
      storage: this.storage,
      openSocket: this.openSocket,
      onFeedback: (kind) => vibrate(kind),
    });
    return this.current;
  }

  /** Close `conn` if it is the page's connection. */
  disconnect(conn: Connection): void {
    conn.close();
    if (this.current === conn) this.current = null;
  }

  /** Reconnect right away when the browser comes back online or to the front. */
  wake(): void {
    this.current?.wake();
  }
}

export const EnvContext = createContext<AppEnv | null>(null);

export function useEnv(): AppEnv {
  const env = useContext(EnvContext);
  if (!env) throw new Error("AppEnv missing");
  return env;
}

export function useConnState(conn: Connection | null): ConnState | null {
  const [state, setState] = useState<ConnState | null>(conn?.state ?? null);
  useEffect(() => {
    setState(conn?.state ?? null);
    return conn?.subscribe(setState);
  }, [conn]);
  return conn ? (state?.code === conn.code ? state : conn.state) : null;
}
