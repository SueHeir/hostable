import { test } from "node:test";
import assert from "node:assert/strict";
import { FALLBACK_COLOR, optionalColor, sanitizeColor } from "../src/color";

test("accepts #rgb, #rrggbb and known names", () => {
  assert.equal(sanitizeColor("#e4572e"), "#e4572e");
  assert.equal(sanitizeColor("#ABC"), "#abc");
  assert.equal(sanitizeColor(" Red "), "red");
  assert.equal(sanitizeColor("darkgreen"), "darkgreen");
});

test("anything else falls back to grey", () => {
  for (const bad of [
    "",
    "#12",
    "#12345",
    "#1234567",
    "#ggg",
    "rgb(1,2,3)",
    "red; background: url(x)",
    "url(javascript:alert(1))",
    "expression(alert(1))",
    "notacolour",
    "var(--x)",
    null,
    undefined,
    42,
    {},
  ]) {
    assert.equal(sanitizeColor(bad), FALLBACK_COLOR, String(bad));
  }
  assert.equal(sanitizeColor("bad", "#000"), "#000");
});

test("optional colours turn bad or empty into none", () => {
  assert.equal(optionalColor(""), null);
  assert.equal(optionalColor("nope"), null);
  assert.equal(optionalColor("#fff"), "#fff");
});
