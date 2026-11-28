// Gzipped size of the JavaScript a browser loads for the join route: the
// entry scripts in dist/index.html, modulepreloads, and everything they
// import statically. Lazily imported chunks (`import(...)`) are left out.
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, normalize, relative } from "node:path";
import { gzipSync } from "node:zlib";
import { fileURLToPath } from "node:url";

const LIMIT = 150 * 1024;
const dist = fileURLToPath(new URL("../dist/", import.meta.url));
const indexPath = join(dist, "index.html");
if (!existsSync(indexPath)) {
  console.error("dist/index.html is missing; run `npm run build` first.");
  process.exit(1);
}

const base = normalizeBase(process.env.HOSTABLE_BASE ?? "/");
function normalizeBase(b) {
  if (!b.startsWith("/")) b = "/" + b;
  return b.endsWith("/") ? b : b + "/";
}

/** A URL in index.html to a file in dist, or null for other origins. */
function fromHtml(url) {
  if (/^[a-z]+:\/\//i.test(url) || url.startsWith("//")) return null;
  let path = url.split(/[?#]/)[0];
  if (path.startsWith(base)) path = path.slice(base.length);
  else if (path.startsWith("/")) path = path.slice(1);
  return normalize(join(dist, path));
}

const html = readFileSync(indexPath, "utf8");
const roots = new Set();
for (const tag of html.match(/<script\b[^>]*>/gi) ?? []) {
  if (!/type=["']?module/i.test(tag)) continue;
  const src = /\ssrc=["']([^"']+)["']/i.exec(tag)?.[1];
  if (src) roots.add(fromHtml(src));
}
for (const tag of html.match(/<link\b[^>]*>/gi) ?? []) {
  if (!/rel=["']?modulepreload/i.test(tag)) continue;
  const href = /\shref=["']([^"']+)["']/i.exec(tag)?.[1];
  if (href) roots.add(fromHtml(href));
}
roots.delete(null);
if (!roots.size) {
  console.error("No module script found in dist/index.html.");
  process.exit(1);
}

// Static imports and re-exports: `import x from "./a.js"`, `import "./a.js"`,
// `export {x} from "./a.js"`. `import(` (dynamic) never matches.
const STATIC = /(?:^|[^\w$.])(?:import|export)\s*(?:[\w$*{}\s,]*?\s*from\s*)?["'`]([^"'`]+\.js)["'`]/g;

const seen = new Set();
const queue = [...roots];
while (queue.length) {
  const file = queue.shift();
  if (seen.has(file)) continue;
  if (!existsSync(file)) {
    console.error(`Referenced file is missing: ${relative(dist, file)}`);
    process.exit(1);
  }
  seen.add(file);
  const code = readFileSync(file, "utf8");
  for (const m of code.matchAll(STATIC)) {
    const spec = m[1];
    if (!spec.startsWith(".") && !spec.startsWith("/")) continue;
    const next = spec.startsWith("/") ? fromHtml(spec) : normalize(join(dirname(file), spec));
    if (next && !seen.has(next)) queue.push(next);
  }
}

let total = 0;
for (const file of seen) {
  const gz = gzipSync(readFileSync(file), { level: 9 }).length;
  total += gz;
  console.log(`${(gz / 1024).toFixed(2).padStart(8)} KB  ${relative(dist, file)}`);
}
console.log(`${(total / 1024).toFixed(2).padStart(8)} KB  total gzipped JS for the join route (limit ${LIMIT / 1024} KB)`);
if (total >= LIMIT) {
  console.error("The join route is too big.");
  process.exit(1);
}
