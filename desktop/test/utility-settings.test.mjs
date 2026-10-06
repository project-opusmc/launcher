import test from "node:test";
import assert from "node:assert/strict";
import { mergeUtilitySettings } from "../src/utilitySettings.ts";

const preference = { enabled: false, anchor: "top-left", offset: "12 · 12", scale: 100, opacity: 100 };
test("normalizing launcher controls retains native HUD options, other modules and root metadata", () => {
  const defaults = { schemaVersion: 1, utilities: { fps: { ...preference }, keystrokes: { ...preference, offset: "12 · 50" } } };
  const input = { schemaVersion: 1, ownerNote: "keep", utilities: {
    fps: { ...preference, enabled: true, scale: 125 },
    keystrokes: { ...preference, enabled: true, showJump: false, showMouseButtons: false },
    "armor-status": { ...preference, enabled: true, showDurability: false },
    future: { custom: 9 },
  } };
  const before = JSON.stringify(input);
  const merged = mergeUtilitySettings(input, defaults);
  assert.equal(merged.utilities.keystrokes.showJump, false);
  assert.equal(merged.utilities.keystrokes.showMouseButtons, false);
  assert.deepEqual(merged.utilities["armor-status"], input.utilities["armor-status"]);
  assert.deepEqual(merged.utilities.future, { custom: 9 });
  assert.equal(merged.utilities.fps.scale, 125);
  assert.equal(merged.ownerNote, "keep");
  assert.equal(JSON.stringify(input), before);
});
test("invalid launcher-known presentation fields fall back without deleting native extras", () => {
  const defaults = { schemaVersion: 1, utilities: { keystrokes: { ...preference, offset: "12 · 50" } } };
  const merged = mergeUtilitySettings({ schemaVersion: 1, utilities: {
    keystrokes: { ...preference, scale: 900, anchor: "bad", showJump: false },
  } }, defaults);
  assert.equal(merged.utilities.keystrokes.scale, 100);
  assert.equal(merged.utilities.keystrokes.anchor, "top-left");
  assert.equal(merged.utilities.keystrokes.showJump, false);
});
