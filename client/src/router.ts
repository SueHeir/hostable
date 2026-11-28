import { BASE } from "./base";

export type Route =
  | { name: "join" }
  | { name: "room"; code: string }
  | { name: "game"; id: string }
  | { name: "missing" };

export const CODE_RE = /^[A-Z]{4}$/;

/** Which screen a path shows, with `base` (e.g. `/play/`) stripped. */
export function parseRoute(pathname: string, base: string = BASE): Route {
  let path = pathname;
  if (path.startsWith(base)) path = "/" + path.slice(base.length);
  else if (path + "/" === base) path = "/";
  path = path.replace(/\/+$/, "") || "/";
  if (path === "/" || path === "/index.html") return { name: "join" };
  const room = /^\/j\/([A-Za-z]{4})$/.exec(path);
  if (room) return { name: "room", code: room[1]!.toUpperCase() };
  const game = /^\/g\/([A-Za-z0-9_-]{1,64})$/.exec(path);
  if (game) return { name: "game", id: game[1]! };
  return { name: "missing" };
}

/** The full URL path for an app path like `/j/ABCD`. */
export function href(path: string, base: string = BASE): string {
  return base + path.replace(/^\/+/, "");
}

type Listener = () => void;
const listeners = new Set<Listener>();

export function onNavigate(fn: Listener): () => void {
  listeners.add(fn);
  const pop = () => fn();
  window.addEventListener("popstate", pop);
  return () => {
    listeners.delete(fn);
    window.removeEventListener("popstate", pop);
  };
}

export function navigate(path: string, replace = false): void {
  const url = href(path);
  if (replace) history.replaceState(null, "", url);
  else history.pushState(null, "", url);
  listeners.forEach((fn) => fn());
}
