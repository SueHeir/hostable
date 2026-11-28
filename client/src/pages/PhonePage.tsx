import { useEffect, useState } from "preact/hooks";
import type { Connection } from "../connection";
import { useConnState, useEnv } from "../env";
import { navigate } from "../router";
import { loadSeat } from "../storage";
import { JoinForm } from "./JoinForm";
import { PhoneScreen } from "./PhoneScreen";

/** `/` and `/j/{CODE}`: the join form, then the player's screen. */
export function PhonePage({ code }: { code: string | null }) {
  const env = useEnv();
  const [conn, setConn] = useState<Connection | null>(() => (code ? env.existing(code) : null));
  const state = useConnState(conn);

  useEffect(() => {
    if (!code) {
      // Back on `/` (e.g. "Join another game"): leave the old room.
      if (conn && conn.state.status !== "connecting") {
        env.disconnect(conn);
        setConn(null);
      }
      return;
    }
    let c = env.existing(code);
    if (!c && loadSeat(env.storage, code)) {
      c = env.connect(code);
      c.resume();
    }
    setConn(c);
  }, [code]);

  // Joined from `/`: move to the room's own address so a reload rejoins.
  useEffect(() => {
    if (conn && state?.status === "live" && conn.code !== code) navigate(`/j/${conn.code}`, !code);
  }, [conn, state?.status, code]);

  const showForm = !conn || !state || state.status === "needs_join" || conn.joining;
  if (showForm) {
    return (
      <JoinForm
        key={code ?? ""}
        initialCode={code ?? ""}
        busy={!!conn?.joining && state?.status === "connecting"}
        error={state?.error ?? null}
        storage={env.storage}
        onJoin={(c, name, color) => {
          const next = env.connect(c);
          setConn(next);
          next.join(name, color);
        }}
      />
    );
  }
  return (
    <main class="play">
      <PhoneScreen conn={conn} state={state} />
    </main>
  );
}
