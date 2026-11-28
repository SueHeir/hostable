import { buttonByText, click, key, mount, pointer, submit, typeInto, type Mounted } from "./dom";
import { afterEach, describe, test } from "node:test";
import assert from "node:assert/strict";
import { WidgetView } from "../src/widgets/Widget";
import type { Widget } from "../src/protocol/Widget";
import { accepts } from "./accepts";

const XSS = `<img src=x onerror="globalThis.pwned=1">`;

let mounted: Mounted[] = [];
afterEach(() => {
  mounted.forEach((m) => m.unmount());
  mounted = [];
});

function show(ui: Widget) {
  const sent: Array<{ widget: string; value: unknown }> = [];
  const m = mount(<WidgetView w={ui} send={(widget, value) => sent.push({ widget, value })} />);
  mounted.push(m);
  return { root: m.root, sent, m };
}

/** Everything sent is valid for the host, as JSON. */
function allAccepted(ui: Widget, sent: Array<{ widget: string; value: unknown }>) {
  for (const s of sent) assert.ok(accepts(ui, s), `host would refuse ${JSON.stringify(s)}`);
}

describe("layout widgets", () => {
  test("column, row and grid wrap their children", () => {
    const ui: Widget = {
      type: "column",
      children: [
        { type: "row", children: [{ type: "text", value: "a" }, { type: "text", value: "b" }] },
        { type: "grid", columns: 3, children: [{ type: "text", value: "c" }] },
      ],
    };
    const { root } = show(ui);
    assert.ok(root.querySelector(".w-column > .w-row"));
    assert.equal(root.querySelectorAll(".w-row p").length, 2);
    const grid = root.querySelector(".w-grid") as HTMLElement;
    assert.match(grid.getAttribute("style") ?? "", /repeat\(3, minmax\(0, 1fr\)\)/);
  });

  test("tabs show only the open tab and switch on tap", () => {
    const ui: Widget = {
      type: "tabs",
      tabs: [
        { label: "Hand", children: [{ type: "text", value: "first page" }] },
        { label: "Board", children: [{ type: "text", value: "second page" }] },
      ],
    };
    const { root } = show(ui);
    const tabs = root.querySelectorAll('[role="tab"]');
    assert.equal(tabs.length, 2);
    assert.equal(tabs[0]!.getAttribute("aria-selected"), "true");
    assert.match(root.textContent!, /first page/);
    assert.doesNotMatch(root.textContent!, /second page/);
    click(tabs[1]);
    assert.match(root.textContent!, /second page/);
    assert.doesNotMatch(root.textContent!, /first page/);
  });

  test("a sheet opens from its label button and closes again", () => {
    const ui: Widget = {
      type: "sheet",
      label: "More",
      children: [{ type: "button", id: "deep", label: "Deep" }],
    };
    const { root, sent } = show(ui);
    assert.equal(root.querySelector('[role="dialog"]'), null);
    click(buttonByText(root, "More"));
    const dialog = root.querySelector('[role="dialog"]');
    assert.ok(dialog);
    assert.equal(dialog.getAttribute("aria-label"), "More");
    click(buttonByText(root, "Deep"));
    assert.deepEqual(sent, [{ widget: "deep", value: null }]);
    allAccepted(ui, sent);
    click(buttonByText(root, "Close"));
    assert.equal(root.querySelector('[role="dialog"]'), null);
  });
});

