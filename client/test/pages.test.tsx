import { buttonByText, click, mount, submit, typeInto, win, type Mounted } from "./dom";
import { afterEach, beforeEach, describe, test } from "node:test";
import assert from "node:assert/strict";
import { act } from "preact/test-utils";
import { App } from "../src/app";
import { AppEnv } from "../src/env";
import { loadSeat, memoryStorage, saveSeat } from "../src/storage";
import { loadTable } from "../src/table/LazyTable";
import { harness } from "./fake-socket";
import type { GameInfo } from "../src/api";

const config = { relay: "ws://relay.test/ws/phone", api: "/api", vapid_public_key: "" };

let mounted: Mounted[] = [];
let env: AppEnv | null = null;
beforeEach(() => win.history.replaceState(null, "", "/"));
afterEach(() => {
  mounted.forEach((m) => m.unmount());
  mounted = [];
  env?.connect("ZZZZ").close();
  env = null;
});

const flush = () => act(async () => {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
});

function app(path: string, fetchFn?: typeof fetch) {
  win.history.replaceState(null, "", path);
  const h = harness();
  const storage = memoryStorage();
  env = new AppEnv(config, storage, h.openSocket, fetchFn);
  const m = mount(<App env={env} />);
  mounted.push(m);
  return { h, storage, root: m.root };
}

describe("join screen", () => {
  test("joining from / upper-cases the code, sends join, stores the seat and moves to /j/CODE", () => {
    const { h, storage, root } = app("/");
    const code = root.querySelector<HTMLInputElement>('input[name="code"]')!;
    const name = root.querySelector<HTMLInputElement>('input[name="name"]')!;
    assert.equal(name.getAttribute("maxlength"), "24");
    const join = () => buttonByText(root, "Join") ?? buttonByText(root, "Joining…");
    assert.equal(join()!.disabled, true);
    typeInto(code, "ab1cdE");
    assert.equal(code.value, "ABCD");
    typeInto(name, "Ana");
    assert.equal(join()!.disabled, false);
    const swatches = root.querySelectorAll<HTMLInputElement>('input[name="color"]');
    assert.ok(swatches.length >= 8);
    assert.ok(swatches[3]!.getAttribute("aria-label"));
    act(() => {
      swatches[3]!.checked = true;
      swatches[3]!.dispatchEvent(new Event("change", { bubbles: true }));
    });
    submit(root.querySelector("form"));
    assert.equal(h.sockets.length, 1);
    h.last.open();
    assert.deepEqual(h.last.sent, [{ type: "join", code: "ABCD", name: "Ana", color: swatches[3]!.value }]);
    act(() => h.last.receive({ type: "joined", player: 0, player_token: "tok" }));
    assert.deepEqual(loadSeat(storage, "ABCD"), { code: "ABCD", player: 0, player_token: "tok" });
    assert.equal(win.location.pathname, "/j/ABCD");
    act(() =>
      h.last.receive({
        type: "from_host",
        message: { type: "ui", version: 1, ui: { type: "column", children: [{ type: "button", id: "buzz", label: "Buzz!", style: "huge" }] } },
      }),
    );
    click(buttonByText(root, "Buzz!"));
    assert.deepEqual(h.last.sent.at(-1), { type: "input", widget: "buzz", value: null });
    assert.equal(h.sockets.length, 1, "the join socket is kept after moving to /j/CODE");
  });

  test("a join error shows on the form", () => {
    const { h, root } = app("/j/WXYZ");
    assert.equal(root.querySelector<HTMLInputElement>('input[name="code"]')!.value, "WXYZ");
    typeInto(root.querySelector('input[name="name"]'), "Bo");
    submit(root.querySelector("form"));
    h.last.open();
    act(() => h.last.receive({ type: "error", message: "No game with that code." }));
    assert.equal(root.querySelector('[role="alert"]')!.textContent, "No game with that code.");
  });
});

