import { useEffect, useRef, useState } from "preact/hooks";
import { claimSeat, describe, getGame, type GameInfo } from "../api";
import type { Connection } from "../connection";
import { useConnState, useEnv, type AppEnv } from "../env";
import { currentSubscription, pushAvailable, subscribe, unsubscribe } from "../push";
import { loadPref, loadSeat, saveSeat, savePref } from "../storage";
import { LazyTable } from "../table/LazyTable";
import { PhoneScreen } from "./PhoneScreen";

export const POLL_MS = 3000;

/** Host-only inputs sent after every (re)join on a remote screen. */
export function remoteHello(env: AppEnv): Array<[string, unknown]> {
  const out: Array<[string, unknown]> = [["$table", true]];
  const auto = loadPref(env.storage, "auto_pass");
  if (auto !== null) out.push(["$auto_pass", auto === "1"]);
  return out;
}

/** `/g/{id}`: a cloud game for remote and async play. */
export function GamePage({ id }: { id: string }) {
  const env = useEnv();
  const [game, setGame] = useState<GameInfo | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [conn, setConn] = useState<Connection | null>(null);
  const [claimError, setClaimError] = useState<string | null>(null);
  const [claiming, setClaiming] = useState<number | null>(null);
  const state = useConnState(conn);
  const socketHasTable = useRef(false);
  socketHasTable.current = state?.status === "live" && !!state.table;

  // Poll the game until the socket brings the table itself.
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      if (!socketHasTable.current) {
        try {
          const g = await getGame(env.config.api, id, env.fetchFn);
          if (!alive) return;
          setGame(g);
          setLoadError(null);
        } catch (e) {
          if (alive) setLoadError(e instanceof Error && e.message ? e.message : "Couldn't load this game.");
        }
      }
      if (alive) timer = setTimeout(tick, POLL_MS);
    };
    void tick();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [id]);

  const start = (code: string) => {
    const c = env.connect(code);
    c.hello = () => remoteHello(env);
    setConn(c);
    c.resume();
  };

  // A seat this browser already holds: take it back.
  useEffect(() => {
    if (game && !conn && loadSeat(env.storage, game.code)) start(game.code);
  }, [game?.code]);

  // Re-send the push subscription after each (re)join.
  useEffect(() => {
    if (!conn || state?.status !== "live" || loadPref(env.storage, "push") !== "1") return;
    if (!pushAvailable(env.config.vapid_public_key)) return;
    void currentSubscription().then((sub) => sub && conn.send("$push", sub));
  }, [conn, state?.status]);

  const seated = !!conn && !!state && state.status !== "needs_join";
  const table = state?.table ?? game?.table ?? null;

  const claim = async (seat: number) => {
    if (!game) return;
    setClaiming(seat);
    setClaimError(null);
    try {
      const got = await claimSeat(env.config.api, id, seat, env.fetchFn);
      saveSeat(env.storage, { code: got.code || game.code, player: got.player, player_token: got.player_token });
      start(got.code || game.code);
      try {
        setGame(await getGame(env.config.api, id, env.fetchFn));
      } catch {
        // The next poll will catch up.
      }
    } catch (e) {
      setClaimError(e instanceof Error && e.message ? e.message : "Couldn't take that seat.");
      try {
        setGame(await getGame(env.config.api, id, env.fetchFn));
      } catch {
        // Keep the old list.
      }
    } finally {
      setClaiming(null);
    }
  };

  if (!game) {
    return (
      <main class="game">
        <p class={loadError ? "t-warning" : "t-muted"} role={loadError ? "alert" : "status"}>
          {loadError ?? "Loading the game…"}
        </p>
      </main>
    );
  }

  const finished = describe(game.finished);
  const paused = describe(game.paused);
  return (
    <main class="game">
      <header class="game-head">
        <h1 class="t-title">{game.name || game.pack}</h1>
        <p class="t-muted">
          Code {game.code} · {game.pace === "async" ? "async" : "live"}
          {finished !== null ? " · finished" : ""}
        </p>
        {finished ? <p class="t-body">{finished}</p> : null}
        {paused !== null ? <p class="t-warning">Paused{paused ? `: ${paused}` : ""}</p> : null}
        {loadError ? <p class="t-warning">{loadError}</p> : null}
      </header>
      <div class="game-layout">
        <section class="game-table" aria-label="Table">
          {table ? (
            <LazyTable table={table} />
          ) : (
            <p class="t-muted">{game.started ? "Waiting for the table…" : "The game hasn't started yet."}</p>
          )}
        </section>
        <aside class="game-side">
          {seated && conn && state ? (
            <>
              <PhoneScreen conn={conn} state={state} showTable={false} />
              <Settings conn={conn} env={env} />
            </>
          ) : null}
          <Seats game={game} canClaim={!seated} claiming={claiming} onClaim={claim} error={claimError} />
        </aside>
      </div>
    </main>
  );
}

