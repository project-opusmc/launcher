import type { TuiUtilityId, TuiUtilityPreference } from "./ui/LauncherTui";

// Match the client HUD editor's scale range, including values above 255%.
export const UTILITY_SCALE_MIN = 50;
export const UTILITY_SCALE_MAX = 400;

export type UtilitySettings = {
  [key: string]: unknown;
  schemaVersion: number;
  utilities: Record<TuiUtilityId, TuiUtilityPreference> & Record<string, unknown>;
};

export type UtilityPreferencePatches = Partial<Record<TuiUtilityId, Partial<TuiUtilityPreference>>>;

/** Keep authoritative disk fields while replaying launcher edits not saved yet. */
export function applyUtilityPreferencePatches(
  settings: UtilitySettings,
  patches: UtilityPreferencePatches,
): UtilitySettings {
  const utilities = { ...settings.utilities };
  for (const [id, patch] of Object.entries(patches) as [TuiUtilityId, Partial<TuiUtilityPreference>][]) {
    utilities[id] = { ...utilities[id], ...patch };
  }
  return { ...settings, utilities };
}

/** Sanitize launcher controls without discarding settings owned by the client. */
export function mergeUtilitySettings(
  settings: UtilitySettings | null | undefined,
  fallback: UtilitySettings,
): UtilitySettings {
  if (!settings?.utilities || settings.schemaVersion !== 1) return fallback;
  const merged = { ...fallback.utilities, ...settings.utilities };
  const validAnchors = ["top-left", "top-right", "bottom-left", "bottom-right"] as const;
  for (const id of Object.keys(fallback.utilities) as TuiUtilityId[]) {
    const candidate = settings.utilities[id];
    const defaults = fallback.utilities[id];
    if (!candidate || typeof candidate !== "object" || Array.isArray(candidate)) {
      merged[id] = { ...defaults };
      continue;
    }
    merged[id] = {
      ...candidate,
      enabled: candidate.enabled === true,
      anchor: validAnchors.includes(candidate.anchor) ? candidate.anchor : defaults.anchor,
      offset: typeof candidate.offset === "string" && candidate.offset.trim() ? candidate.offset : defaults.offset,
      scale: Number.isInteger(candidate.scale) && candidate.scale >= UTILITY_SCALE_MIN && candidate.scale <= UTILITY_SCALE_MAX ? candidate.scale : defaults.scale,
      opacity: Number.isInteger(candidate.opacity) && candidate.opacity >= 25 && candidate.opacity <= 100 ? candidate.opacity : defaults.opacity,
    };
  }
  return { ...settings, schemaVersion: 1, utilities: merged };
}
