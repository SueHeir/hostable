// Built-in icons a game can show with `{"type":"image","src":"icon:<name>"}`.
// Each is a path in a 24x24 box, drawn in the current text colour.

export const ICONS: Record<string, string> = {
  star: "M12 2l3 6.6 7.2.7-5.4 4.8 1.6 7.1L12 17.6 5.6 21.2l1.6-7.1L1.8 9.3 9 8.6z",
  heart: "M12 21s-8-5.2-8-11a4.5 4.5 0 0 1 8-2.8A4.5 4.5 0 0 1 20 10c0 5.8-8 11-8 11z",
  coin: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zm0 3a6 6 0 1 1 0 12 6 6 0 0 1 0-12z",
  shield: "M12 2l8 3v6c0 5-3.4 9.4-8 11-4.6-1.6-8-6-8-11V5z",
  sword: "M19 2h3v3L10 17l2 2-1.5 1.5-2-2L5 22l-3-3 3.5-3.5-2-2L5 12l2 2z",
  check: "M9 16.2l-4.2-4.2L3.4 13.4 9 19 21 7l-1.4-1.4z",
  cross: "M18.3 5.7L12 12l6.3 6.3-1.4 1.4L10.6 13.4 4.3 19.7 2.9 18.3 9.2 12 2.9 5.7 4.3 4.3l6.3 6.3 6.3-6.3z",
  clock: "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zm1 5h-2v6l5 3 1-1.7-4-2.3z",
  crown: "M3 7l4.5 4L12 4l4.5 7L21 7l-2 12H5z",
  dice: "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zm3 3.5a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3zm8 8a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3zm-4-4a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3z",
  person: "M12 12a5 5 0 1 0 0-10 5 5 0 0 0 0 10zm0 2c-5 0-9 2.5-9 6v2h18v-2c0-3.5-4-6-9-6z",
  flag: "M5 2h2v20H5zm3 1h11l-3 4.5 3 4.5H8z",
};

export function Icon({ name, label, size = 32 }: { name: string; label: string; size?: number }) {
  const d = ICONS[name];
  if (!d) return null;
  return (
    <svg
      class="icon"
      viewBox="0 0 24 24"
      width={size}
      height={size}
      role="img"
      aria-label={label || name}
      fill="currentColor"
    >
      <path d={d} />
    </svg>
  );
}