describe("display widgets", () => {
  test("text styles map to elements and classes", () => {
    const ui: Widget = {
      type: "column",
      children: [
        { type: "text", value: "plain" },
        { type: "text", value: "Big", style: "title" },
        { type: "text", value: "quiet", style: "muted" },
        { type: "text", value: "careful", style: "warning" },
      ],
    };
    const { root } = show(ui);
    assert.equal(root.querySelector("p.t-body")!.textContent, "plain");
    assert.equal(root.querySelector("h2.t-title")!.textContent, "Big");
    assert.equal(root.querySelector("p.t-muted")!.textContent, "quiet");
    assert.equal(root.querySelector("p.t-warning")!.textContent, "careful");
  });

  test("text is never interpreted as HTML", () => {
    const ui: Widget = {
      type: "column",
      children: [
        { type: "text", value: XSS, style: "title" },
        { type: "text", value: XSS },
        { type: "button", id: "b", label: XSS },
        { type: "choice", id: "c", options: [{ value: "v", label: XSS, detail: XSS }], min: 1, max: 1, label: XSS },
        { type: "hand", id: "h", cards: [{ value: "k", title: XSS, text: XSS, corner: XSS }], min: 1, max: 1 },
        { type: "sheet", label: XSS, children: [] },
        { type: "tabs", tabs: [{ label: XSS, children: [{ type: "text", value: XSS }] }] },
        { type: "meter", label: XSS, value: 1, max: 2 },
        { type: "timer", ends_in_ms: 1000, label: XSS },
        { type: "text_input", id: "t", placeholder: XSS, max_len: 10, submit_label: XSS },
        { type: "draw", id: "d", prompt: XSS },
      ],
    };
    const { root } = show(ui);
    assert.equal(root.querySelectorAll("img").length, 0);
    assert.ok(root.textContent!.includes(XSS));
    assert.equal((globalThis as Record<string, unknown>).pwned, undefined);
  });

  test("images only come from assets/ or built-in icons", () => {
    const ui: Widget = {
      type: "column",
      children: [
        { type: "image", src: "assets/cards/ace.png", alt: "Ace", height: 40 },
        { type: "image", src: "icon:star", alt: "Star" },
        { type: "image", src: "https://evil.example/x.png", alt: "remote" },
        { type: "image", src: "javascript:alert(1)", alt: "js" },
        { type: "image", src: "data:image/png;base64,AAAA", alt: "data" },
        { type: "image", src: "assets/../secret.png", alt: "dots" },
        { type: "image", src: "/assets/x.png", alt: "absolute" },
        { type: "image", src: "icon:not-a-real-icon", alt: "unknown icon" },
      ],
    };
    const { root } = show(ui);
    const imgs = root.querySelectorAll("img");
    assert.equal(imgs.length, 1);
    assert.equal(imgs[0]!.getAttribute("src"), "/assets/cards/ace.png");
    assert.equal(imgs[0]!.getAttribute("alt"), "Ace");
    assert.equal(imgs[0]!.getAttribute("height"), "40");
    const svgs = root.querySelectorAll("svg");
    assert.equal(svgs.length, 1);
    assert.equal(svgs[0]!.getAttribute("aria-label"), "Star");
  });

  test("meters show value, max and style", () => {
    const ui: Widget = {
      type: "column",
      children: [
        { type: "meter", label: "HP", value: 3, max: 4, style: "health" },
        { type: "meter", label: "Mana", value: 9, max: 5, style: "mana" },
        { type: "meter", label: "Progress", value: 1, max: 0 },
      ],
    };
    const { root } = show(ui);
    const meters = root.querySelectorAll('[role="meter"]');
    assert.equal(meters.length, 3);
    assert.equal(meters[0]!.getAttribute("aria-valuenow"), "3");
    assert.equal(meters[0]!.getAttribute("aria-valuemax"), "4");
    assert.ok(root.querySelector(".meter-health"));
    assert.ok(root.querySelector(".meter-mana"));
    assert.ok(root.querySelector(".meter-normal"));
    const fills = root.querySelectorAll(".meter-fill");
    assert.match(fills[0]!.getAttribute("style")!, /width: 75\.0%/);
    assert.match(fills[1]!.getAttribute("style")!, /width: 100\.0%/);
    assert.match(fills[2]!.getAttribute("style")!, /width: 0\.0%/);
    assert.match(root.textContent!, /3 \/ 4/);
  });

  test("timers count down from ends_in_ms with a label and bar", () => {
    const ui: Widget = { type: "timer", ends_in_ms: 4200, total_ms: 8400, label: "Respond?" };
    const { root } = show(ui);
    const timer = root.querySelector('[role="timer"]')!;
    assert.equal(timer.getAttribute("aria-label"), "Respond?");
    assert.match(root.querySelector(".timer-secs")!.textContent!, /^[45]s$/);
    assert.match(root.querySelector(".timer-fill")!.getAttribute("style")!, /width: (4\d|50)\.\d%/);
    const bare = show({ type: "timer", ends_in_ms: 0 });
    assert.equal(bare.root.querySelector(".timer-bar"), null);
    assert.equal(bare.root.querySelector(".timer-secs")!.textContent, "0s");
  });
});

