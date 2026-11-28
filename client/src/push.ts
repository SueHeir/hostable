import { BASE } from "./base";

/** Web Push works here and the server has a key. */
export function pushAvailable(vapidKey: string): boolean {
  return (
    !!vapidKey &&
    typeof navigator !== "undefined" &&
    "serviceWorker" in navigator &&
    typeof window !== "undefined" &&
    "PushManager" in window &&
    "Notification" in window
  );
}

/** A VAPID key in base64url to the bytes `pushManager.subscribe` wants. */
export function keyBytes(base64url: string): Uint8Array<ArrayBuffer> {
  const pad = "=".repeat((4 - (base64url.length % 4)) % 4);
  const b64 = (base64url + pad).replace(/-/g, "+").replace(/_/g, "/");
  const raw = atob(b64);
  const out = new Uint8Array(new ArrayBuffer(raw.length));
  for (let i = 0; i < raw.length; i++) out[i] = raw.charCodeAt(i);
  return out;
}

export function registerWorker(): Promise<ServiceWorkerRegistration> {
  return navigator.serviceWorker.register(`${BASE}sw.js`, { scope: BASE });
}

/** Ask permission and subscribe; returns the subscription as JSON for `$push`. */
export async function subscribe(vapidKey: string): Promise<PushSubscriptionJSON> {
  const permission = await Notification.requestPermission();
  if (permission !== "granted") throw new Error("Notifications are blocked for this site.");
  const reg = await registerWorker();
  await navigator.serviceWorker.ready;
  const existing = await reg.pushManager.getSubscription();
  const sub =
    existing ??
    (await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes(vapidKey) }));
  return sub.toJSON();
}

/** The current subscription, if one exists, without prompting. */
export async function currentSubscription(): Promise<PushSubscriptionJSON | null> {
  try {
    const reg = await navigator.serviceWorker.getRegistration(BASE);
    const sub = await reg?.pushManager.getSubscription();
    return sub ? sub.toJSON() : null;
  } catch {
    return null;
  }
}

export async function unsubscribe(): Promise<void> {
  try {
    const reg = await navigator.serviceWorker.getRegistration(BASE);
    await (await reg?.pushManager.getSubscription())?.unsubscribe();
  } catch {
    // Already gone.
  }
}