describe("room screen", () => {
  test("/j/CODE with a stored token rejoins; banners for reconnecting and host away", () => {
    win.history.replaceState(null, "", "/j/ABCD");
    const h = harness();
    const storage = memoryStorage();
    saveSeat(storage, { code: "ABCD", player: 4, player_token: "mine" });
    env = new AppEnv(config, storage, h.openSocket);
    const m = mount(<App env={env} />);
    mounted.push(m);
    const root = m.root;
    assert.equal(root.querySelector("form"), null);
    h.last.open();
    assert.deepEqual(h.last.sent, [{ type: "rejoin", code: "ABCD", player_token: "mine" }]);
    act(() => {
      h.last.receive({ type: "rejoined", player: 4 });
      h.last.receive({ type: "from_host", message: { type: "ui", version: 7, ui: { type: "text", value: "Round 3 · you have 2" } } });
    });
    assert.match(root.textContent!, /Round 3 · you have 2/);
    act(() => h.last.receive({ type: "host_away" }));
    assert.match(root.textContent!, /The host is reconnecting/);
    act(() => h.last.receive({ type: "host_back" }));
    assert.doesNotMatch(root.textContent!, /The host is reconnecting/);
    act(() => h.last.drop());
    assert.match(root.textContent!, /Reconnecting…/);
    assert.match(root.textContent!, /Round 3 · you have 2/, "the last screen stays");
  });

  test("an invalid seat clears the token and shows the form with the code filled", () => {
    win.history.replaceState(null, "", "/j/ABCD");
    const h = harness();
    const storage = memoryStorage();
    saveSeat(storage, { code: "ABCD", player: 4, player_token: "old" });
    env = new AppEnv(config, storage, h.openSocket);
    const m = mount(<App env={env} />);
    mounted.push(m);
    h.last.open();
    act(() => h.last.receive({ type: "error", message: "That seat is no longer valid." }));
    assert.equal(loadSeat(storage, "ABCD"), null);
    assert.equal(m.root.querySelector<HTMLInputElement>('input[name="code"]')!.value, "ABCD");
    assert.match(m.root.textContent!, /That seat is no longer valid\./);
  });
});

describe("leaving a room", () => {
  test("Join another game from an ended room shows the form on /", () => {
    win.history.replaceState(null, "", "/j/ABCD");
    const h = harness();
    const storage = memoryStorage();
    saveSeat(storage, { code: "ABCD", player: 1, player_token: "t" });
    env = new AppEnv(config, storage, h.openSocket);
    const m = mount(<App env={env} />);
    mounted.push(m);
    h.last.open();
    act(() => {
      h.last.receive({ type: "rejoined", player: 1 });
      h.last.receive({ type: "error", message: "This seat was opened somewhere else." });
    });
    assert.match(m.root.textContent!, /opened somewhere else/);
    click(buttonByText(m.root, "Join another game"));
    assert.equal(win.location.pathname, "/");
    assert.equal(m.root.querySelector<HTMLInputElement>('input[name="code"]')!.value, "");
    assert.ok(h.last.closed);
  });
});

describe("remote game page", () => {
  const game: GameInfo = {
    id: "g1",
    pack: "buzzer",
    name: "Buzzer",
    code: "BCDF",
    pace: "async",
    started: true,
    finished: null,
    paused: null,
    seats: [
      { seat: 0, name: "Ana", claimed: true, connected: true },
      { seat: 1, name: "Bo", claimed: false, connected: false },
    ],
    table: { title: "Buzzer", status: "Round 1: buzz!", players: [{ name: "Ana", color: "#e4572e", lines: ["0 points"], active: false }], log: [] },
  };

  test("shows the table and seats, claims a seat, then rejoins asking for the table", async () => {
    await loadTable();
    const calls: Array<{ url: string; init?: RequestInit }> = [];
    const fetchFn = (async (url: string, init?: RequestInit) => {
      calls.push({ url, init });
      if (url.endsWith("/claim")) return new Response(JSON.stringify({ code: "BCDF", player: 7, player_token: "claimed" }));
      return new Response(JSON.stringify(game));
    }) as unknown as typeof fetch;
    const { h, storage, root } = app("/g/g1", fetchFn);
    await flush();
    assert.equal(calls[0]!.url, "/api/games/g1");
    assert.match(root.textContent!, /Round 1: buzz!/);
    assert.match(root.textContent!, /Ana/);
    const take = buttonByText(root, "Take this seat");
    assert.ok(take, "the open seat can be taken");
    assert.equal(root.querySelectorAll("button").length, 1, "claimed seats have no button");
    click(take);
    await flush();
    const claim = calls.find((c) => c.url === "/api/games/g1/claim")!;
    assert.equal(claim.init?.method, "POST");
    assert.deepEqual(JSON.parse(claim.init!.body as string), { seat: 1 });
    assert.deepEqual(loadSeat(storage, "BCDF"), { code: "BCDF", player: 7, player_token: "claimed" });
    h.last.open();
    act(() => h.last.receive({ type: "rejoined", player: 7 }));
    assert.deepEqual(h.last.sent, [
      { type: "rejoin", code: "BCDF", player_token: "claimed" },
      { type: "input", widget: "$table", value: true },
    ]);
    assert.equal(buttonByText(root, "Take this seat"), undefined);
    const auto = Array.from(root.querySelectorAll("label.check")).find((l) => /Only ask me/.test(l.textContent!));
    assert.ok(auto, "auto-pass setting is shown");
    assert.doesNotMatch(root.textContent!, /Notify me/, "no push without a VAPID key");
    const box = auto.querySelector("input")!;
    act(() => {
      box.checked = true;
      box.dispatchEvent(new Event("change", { bubbles: true }));
    });
    assert.deepEqual(h.last.sent.at(-1), { type: "input", widget: "$auto_pass", value: true });
    assert.equal(storage.getItem("hostable:pref:auto_pass"), "1");
  });
});
