import { describe, test } from "node:test";
import assert from "node:assert/strict";
import { BAD_TOKEN, backoffMs, Connection, GAME_ENDED, NO_ROOM, OPENED_ELSEWHERE, REMOVED } from "../src/connection";
import { loadSeat, memoryStorage, saveSeat, seatKey } from "../src/storage";
import type { Widget } from "../src/protocol/Widget";
import type { Feedback } from "../src/protocol/Feedback";
import { harness } from "./fake-socket";

const URL = "ws://relay.test/ws/phone";
const ui = (label: string): Widget => ({ type: "text", value: label });

function setup(code = "abcd") {
  const h = harness();
  const storage = memoryStorage();
  const feedback: Feedback[] = [];
  const conn = new Connection(code, {
    url: URL,
    storage,
    openSocket: h.openSocket,
    setTimer: h.setTimer,
    clearTimer: h.clearTimer,
    onFeedback: (k) => feedback.push(k),
  });
  return { h, storage, conn, feedback };
}

describe("connection", () => {
  test("join sends the join message and stores the token on joined", () => {
    const { h, storage, conn } = setup();
    conn.join("Ana", "#e4572e");
    assert.equal(conn.state.status, "connecting");
    assert.equal(conn.joining, true);
    const s = h.last;
    assert.equal(s.url, URL);
    s.open();
    assert.deepEqual(s.sent, [{ type: "join", code: "ABCD", name: "Ana", color: "#e4572e" }]);
    s.receive({ type: "joined", player: 3, player_token: "tok-3" });
    assert.equal(conn.state.status, "live");
    assert.equal(conn.state.player, 3);
    assert.equal(conn.joining, false);
    assert.deepEqual(loadSeat(storage, "ABCD"), { code: "ABCD", player: 3, player_token: "tok-3" });
    assert.ok(storage.data.has(seatKey("ABCD")));
  });

  test("inputs go out as {type:input, widget, value} only while live", () => {
    const { h, conn } = setup();
    conn.join("Ana", "#e4572e");
    assert.equal(conn.send("buzz", null), false);
    h.last.open();
    assert.equal(conn.send("buzz", null), false, "not before joined");
    h.last.receive({ type: "joined", player: 0, player_token: "t" });
    assert.equal(conn.send("buzz", undefined), true);
    assert.equal(conn.send("pick", ["a", "b"]), true);
    assert.deepEqual(h.last.sent.slice(1), [
      { type: "input", widget: "buzz", value: null },
      { type: "input", widget: "pick", value: ["a", "b"] },
    ]);
  });

  test("a dropped socket keeps the screen and rejoins with the stored token, backing off", () => {
    const { h, conn } = setup();
    conn.join("Ana", "#e4572e");
    h.last.open();
    h.last.receive({ type: "joined", player: 1, player_token: "secret" });
    h.last.receive({ type: "from_host", message: { type: "ui", version: 4, ui: ui("Round 1") } });
    h.last.drop();
    assert.equal(conn.state.status, "reconnecting");
    assert.deepEqual(conn.state.ui, ui("Round 1"), "the last screen stays up");
    assert.equal(h.sockets.length, 1, "waits before reconnecting");
    assert.equal(h.runTimer(), backoffMs(0));
    assert.equal(h.sockets.length, 2);
    h.last.drop(); // refused while the network is down
    assert.equal(h.runTimer(), backoffMs(1));
    assert.ok(backoffMs(1) > backoffMs(0));
    const s = h.last;
    s.open();
    assert.deepEqual(s.sent, [{ type: "rejoin", code: "ABCD", player_token: "secret" }]);
    s.receive({ type: "rejoined", player: 1 });
    assert.equal(conn.state.status, "live");
    s.receive({ type: "from_host", message: { type: "ui", version: 2, ui: ui("Round 2") } });
    assert.deepEqual(conn.state.ui, ui("Round 2"), "versions start fresh on a new socket");
    assert.equal(backoffMs(30), 10_000);
  });

  test("resume rejoins from storage after a reload, and wake skips the wait", () => {
    const { h, storage, conn } = setup("WXYZ");
    saveSeat(storage, { code: "wxyz", player: 2, player_token: "kept" });
    assert.equal(conn.resume(), true);
    h.last.open();
    assert.deepEqual(h.last.sent, [{ type: "rejoin", code: "WXYZ", player_token: "kept" }]);
    h.last.receive({ type: "rejoined", player: 2 });
    h.last.drop();
    assert.equal(h.timers.length, 1);
    conn.wake();
    assert.equal(h.timers.length, 0);
    assert.equal(h.sockets.length, 2);
    const other = setup("QQQQ");
    assert.equal(other.conn.resume(), false);
  });

  test("stale ui and table versions are ignored", () => {
    const { h, conn } = setup();
    conn.join("Ana", "#e4572e");
    h.last.open();
    h.last.receive({ type: "joined", player: 0, player_token: "t" });
    const send = (version: number, label: string) =>
      h.last.receive({ type: "from_host", message: { type: "ui", version, ui: ui(label) } });
    send(5, "five");
    send(3, "three");
    assert.deepEqual(conn.state.ui, ui("five"));
    send(6, "six");
    assert.deepEqual(conn.state.ui, ui("six"));
    const table = (version: number, title: string) =>
      h.last.receive({ type: "from_host", message: { type: "table", version, table: { title, status: "", players: [], log: [] } } });
    table(9, "new");
    table(8, "old");
    assert.equal(conn.state.table?.title, "new");
  });

  test("host_away and host_back toggle the banner without dropping the screen", () => {
    const { h, conn } = setup();
    conn.join("Ana", "#e4572e");
    h.last.open();
    h.last.receive({ type: "joined", player: 0, player_token: "t" });
    h.last.receive({ type: "from_host", message: { type: "ui", version: 1, ui: ui("hi") } });
    h.last.receive({ type: "host_away" });
    assert.equal(conn.state.hostAway, true);
    assert.deepEqual(conn.state.ui, ui("hi"));
    h.last.receive({ type: "host_back" });
    assert.equal(conn.state.hostAway, false);
    assert.equal(conn.state.status, "live");
  });

  test("a rejoin clears a host_away seen before the drop", () => {
    const { h, conn } = setup();
    conn.join("Ana", "#e4572e");
    h.last.open();
    h.last.receive({ type: "joined", player: 0, player_token: "t" });
    h.last.receive({ type: "host_away" });
    h.last.drop();
    h.runTimer();
    h.last.open();
    h.last.receive({ type: "rejoined", player: 0 });
    assert.equal(conn.state.hostAway, false, "the host came back while we were offline");
  });

  test("a bad stored token clears storage and returns to the form, without retrying", () => {
    const { h, storage, conn } = setup();
    saveSeat(storage, { code: "ABCD", player: 1, player_token: "stale" });
    conn.resume();
    h.last.open();
    h.last.receive({ type: "error", message: BAD_TOKEN });
    assert.equal(loadSeat(storage, "ABCD"), null);
    assert.equal(conn.state.status, "needs_join");
    assert.equal(conn.state.error, BAD_TOKEN);
    h.last.drop();
    assert.equal(h.timers.length, 0, "no reconnect loop");
    assert.equal(h.sockets.length, 1);
  });

  test("a vanished room also forgets the seat", () => {
    const { h, storage, conn } = setup();
    saveSeat(storage, { code: "ABCD", player: 1, player_token: "x" });
    conn.resume();
    h.last.open();
    h.last.receive({ type: "error", message: NO_ROOM });
    assert.equal(loadSeat(storage, "ABCD"), null);
    assert.equal(conn.state.status, "needs_join");
  });

  test("a refused join shows the error on the form and does not retry", () => {
    const { h, storage, conn } = setup();
    conn.join("Ana", "#e4572e");
    h.last.open();
    h.last.receive({ type: "error", message: "This game is full." });
    assert.equal(conn.state.status, "needs_join");
    assert.equal(conn.state.error, "This game is full.");
    assert.equal(loadSeat(storage, "ABCD"), null);
    assert.equal(h.timers.length, 0);
  });

  test("an unreachable relay during join reports instead of retrying", () => {
    const { h, conn } = setup();
    conn.join("Ana", "#e4572e");
    h.last.drop();
    assert.equal(conn.state.status, "needs_join");
    assert.match(conn.state.error ?? "", /Couldn't reach/);
    assert.equal(h.timers.length, 0);
  });

  test("fatal errors after joining stop reconnecting", () => {
    for (const [message, status, forgets] of [
      [REMOVED, "needs_join", true],
      [GAME_ENDED, "ended", true],
      [OPENED_ELSEWHERE, "ended", false],
    ] as const) {
      const { h, storage, conn } = setup();
      conn.join("Ana", "#e4572e");
      h.last.open();
      h.last.receive({ type: "joined", player: 0, player_token: "t" });
      h.last.receive({ type: "error", message });
      h.last.drop();
      assert.equal(conn.state.status, status, message);
      assert.equal(conn.state.error, message);
      assert.equal(loadSeat(storage, "ABCD") === null, forgets, message);
      assert.equal(h.timers.length, 0, message);
    }
  });

  test("opened elsewhere can be taken back with retry", () => {
    const { h, conn } = setup();
    conn.join("Ana", "#e4572e");
    h.last.open();
    h.last.receive({ type: "joined", player: 0, player_token: "t" });
    h.last.receive({ type: "error", message: OPENED_ELSEWHERE });
    conn.retry();
    h.last.open();
    assert.deepEqual(h.last.sent, [{ type: "rejoin", code: "ABCD", player_token: "t" }]);
  });

  test("other errors are passing notices", () => {
    const { h, conn } = setup();
    conn.join("Ana", "#e4572e");
    h.last.open();
    h.last.receive({ type: "joined", player: 0, player_token: "t" });
    h.last.receive({ type: "error", message: "Too many messages; slow down." });
    assert.equal(conn.state.status, "live");
    assert.equal(conn.state.notice, "Too many messages; slow down.");
  });

  test("feedback, seat info, and host-only hello inputs after every (re)join", () => {
    const { h, conn, feedback } = setup();
    conn.hello = () => [["$table", true], ["$auto_pass", false]];
    conn.join("Ana", "#e4572e");
    h.last.open();
    h.last.receive({ type: "joined", player: 0, player_token: "t" });
    assert.deepEqual(h.last.sent.slice(1), [
      { type: "input", widget: "$table", value: true },
      { type: "input", widget: "$auto_pass", value: false },
    ]);
    h.last.receive({ type: "from_host", message: { type: "feedback", kind: "your_turn" } });
    h.last.receive({ type: "from_host", message: { type: "seat", seat: 2, name: "Ana", game: "Buzzer" } });
    assert.deepEqual(feedback, ["your_turn"]);
    assert.deepEqual(conn.state.seat, { seat: 2, name: "Ana", game: "Buzzer" });
    h.last.drop();
    h.runTimer();
    h.last.open();
    h.last.receive({ type: "rejoined", player: 0 });
    assert.deepEqual(h.last.sent.map((m) => (m as { type: string; widget?: string }).widget ?? (m as { type: string }).type), [
      "rejoin",
      "$table",
      "$auto_pass",
    ]);
  });

  test("only one socket is open at a time; close stops everything", () => {
    const { h, conn } = setup();
    conn.join("Ana", "#e4572e");
    h.last.open();
    h.last.receive({ type: "joined", player: 0, player_token: "t" });
    conn.retry();
    assert.equal(h.sockets.length, 2);
    assert.equal(h.sockets[0]!.closed, true);
    h.sockets[0]!.drop(); // a late close from the old socket is ignored
    assert.equal(h.timers.length, 0);
    conn.close();
    assert.equal(h.sockets[1]!.closed, true);
    h.sockets[1]!.drop();
    assert.equal(h.timers.length, 0);
    h.sockets[1]!.receive({ type: "from_host", message: { type: "ui", version: 99, ui: ui("late") } });
    assert.equal(conn.state.ui, null);
  });

  test("garbage frames are ignored", () => {
    const { h, conn } = setup();
    conn.join("Ana", "#e4572e");
    h.last.open();
    h.last.onmessage?.({ data: "not json" });
    h.last.onmessage?.({ data: 42 });
    assert.equal(conn.state.status, "connecting");
  });
});
