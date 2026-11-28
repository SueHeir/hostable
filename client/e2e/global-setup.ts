// Starts the real relay and cloud host for the e2e run and stops them after.
//
//   ../target/debug/hostable-relay --addr 127.0.0.1:0
//   ../target/debug/hostable-cloud --addr 127.0.0.1:0 --relay ws://127.0.0.1:PORT
//       --db <tmp>/cloud.db --packs ../target/packs --client-dir dist
//
// Each prints "hostable <name> listening on 127.0.0.1:PORT" on stdout. The
// cloud URL reaches the tests as HOSTABLE_CLOUD_URL.
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const clientDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repo = resolve(clientDir, "..");
const RELAY = join(repo, "target/debug/hostable-relay");
const CLOUD = join(repo, "target/debug/hostable-cloud");
const PACKS = join(repo, "target/packs");
const START_TIMEOUT_MS = 20_000;

function need(path: string, what: string, hint: string) {
  if (!existsSync(path)) {
    throw new Error(`e2e needs ${what} at ${path}, which does not exist. ${hint}`);
  }
}

/** Spawn a server and wait for its "listening on ADDR" line. */
function start(name: string, bin: string, args: string[]): Promise<{ child: ChildProcess; addr: string }> {
  return new Promise((ok, fail) => {
    const child = spawn(bin, args, { cwd: clientDir, stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    let err = "";
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      fail(new Error(`${name} did not print its address within ${START_TIMEOUT_MS} ms.\nstdout: ${out}\nstderr: ${err}`));
    }, START_TIMEOUT_MS);
    const pattern = new RegExp(`hostable ${name} listening on (\\S+)`);
    child.stdout!.setEncoding("utf8");
    child.stderr!.setEncoding("utf8");
    child.stdout!.on("data", (chunk: string) => {
      out += chunk;
      const m = pattern.exec(out);
      if (m) {
        clearTimeout(timer);
        ok({ child, addr: m[1]! });
      }
    });
    child.stderr!.on("data", (chunk: string) => {
      err += chunk;
      if (process.env.HOSTABLE_E2E_VERBOSE) process.stderr.write(`[${name}] ${chunk}`);
    });
    child.on("error", (e) => {
      clearTimeout(timer);
      fail(new Error(`${name} failed to start: ${e.message}`));
    });
    child.on("exit", (code) => {
      clearTimeout(timer);
      fail(new Error(`${name} exited with ${code} before listening.\nstdout: ${out}\nstderr: ${err}`));
    });
  });
}

export default async function globalSetup() {
  need(RELAY, "the relay binary", "Build it with `cargo build -p hostable-relay`.");
  need(CLOUD, "the cloud host binary", "Build it with `cargo build` (the hostable-cloud crate).");
  need(PACKS, "built game packs (including buzzer)", "Build the packs into target/packs first.");

  // Always test what the current source builds to, served from the root.
  execFileSync(process.execPath, [join(clientDir, "node_modules/vite/bin/vite.js"), "build"], {
    cwd: clientDir,
    env: { ...process.env, HOSTABLE_BASE: "/" },
    stdio: "inherit",
  });
  need(join(clientDir, "dist/index.html"), "the built client", "`npm run build` failed.");

  const tmp = mkdtempSync(join(tmpdir(), "hostable-e2e-"));
  const children: ChildProcess[] = [];
  const stop = () => {
    for (const c of children) if (c.exitCode === null) c.kill("SIGTERM");
    rmSync(tmp, { recursive: true, force: true });
  };
  try {
    const relay = await start("relay", RELAY, ["--addr", "127.0.0.1:0"]);
    children.push(relay.child);
    const cloud = await start("cloud", CLOUD, [
      "--addr", "127.0.0.1:0",
      "--relay", `ws://${relay.addr}`,
      "--db", join(tmp, "cloud.db"),
      "--packs", PACKS,
      "--client-dir", "dist",
    ]);
    children.push(cloud.child);
    process.env.HOSTABLE_RELAY_ADDR = relay.addr;
    process.env.HOSTABLE_CLOUD_URL = `http://${cloud.addr}`;
  } catch (e) {
    stop();
    throw e;
  }
  return async () => stop();
}
