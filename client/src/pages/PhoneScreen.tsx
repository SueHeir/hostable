import { useCallback } from "preact/hooks";
import type { Connection, ConnState } from "../connection";
import { LazyTable } from "../table/LazyTable";
import { WidgetView } from "../widgets/Widget";
import { navigate } from "../router";

/** A seated player's screen: banners, the host's widget tree, and the table if sent. */
export function PhoneScreen({ conn, state, showTable = true }: { conn: Connection; state: ConnState; showTable?: boolean }) {
  const send = useCallback((widget: string, value: unknown) => {
    conn.send(widget, value);
  }, [conn]);
  return (
    <div class="phone">
      {state.seat ? (
        <p class="seat-line t-muted">
          {state.seat.name}
          {state.seat.game ? ` · ${state.seat.game}` : ""}
          {state.seat.seat === null ? " · watching" : ""}
        </p>
      ) : null}
      {state.status === "reconnecting" ? (
        <p class="banner" role="status">
          Reconnecting…
        </p>
      ) : null}
      {state.hostAway && state.status === "live" ? (
        <p class="banner" role="status">
          The host is reconnecting
        </p>
      ) : null}
      {state.notice ? <p class="banner banner-quiet">{state.notice}</p> : null}
      {state.status === "ended" ? (
        <div class="ended" role="alert">
          <p class="t-warning">{state.error}</p>
          <div class="w-row">
            <button type="button" class="btn btn-primary" onClick={() => conn.retry()}>
              Try again
            </button>
            <button type="button" class="btn" onClick={() => navigate("/")}>
              Join another game
            </button>
          </div>
        </div>
      ) : null}
      {state.ui ? (
        <div class={`screen${state.status === "live" ? "" : " stale"}`}>
          <WidgetView w={state.ui} send={send} />
        </div>
      ) : state.status !== "ended" ? (
        <p class="t-muted connecting">Connecting…</p>
      ) : null}
      {showTable && state.table ? <LazyTable table={state.table} /> : null}
    </div>
  );
}
