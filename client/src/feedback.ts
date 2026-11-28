import type { Feedback } from "./protocol/Feedback";

export const VIBRATION: Record<Feedback, number[]> = {
  tap: [15],
  success: [40, 60, 40],
  warning: [120, 60, 120],
  your_turn: [200, 100, 200, 100, 200],
};

/** Buzz the phone for a `feedback` message, if the browser can. */
export function vibrate(kind: Feedback, nav: { vibrate?: (p: number[]) => boolean } | undefined =
  typeof navigator === "undefined" ? undefined : navigator): boolean {
  const pattern = VIBRATION[kind];
  if (!pattern || !nav || typeof nav.vibrate !== "function") return false;
  try {
    return nav.vibrate(pattern);
  } catch {
    return false;
  }
}
