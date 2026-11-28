import { render } from "preact";
import "./styles.css";
import { App } from "./app";
import { BASE } from "./base";
import { loadConfig } from "./config";
import { AppEnv } from "./env";
import { browserStorage } from "./storage";

async function start() {
  const config = await loadConfig();
  const env = new AppEnv(config, browserStorage());
  render(<App env={env} />, document.getElementById("app")!);
  if (import.meta.env.PROD && "serviceWorker" in navigator) {
    // The worker caches the app shell for offline loads and shows pushes.
    const register = () =>
      navigator.serviceWorker.register(`${BASE}sw.js`, { scope: BASE }).catch(() => undefined);
    if (document.readyState === "complete") void register();
    else window.addEventListener("load", () => void register(), { once: true });
  }
}

void start();
