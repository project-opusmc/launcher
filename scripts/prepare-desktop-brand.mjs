import { copyFileSync, cpSync, existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const launcherRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const desktopRoot = join(launcherRoot, "desktop");
const logo = join(desktopRoot, "public/brand/opus-woven-o-c3.svg");
const appIcon = join(desktopRoot, "public/brand/opus-app-icon-c3-black.svg");
const clientLogo = resolve(launcherRoot, "../ui-v2/public/brand/opus-woven-o-c3.svg");

// Use the approved client mark in a superproject checkout. The local copy
// keeps standalone launcher builds independent of the client source tree.
mkdirSync(dirname(logo), { recursive: true });
if (existsSync(clientLogo)) copyFileSync(clientLogo, logo);
if (!existsSync(logo)) throw new Error(`Missing approved Opus logo: ${logo}`);
if (!existsSync(appIcon)) throw new Error(`Missing approved black Opus app icon: ${appIcon}`);

const temporary = mkdtempSync(join(tmpdir(), "opus-brand-icons-"));
const tauri = join(desktopRoot, "node_modules/@tauri-apps/cli/tauri.js");
try {
  for (const extra of [[], ["--png", "64"]]) {
    const result = spawnSync(process.execPath, [tauri, "icon", appIcon, "--output", temporary, ...extra], {
      cwd: desktopRoot,
      encoding: "utf8",
    });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(result.stderr || result.stdout || "Opus icon export failed");
  }
  const icons = join(desktopRoot, "src-tauri/icons");
  cpSync(temporary, icons, { recursive: true });
  const gameResources = join(desktopRoot, "src-tauri/resources/Minecraft.app/Contents/Resources");
  mkdirSync(gameResources, { recursive: true });
  copyFileSync(join(temporary, "icon.icns"), join(gameResources, "Opus.icns"));
  console.log("Opus C3 branding: approved black app icon exported for launcher and game app");
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
