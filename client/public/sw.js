// Hostable service worker: keeps the app shell for offline loads and shows
// Web Push notifications ("your turn"). It never caches the API, the
// sockets or config.json.

const CACHE = "hostable-shell-v1";
const SCOPE = new URL(self.registration.scope);
const SHELL = ["./", "manifest.webmanifest", "icon.svg", "icon-192.png", "icon-512.png"].map(
  (p) => new URL(p, SCOPE).href,
);
const INDEX = new URL("./", SCOPE).href;

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE)
      .then((cache) => cache.addAll(SHELL))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

function neverCache(url) {
  const rel = url.pathname.slice(SCOPE.pathname.length);
  return (
    url.origin !== SCOPE.origin ||
    !url.pathname.startsWith(SCOPE.pathname) ||
    rel.startsWith("api/") ||
    rel.startsWith("ws/") ||
    rel === "config.json" ||
    rel === "sw.js"
  );
}

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (neverCache(url)) return;

  if (req.mode === "navigate") {
    // Network first so a new deploy shows at once; the cached shell offline.
    event.respondWith(
      fetch(req)
        .then((res) => {
          if (res.ok) {
            const copy = res.clone();
            caches.open(CACHE).then((c) => c.put(INDEX, copy));
          }
          return res;
        })
        .catch(() => caches.match(INDEX).then((hit) => hit || Response.error())),
    );
    return;
  }

  // Hashed app files and the shell: cache first, filled as they load.
  // Everything else (pack assets, which aren't hashed) goes to the network.
  const rel = url.pathname.slice(SCOPE.pathname.length);
  if (!rel.startsWith("app/") && !SHELL.includes(url.href)) return;
  event.respondWith(
    caches.match(req).then(
      (hit) =>
        hit ||
        fetch(req).then((res) => {
          if (res.ok && res.type === "basic") {
            const copy = res.clone();
            caches.open(CACHE).then((c) => c.put(req, copy));
          }
          return res;
        }),
    ),
  );
});

self.addEventListener("push", (event) => {
  let data = {};
  try {
    data = event.data ? event.data.json() : {};
  } catch {
    data = { body: event.data ? event.data.text() : "" };
  }
  const title = typeof data.title === "string" && data.title ? data.title : "Hostable";
  const body = typeof data.body === "string" ? data.body : "It's your turn.";
  const url = typeof data.url === "string" && data.url ? data.url : INDEX;
  event.waitUntil(
    self.registration.showNotification(title, {
      body,
      icon: new URL("icon-192.png", SCOPE).href,
      badge: new URL("icon-192.png", SCOPE).href,
      tag: url,
      renotify: true,
      data: { url },
    }),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const target = new URL((event.notification.data && event.notification.data.url) || INDEX, SCOPE).href;
  event.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((wins) => {
      for (const win of wins) {
        if (win.url === target && "focus" in win) return win.focus();
      }
      for (const win of wins) {
        if (new URL(win.url).origin === SCOPE.origin && "navigate" in win) {
          return win.focus().then(() => win.navigate(target));
        }
      }
      return self.clients.openWindow(target);
    }),
  );
});
