import assert from "node:assert/strict";
import test from "node:test";
import { applyUtilityPreferencePatches, mergeUtilitySettings } from "../src/utilitySettings.ts";

const fallback = {
  schemaVersion: 1,
  utilities: {
    fps: { enabled: false, anchor: "top-left", offset: "12 · 12", scale: 100, opacity: 100 },
    keystrokes: { enabled: false, anchor: "top-left", offset: "12 · 50", scale: 100, opacity: 100 },
  },
};

test("HUD scales survive Launcher hydration without changing saved options", () => {
  for (const scale of [50, 150, 154, 256, 300, 400]) {
    const saved = {
      schemaVersion: 1,
      ownerNote: "preserve",
      utilities: {
        fps: { enabled: true, anchor: "top-left", offset: "68 · 55", scale: 150, opacity: 100 },
        keystrokes: {
          enabled: true, anchor: "top-left", offset: "651 · 89", scale, opacity: 100,
          showJump: true, showMouseButtons: true,
        },
        "armor-status": { enabled: true, anchor: "bottom-right", offset: "12 · 12", scale: 300, opacity: 90 },
      },
    };
    assert.deepEqual(mergeUtilitySettings(saved, fallback), saved, `saved scale ${scale}`);
  }
});

test("out of range scales and non-integer values use the fallback", () => {
  for (const scale of [49, 401, 150.5, "154", null]) {
    const saved = structuredClone(fallback);
    saved.utilities.keystrokes.scale = scale;
    assert.equal(mergeUtilitySettings(saved, fallback).utilities.keystrokes.scale, 100);
  }
});

test("pending launcher patches overlay authoritative settings without dropping client fields", () => {
  const authoritative = {
    schemaVersion: 1,
    ownerNote: "newer client document",
    utilities: {
      fps: {
        enabled: true, anchor: "top-right", offset: "21 · 32", scale: 225, opacity: 90,
        textColor: "#7f123456",
      },
      keystrokes: {
        enabled: true, anchor: "bottom-right", offset: "12 · 50", scale: 175, opacity: 100,
        showJump: false,
      },
    },
  };

  const merged = applyUtilityPreferencePatches(authoritative, { fps: { opacity: 75 } });

  assert.equal(merged.utilities.fps.opacity, 75);
  assert.equal(merged.utilities.fps.scale, 225);
  assert.equal(merged.utilities.fps.textColor, "#7f123456");
  assert.equal(merged.utilities.keystrokes.scale, 175);
  assert.equal(merged.ownerNote, "newer client document");
  assert.equal(authoritative.utilities.fps.opacity, 90, "patching must not mutate the response snapshot");
});
