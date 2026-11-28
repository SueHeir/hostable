import { useEffect, useState } from "preact/hooks";
import { EnvContext, type AppEnv } from "./env";
import { GamePage } from "./pages/GamePage";
import { PhonePage } from "./pages/PhonePage";
import { navigate, onNavigate, parseRoute } from "./router";

export function App({ env }: { env: AppEnv }) {
  const [path, setPath] = useState(() => location.pathname);
  useEffect(() => onNavigate(() => setPath(location.pathname)), []);
  useEffect(() => {
    const wake = () => env.wake();
    const visible = () => document.visibilityState === "visible" && env.wake();
    window.addEventListener("online", wake);
    document.addEventListener("visibilitychange", visible);
    return () => {
      window.removeEventListener("online", wake);
      document.removeEventListener("visibilitychange", visible);
    };
  }, [env]);
  const route = parseRoute(path);
  let page;
  switch (route.name) {
    case "join":
      page = <PhonePage code={null} />;
      break;
    case "room":
      page = <PhonePage code={route.code} />;
      break;
    case "game":
      page = <GamePage id={route.id} />;
      break;
    case "missing":
      page = (
        <main class="join">
          <h1 class="brand">Hostable</h1>
          <p class="t-body">There's nothing here.</p>
          <button type="button" class="btn btn-primary" onClick={() => navigate("/")}>
            Join a game
          </button>
        </main>
      );
      break;
  }
  return <EnvContext.Provider value={env}>{page}</EnvContext.Provider>;
}