describe("input widgets send what the host accepts", () => {
  test("buttons send null and respect style and disabled", () => {
    const ui: Widget = {
      type: "row",
      children: [
        { type: "button", id: "buzz", label: "Buzz!", style: "huge" },
        { type: "button", id: "go", label: "Go", style: "primary" },
        { type: "button", id: "quit", label: "Quit", style: "danger" },
        { type: "button", id: "undo", label: "Undo", disabled: true },
      ],
    };
    const { root, sent } = show(ui);
    assert.ok(buttonByText(root, "Buzz!")!.classList.contains("btn-huge"));
    assert.ok(buttonByText(root, "Go")!.classList.contains("btn-primary"));
    assert.ok(buttonByText(root, "Quit")!.classList.contains("btn-danger"));
    const undo = buttonByText(root, "Undo")!;
    assert.ok(undo.classList.contains("btn-normal"));
    assert.equal(undo.disabled, true);
    click(buttonByText(root, "Buzz!"));
    click(undo);
    assert.deepEqual(sent, [{ widget: "buzz", value: null }]);
    assert.equal(JSON.stringify({ type: "input", ...sent[0] }), '{"type":"input","widget":"buzz","value":null}');
    allAccepted(ui, sent);
  });

  for (const style of ["list", "grid", "cards"] as const) {
    test(`single-pick choice (${style}) sends the value at once`, () => {
      const ui: Widget = {
        type: "choice",
        id: "pick",
        style,
        label: "Which one?",
        min: 1,
        max: 1,
        options: [
          { value: "a", label: "Apple", detail: "red" },
          { value: "b", label: "Banana", image: "icon:star" },
          { value: "c", label: "Cherry", disabled: true },
        ],
      };
      const { root, sent } = show(ui);
      assert.ok(root.querySelector(`.choice-${style}`));
      assert.equal(root.querySelector('[role="group"]')!.getAttribute("aria-label"), "Which one?");
      assert.equal(root.querySelectorAll("button.option").length, 3);
      assert.equal(buttonByText(root, "Confirm"), undefined);
      assert.match(root.textContent!, /red/);
      const opts = root.querySelectorAll<HTMLButtonElement>("button.option");
      assert.equal(opts[2]!.disabled, true);
      click(opts[1]);
      click(opts[2]);
      assert.deepEqual(sent, [{ widget: "pick", value: "b" }]);
      allAccepted(ui, sent);
    });
  }

  test("multi-pick choice toggles and confirms only within min..max", () => {
    const ui: Widget = {
      type: "choice",
      id: "discard",
      min: 2,
      max: 3,
      options: ["w", "x", "y", "z"].map((v) => ({ value: v, label: v.toUpperCase() })),
    };
    const { root, sent } = show(ui);
    const opt = (label: string) => buttonByText(root, label)!;
    const confirm = () => buttonByText(root, "Confirm")!;
    assert.equal(confirm().disabled, true);
    click(opt("W"));
    assert.equal(opt("W").getAttribute("aria-pressed"), "true");
    assert.equal(confirm().disabled, true, "one pick is below min");
    click(opt("Y"));
    assert.equal(confirm().disabled, false);
    click(opt("Z"));
    click(opt("X"));
    assert.equal(opt("X").getAttribute("aria-pressed"), "false", "a fourth pick is refused at max");
    click(opt("Z"));
    assert.equal(opt("Z").getAttribute("aria-pressed"), "false", "tapping again unpicks");
    assert.deepEqual(sent, [], "nothing is sent before Confirm");
    click(confirm());
    assert.deepEqual(sent, [{ widget: "discard", value: ["w", "y"] }]);
    allAccepted(ui, sent);
    assert.equal(confirm().disabled, true, "picks reset after sending");
  });

  test("hand cards show title, text, corner and colour, and pick like a choice", () => {
    const ui: Widget = {
      type: "hand",
      id: "hand",
      min: 1,
      max: 1,
      cards: [
        { value: "c1", title: "Spark", text: "Deal 1", corner: "2", color: "#e4572e" },
        { value: "c2", title: "Ward", color: "url(javascript:alert(1))" },
        { value: "c3", title: "Blocked", disabled: true },
      ],
    };
    const { root, sent } = show(ui);
    const cards = root.querySelectorAll<HTMLButtonElement>("button.card");
    assert.equal(cards.length, 3);
    assert.ok(root.querySelector(".cards.fan"));
    assert.equal(cards[0]!.querySelector(".card-title")!.textContent, "Spark");
    assert.equal(cards[0]!.querySelector(".card-text")!.textContent, "Deal 1");
    assert.equal(cards[0]!.querySelector(".card-corner")!.textContent, "2");
    assert.match(cards[0]!.getAttribute("style")!, /border-color: #e4572e/);
    assert.doesNotMatch(cards[1]!.getAttribute("style") ?? "", /javascript|border-color/);
    assert.equal(cards[2]!.disabled, true);
    click(cards[0]);
    assert.deepEqual(sent, [{ widget: "hand", value: "c1" }]);
    allAccepted(ui, sent);
  });

  test("multi-pick hand sends a list after Confirm", () => {
    const ui: Widget = {
      type: "hand",
      id: "hand",
      min: 1,
      max: 2,
      cards: ["a", "b", "c"].map((v) => ({ value: v, title: v })),
    };
    const { root, sent } = show(ui);
    const cards = root.querySelectorAll("button.card");
    click(cards[2]);
    click(cards[0]);
    click(buttonByText(root, "Confirm"));
    assert.deepEqual(sent, [{ widget: "hand", value: ["a", "c"] }]);
    allAccepted(ui, sent);
  });

  test("text input sends its text, capped at max_len", () => {
    const ui: Widget = { type: "text_input", id: "name", placeholder: "Your word", max_len: 5, submit_label: "Guess" };
    const { root, sent } = show(ui);
    const input = root.querySelector("input")!;
    assert.equal(input.getAttribute("maxlength"), "5");
    const label = root.querySelector(`label[for="${input.id}"]`);
    assert.equal(label?.textContent, "Your word");
    const button = buttonByText(root, "Guess")!;
    assert.equal(button.disabled, true);
    typeInto(input, "abcdefgh");
    assert.equal(input.value, "abcde");
    submit(root.querySelector("form"));
    assert.deepEqual(sent, [{ widget: "name", value: "abcde" }]);
    allAccepted(ui, sent);
    assert.equal(input.value, "");
  });

  test("dpad sends directions", () => {
    const ui: Widget = { type: "dpad", id: "pad" };
    const { root, sent } = show(ui);
    for (const label of ["Up", "Down", "Left", "Right"]) click(root.querySelector(`[aria-label="${label}"]`));
    assert.deepEqual(
      sent.map((s) => s.value),
      ["up", "down", "left", "right"],
    );
    assert.ok(sent.every((s) => s.widget === "pad"));
    allAccepted(ui, sent);
  });

  test("touchpad sends taps and {dx, dy} moves", () => {
    const ui: Widget = { type: "touchpad", id: "touch" };
    const { root, sent } = show(ui);
    const pad = root.querySelector(".touchpad")!;
    pointer(pad, "pointerdown", 0.5, 0.5);
    pointer(pad, "pointerup", 0.5, 0.5);
    assert.deepEqual(sent, [{ widget: "touch", value: { tap: true } }]);
    pointer(pad, "pointerdown", 0.1, 0.1);
    pointer(pad, "pointermove", 0.4, 0.0);
    pointer(pad, "pointerup", 0.4, 0.0);
    assert.deepEqual(sent[1], { widget: "touch", value: { dx: 0.3, dy: -0.1 } });
    key(pad, "ArrowLeft");
    key(pad, "Enter");
    assert.deepEqual(sent.slice(2).map((s) => s.value), [{ dx: -0.1, dy: 0 }, { tap: true }]);
    allAccepted(ui, sent);
  });

  test("draw sends normalised strokes on Done", () => {
    const ui: Widget = { type: "draw", id: "art", prompt: "Draw a cat" };
    const { root, sent } = show(ui);
    assert.match(root.textContent!, /Draw a cat/);
    const canvas = root.querySelector("canvas")!;
    assert.equal(buttonByText(root, "Done")!.disabled, true);
    pointer(canvas, "pointerdown", 0.1, 0.2);
    pointer(canvas, "pointermove", 0.5, 0.6);
    pointer(canvas, "pointermove", 1.7, -3);
    pointer(canvas, "pointerup", 1.7, -3);
    pointer(canvas, "pointerdown", 0.3, 0.3);
    pointer(canvas, "pointerup", 0.3, 0.3);
    click(buttonByText(root, "Done"));
    assert.deepEqual(sent, [
      {
        widget: "art",
        value: [
          [[0.1, 0.2], [0.5, 0.6], [1, 0]],
          [[0.3, 0.3]],
        ],
      },
    ]);
    allAccepted(ui, sent);
    assert.equal(buttonByText(root, "Done")!.disabled, true, "the canvas clears after sending");
  });
});
