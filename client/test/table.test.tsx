import { mount, type Mounted } from "./dom";
import { afterEach, test } from "node:test";
import assert from "node:assert/strict";
import { TableView } from "../src/table/TableView";
import { LazyTable, loadTable } from "../src/table/LazyTable";
import type { TableView as Table } from "../src/protocol/TableView";

let mounted: Mounted[] = [];
afterEach(() => {
  mounted.forEach((m) => m.unmount());
  mounted = [];
});

const table: Table = {
  title: "Buzzer",
  status: "Bo buzzed first",
  players: [
    { name: "Ana", color: "#e4572e", lines: ["2 points"], active: false },
    { name: `<img src=x onerror=alert(1)>`, color: "expression(alert(1))", lines: ["3 points"], active: true },
  ],
  log: ["Round 1", "Bo buzzed"],
  scene: { width: 10, height: 10, background: "", shapes: [{ kind: "circle", x: 5, y: 5, r: 2, fill: "red", stroke: "", label: "" }] },
};

test("table view shows status, players with the active one highlighted, and the log", () => {
  const m = mount(<TableView table={table} />);
  mounted.push(m);
  const root = m.root;
  assert.equal(root.querySelector("h2")!.textContent, "Buzzer");
  assert.equal(root.querySelector('[role="status"]')!.textContent, "Bo buzzed first");
  const players = root.querySelectorAll("li.player");
  assert.equal(players.length, 2);
  assert.ok(!players[0]!.classList.contains("active"));
  assert.ok(players[1]!.classList.contains("active"));
  assert.equal(players[1]!.getAttribute("aria-current"), "true");
  assert.match(players[0]!.textContent!, /Ana.*2 points/);
  assert.equal(root.querySelectorAll("img").length, 0, "names are text");
  const dots = root.querySelectorAll<HTMLElement>(".dot");
  assert.match(dots[0]!.getAttribute("style")!, /#e4572e/);
  assert.match(dots[1]!.getAttribute("style")!, /#888888/);
  assert.deepEqual(Array.from(root.querySelectorAll(".log li")).map((li) => li.textContent), ["Round 1", "Bo buzzed"]);
  assert.ok(root.querySelector("canvas.scene"));
});

test("the lazy wrapper renders the table once its chunk has loaded", async () => {
  await loadTable();
  const m = mount(<LazyTable table={table} />);
  mounted.push(m);
  assert.match(m.root.textContent!, /Bo buzzed first/);
});
