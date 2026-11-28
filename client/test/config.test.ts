import { describe, test } from "node:test";
import assert from "node:assert/strict";
import { fallbackConfig, loadConfig } from "../src/config";
import { href, parseRoute } from "../src/router";
import { normalizeBase } from "../src/base";
import { imageSource } from "../src/assets";

describe("routes", () => {
  test("parse at the root", () => {
    assert.deepEqual(parseRoute("/", "/"), { name: "join" });
    assert.deepEqual(parseRoute("/j/abcd", "/"), { name: "room", code: "ABCD" });
    assert.deepEqual(parseRoute("/j/ABCD/", "/"), { name: "room", code: "ABCD" });
    assert.deepEqual(parseRoute("/g/g7Kx_9-a", "/"), { name: "game", id: "g7Kx_9-a" });
    assert.deepEqual(parseRoute("/j/AB1D", "/"), { name: "missing" });
    assert.deepEqual(parseRoute("/nope", "/"), { name: "missing" });
  });

  test("parse under a base path", () => {
    assert.deepEqual(parseRoute("/play/", "/play/"), { name: "join" });
    assert.deepEqual(parseRoute("/play", "/play/"), { name: "join" });
    assert.deepEqual(parseRoute("/play/j/WXYZ", "/play/"), { name: "room", code: "WXYZ" });
    assert.deepEqual(parseRoute("/play/g/abc", "/play/"), { name: "game", id: "abc" });
    assert.equal(href("/j/ABCD", "/play/"), "/play/j/ABCD");
    assert.equal(normalizeBase("play"), "/play/");
  });

  test("pack assets resolve under the base", () => {
    assert.deepEqual(imageSource("assets/a.png", "/play/"), { kind: "asset", url: "/play/assets/a.png" });
  });
});

describe("runtime config", () => {
  const loc = { protocol: "https:", host: "example.org" };

  test("reads config.json from the base", async () => {
    let asked = "";
    const fetchFn = (async (url: string) => {
      asked = url;
      return new Response(JSON.stringify({ relay: "wss://r/ws/phone", api: "/play/api/", vapid_public_key: "BXYZ" }));
    }) as unknown as typeof fetch;
    const c = await loadConfig(fetchFn, loc, "/play/");
    assert.equal(asked, "/play/config.json");
    assert.deepEqual(c, { relay: "wss://r/ws/phone", api: "/play/api", vapid_public_key: "BXYZ" });
  });

  test("falls back when the fetch fails or is not found", async () => {
    const failing = (async () => {
      throw new Error("offline");
    }) as unknown as typeof fetch;
    assert.deepEqual(await loadConfig(failing, loc, "/play/"), {
      relay: "wss://example.org/play/ws/phone",
      api: "/play/api",
      vapid_public_key: "",
    });
    const missing = (async () => new Response("no", { status: 404 })) as unknown as typeof fetch;
    assert.deepEqual(await loadConfig(missing, { protocol: "http:", host: "localhost:5173" }, "/"), {
      relay: "ws://localhost:5173/ws/phone",
      api: "/api",
      vapid_public_key: "",
    });
    assert.equal(fallbackConfig(loc, "/").relay, "wss://example.org/ws/phone");
  });
});
