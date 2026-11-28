// Draws the app icons into public/: icon.svg plus 192 and 512 px PNGs.
// Run with `npm run icons`; the output is committed.
import { writeFileSync } from "node:fs";
import { deflateSync } from "node:zlib";
import { fileURLToPath } from "node:url";

const out = (name) => fileURLToPath(new URL(`../public/${name}`, import.meta.url));

const BLUE = [31, 111, 235];
const WHITE = [255, 255, 255];
const AMBER = [243, 167, 18];

// The design, in a 0..1 box: a rounded blue tile, a white round table seen
// from above, and an amber dot for the shared screen in the middle.
function colourAt(x, y) {
  const r = 0.2;
  const cx = Math.min(Math.max(x, r), 1 - r);
  const cy = Math.min(Math.max(y, r), 1 - r);
  if (Math.hypot(x - cx, y - cy) > r) return null;
  const d = Math.hypot(x - 0.5, y - 0.5);
  if (d < 0.11) return AMBER;
  if (d < 0.3 && d > 0.2) return WHITE;
  for (let k = 0; k < 6; k++) {
    const a = (k * Math.PI) / 3 - Math.PI / 2;
    if (Math.hypot(x - (0.5 + 0.38 * Math.cos(a)), y - (0.5 + 0.38 * Math.sin(a))) < 0.055) return WHITE;
  }
  return BLUE;
}

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function png(size) {
  const ss = 4; // supersampling for smooth edges
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let py = 0; py < size; py++) {
    raw[py * (size * 4 + 1)] = 0;
    for (let px = 0; px < size; px++) {
      let r = 0, g = 0, b = 0, a = 0;
      for (let sy = 0; sy < ss; sy++) {
        for (let sx = 0; sx < ss; sx++) {
          const c = colourAt((px + (sx + 0.5) / ss) / size, (py + (sy + 0.5) / ss) / size);
          if (c) { r += c[0]; g += c[1]; b += c[2]; a += 1; }
        }
      }
      const o = py * (size * 4 + 1) + 1 + px * 4;
      const n = ss * ss;
      raw[o] = a ? Math.round(r / a) : 0;
      raw[o + 1] = a ? Math.round(g / a) : 0;
      raw[o + 2] = a ? Math.round(b / a) : 0;
      raw[o + 3] = Math.round((a / n) * 255);
    }
  }
  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header[8] = 8; // bit depth
  header[9] = 6; // RGBA
  const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  return Buffer.concat([
    signature,
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const seats = Array.from({ length: 6 }, (_, k) => {
  const a = (k * Math.PI) / 3 - Math.PI / 2;
  return `<circle cx="${(50 + 38 * Math.cos(a)).toFixed(2)}" cy="${(50 + 38 * Math.sin(a)).toFixed(2)}" r="5.5" fill="#fff"/>`;
}).join("");
const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">` +
  `<rect width="100" height="100" rx="20" fill="#1f6feb"/>` +
  `<circle cx="50" cy="50" r="25" fill="none" stroke="#fff" stroke-width="10"/>` +
  `<circle cx="50" cy="50" r="11" fill="#f3a712"/>${seats}</svg>\n`;

writeFileSync(out("icon.svg"), svg);
writeFileSync(out("icon-192.png"), png(192));
writeFileSync(out("icon-512.png"), png(512));
console.log("wrote public/icon.svg, icon-192.png, icon-512.png");