function Seats({ game, canClaim, claiming, onClaim, error }: {
  game: GameInfo;
  canClaim: boolean;
  claiming: number | null;
  onClaim: (seat: number) => void;
  error: string | null;
}) {
  if (!game.seats.length) return null;
  return (
    <section class="seats" aria-label="Seats">
      <h2 class="group-label">Seats</h2>
      <ul>
        {game.seats.map((s) => (
          <li key={s.seat} class="seat">
            <span class={`dot${s.connected ? " online" : ""}`} aria-hidden="true" />
            <span class="seat-name">{s.name || `Seat ${s.seat + 1}`}</span>
            <span class="t-muted">{s.claimed ? (s.connected ? "online" : "away") : "open"}</span>
            {canClaim && !s.claimed ? (
              <button type="button" class="btn btn-primary" disabled={claiming !== null} onClick={() => onClaim(s.seat)}>
                {claiming === s.seat ? "Taking…" : "Take this seat"}
              </button>
            ) : null}
          </li>
        ))}
      </ul>
      {error ? (
        <p class="t-warning" role="alert">
          {error}
        </p>
      ) : null}
    </section>
  );
}

function Settings({ conn, env }: { conn: Connection; env: AppEnv }) {
  const [autoPass, setAutoPass] = useState(() => loadPref(env.storage, "auto_pass") === "1");
  const [push, setPush] = useState(() => loadPref(env.storage, "push") === "1");
  const [pushError, setPushError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const canPush = pushAvailable(env.config.vapid_public_key);
  return (
    <section class="settings" aria-label="Settings">
      <h2 class="group-label">Settings</h2>
      {canPush ? (
        <label class="check">
          <input
            type="checkbox"
            checked={push}
            disabled={busy}
            onChange={async (e) => {
              const on = (e.currentTarget as HTMLInputElement).checked;
              setBusy(true);
              setPushError(null);
              try {
                if (on) {
                  const sub = await subscribe(env.config.vapid_public_key);
                  conn.send("$push", sub);
                } else {
                  await unsubscribe();
                }
                savePref(env.storage, "push", on ? "1" : "0");
                setPush(on);
              } catch (err) {
                setPush(false);
                setPushError(err instanceof Error ? err.message : "Couldn't turn on notifications.");
              } finally {
                setBusy(false);
              }
            }}
          />
          <span>Notify me when it's my turn</span>
        </label>
      ) : null}
      {pushError ? <p class="t-warning">{pushError}</p> : null}
      <label class="check">
        <input
          type="checkbox"
          checked={autoPass}
          onChange={(e) => {
            const on = (e.currentTarget as HTMLInputElement).checked;
            setAutoPass(on);
            savePref(env.storage, "auto_pass", on ? "1" : "0");
            conn.send("$auto_pass", on);
          }}
        />
        <span>Only ask me when I can respond</span>
      </label>
    </section>
  );
}
