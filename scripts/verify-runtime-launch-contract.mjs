import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFileSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const [bootstrapRootArgument] = process.argv.slice(2);
if (!bootstrapRootArgument) {
  throw new Error("Usage: node verify-runtime-launch-contract.mjs <bootstrap-resources>");
}

const bootstrapRoot = resolve(bootstrapRootArgument);
const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const forgeSource = resolve(scriptDirectory, "../crates/engine/src/forge.rs");

function readConstant(source, name, pattern) {
  const match = source.match(pattern);
  if (!match) {
    throw new Error(`Launcher Forge contract is missing ${name}`);
  }
  return match[1];
}

function readZipEntry(archive, entry) {
  try {
    return execFileSync("unzip", ["-p", archive, entry]);
  } catch {
    throw new Error(`Runtime artifact is missing ZIP entry ${entry}: ${archive}`);
  }
}

const forge = readFileSync(forgeSource, "utf8");
const expected = {
  coremod: {
    sha1: readConstant(
      forge,
      "FORGE_COREMOD_SHA1",
      /FORGE_COREMOD_SHA1:\s*&str\s*=\s*"([0-9a-f]{40})"/,
    ),
    size: Number(
      readConstant(
        forge,
        "FORGE_COREMOD_SIZE",
        /FORGE_COREMOD_SIZE:\s*u64\s*=\s*([0-9_]+)/,
      ).replaceAll("_", ""),
    ),
  },
  nativeUi: {
    sha1: readConstant(
      forge,
      "FORGE_NATIVE_UI_SHA1",
      /FORGE_NATIVE_UI_SHA1:\s*&str\s*=\s*"([0-9a-f]{40})"/,
    ),
    size: Number(
      readConstant(
        forge,
        "FORGE_NATIVE_UI_SIZE",
        /FORGE_NATIVE_UI_SIZE:\s*u64\s*=\s*([0-9_]+)/,
      ).replaceAll("_", ""),
    ),
  },
  opusconfig: {
    sha1: readConstant(
      forge,
      "FORGE_OPUSCONFIG_SHA1",
      /FORGE_OPUSCONFIG_SHA1:\s*&str\s*=\s*"([0-9a-f]{40})"/,
    ),
    size: Number(
      readConstant(
        forge,
        "FORGE_OPUSCONFIG_SIZE",
        /FORGE_OPUSCONFIG_SIZE:\s*u64\s*=\s*([0-9_]+)/,
      ).replaceAll("_", ""),
    ),
  },
  elementa: {
    sha1: readConstant(
      forge,
      "FORGE_ELEMENTA_SHA1",
      /FORGE_ELEMENTA_SHA1:\s*&str\s*=\s*"([0-9a-f]{40})"/,
    ),
    size: Number(
      readConstant(
        forge,
        "FORGE_ELEMENTA_SIZE",
        /FORGE_ELEMENTA_SIZE:\s*u64\s*=\s*([0-9_]+)/,
      ).replaceAll("_", ""),
    ),
  },
  universalcraft: {
    sha1: readConstant(
      forge,
      "FORGE_UNIVERSALCRAFT_SHA1",
      /FORGE_UNIVERSALCRAFT_SHA1:\s*&str\s*=\s*"([0-9a-f]{40})"/,
    ),
    size: Number(
      readConstant(
        forge,
        "FORGE_UNIVERSALCRAFT_SIZE",
        /FORGE_UNIVERSALCRAFT_SIZE:\s*u64\s*=\s*([0-9_]+)/,
      ).replaceAll("_", ""),
    ),
  },
  lwjglMacosCompat: {
    sha1: readConstant(
      forge,
      "FORGE_LWJGL_MACOS_COMPAT_SHA1",
      /FORGE_LWJGL_MACOS_COMPAT_SHA1:\s*&str\s*=\s*"([0-9a-f]{40})"/,
    ),
    size: Number(
      readConstant(
        forge,
        "FORGE_LWJGL_MACOS_COMPAT_SIZE",
        /FORGE_LWJGL_MACOS_COMPAT_SIZE:\s*u64\s*=\s*([0-9_]+)/,
      ).replaceAll("_", ""),
    ),
    nativeSha1: readConstant(
      forge,
      "FORGE_LWJGL_MACOS_NATIVE_SHA1",
      /FORGE_LWJGL_MACOS_NATIVE_SHA1:\s*\&str\s*=\s*"([0-9a-f]{40})"/,
    ),
    nativeSize: Number(
      readConstant(
        forge,
        "FORGE_LWJGL_MACOS_NATIVE_SIZE",
        /FORGE_LWJGL_MACOS_NATIVE_SIZE:\s*u64\s*=\s*([0-9_]+)/,
      ).replaceAll("_", ""),
    ),
    guardSha1: readConstant(
      forge,
      "FORGE_LWJGL_RESIZE_GUARD_SHA1",
      /FORGE_LWJGL_RESIZE_GUARD_SHA1:\s*&str\s*=\s*"([0-9a-f]{40})"/,
    ),
    guardSize: Number(
      readConstant(
        forge,
        "FORGE_LWJGL_RESIZE_GUARD_SIZE",
        /FORGE_LWJGL_RESIZE_GUARD_SIZE:\s*u64\s*=\s*([0-9_]+)/,
      ).replaceAll("_", ""),
    ),
  },
};

const manifest = JSON.parse(
  readFileSync(join(bootstrapRoot, "runtime-manifest.json"), "utf8"),
);
const checksums = JSON.parse(
  readFileSync(join(bootstrapRoot, "runtime-checksums.json"), "utf8"),
);
if (
  manifest.schemaVersion !== 1 ||
  manifest.protocolVersion !== 1 ||
  manifest.minecraftVersion !== "1.8.9" ||
  !Array.isArray(manifest.artifacts) ||
  manifest.artifacts.length !== 7 ||
  checksums.schemaVersion !== 1 ||
  checksums.algorithm !== "SHA-256" ||
  checksums.files === null ||
  typeof checksums.files !== "object"
) {
  throw new Error("Unsupported OPUS Runtime artifact contract");
}

const records = new Map();
for (const artifact of manifest.artifacts) {
  if (!artifact || typeof artifact !== "object" || records.has(artifact.role)) {
    throw new Error("Runtime manifest has duplicate or invalid artifact roles");
  }
  records.set(artifact.role, artifact);
}

const required = [
  ["runtime-legacy-1.8.9", "coremod", "opus-runtime-legacy-1.8.9-", null],
  [
    "lwjgl-macos-compat",
    "lwjglMacosCompat",
    null,
    "opus-lwjgl-macos-compat-2.9.2-nightly-20140822.jar",
  ],
  ["native-ui", "nativeUi", null, "opus-native-ui-1.8.9-0.1.0.jar"],
  [
    "opusconfig",
    "opusconfig",
    null,
    "opus-oneconfig-1.8.9-forge-0.2.2-alpha228-full.jar",
  ],
  ["elementa", "elementa", null, "opus-elementa-762.jar"],
  [
    "universalcraft",
    "universalcraft",
    null,
    "opus-universalcraft-1.8.9-forge-516.jar",
  ],
];
for (const [role, contract, namePrefix, exactName] of required) {
  const artifact = records.get(role);
  const validName =
    typeof artifact?.file === "string" &&
    (exactName
      ? artifact.file === exactName
      : artifact.file.startsWith(namePrefix) && artifact.file.endsWith(".jar"));
  if (
    !artifact ||
    !validName ||
    !Number.isSafeInteger(artifact.size) ||
    artifact.size <= 0 ||
    !/^[0-9a-f]{64}$/.test(artifact.sha256)
  ) {
    throw new Error(`Runtime manifest has an invalid ${role} record`);
  }

  const path = join(bootstrapRoot, artifact.file);
  const bytes = readFileSync(path);
  const actualSha256 = createHash("sha256").update(bytes).digest("hex");
  const actualSha1 = createHash("sha1").update(bytes).digest("hex");
  if (
    bytes.length !== artifact.size ||
    actualSha256 !== artifact.sha256 ||
    checksums.files[artifact.file] !== actualSha256
  ) {
    throw new Error(`Runtime artifact integrity mismatch: ${artifact.file}`);
  }
  if (actualSha1 !== expected[contract].sha1 || bytes.length !== expected[contract].size) {
    throw new Error(
      `Launcher Forge contract does not match ${artifact.file}: ` +
        `expected ${expected[contract].sha1}/${expected[contract].size}, ` +
        `got ${actualSha1}/${bytes.length}`,
    );
  }

  if (role === "lwjgl-macos-compat") {
    for (const requiredEntry of [
      "org/lwjgl/MacOSXSysImplementation.class",
      "org/lwjgl/J2SESysImplementation.class",
      "org/lwjgl/DefaultSysImplementation.class",
      "org/lwjgl/SysImplementation.class",
      "org/lwjgl/opengl/MacOSXDisplay.class",
      "org/lwjgl/opengl/MacOSXCanvasPeerInfo.class",
      "org/lwjgl/opengl/MacOSXCanvasPeerInfo$1.class",
      "org/lwjgl/opengl/MacOSXDisplayPeerInfo.class",
    ]) {
      readZipEntry(path, requiredEntry);
    }
    const displaySymbols = readZipEntry(
      path,
      "org/lwjgl/opengl/MacOSXDisplay.class",
    ).toString("latin1");
    const peerInfoSymbols = readZipEntry(
      path,
      "org/lwjgl/opengl/MacOSXCanvasPeerInfo.class",
    ).toString("latin1");
    if (
      !displaySymbols.includes("OPUS_MODERN_MACOS_DISPLAY_COMPAT_V1") ||
      !displaySymbols.includes("OPUS_COCOA_LIVE_RESIZE_HANDSHAKE_V10_APPKIT_DRAWABLE_UPDATE") ||
      !displaySymbols.includes("opus.lwjgl.macos.liveResizeGuard") ||
      !peerInfoSymbols.includes("OPUS_MODERN_MACOS_PIXEL_FORMAT_COMPAT_V2")
    ) {
      throw new Error(
        `Embedded LWJGL Java compatibility patch is missing: ${artifact.file}`,
      );
    }
    const native = readZipEntry(path, "liblwjgl.dylib");
    const nativeSha1 = createHash("sha1").update(native).digest("hex");
    const isMachO =
      native.subarray(0, 4).equals(Buffer.from([0xca, 0xfe, 0xba, 0xbe])) ||
      native.subarray(0, 4).equals(Buffer.from([0xcf, 0xfa, 0xed, 0xfe]));
    if (
      native.length !== expected[contract].nativeSize ||
      nativeSha1 !== expected[contract].nativeSha1 ||
      !isMachO
    ) {
      throw new Error(
        `Embedded macOS LWJGL native integrity mismatch: ${artifact.file}`,
      );
    }
    const guard = readZipEntry(path, "libopus_lwjgl_resize_guard.dylib");
    const guardSha1 = createHash("sha1").update(guard).digest("hex");
    const guardIsMachO = guard
      .subarray(0, 4)
      .equals(Buffer.from([0xcf, 0xfa, 0xed, 0xfe]));
    if (
      guard.length !== expected[contract].guardSize ||
      guardSha1 !== expected[contract].guardSha1 ||
      !guardIsMachO ||
      !guard
        .toString("latin1")
        .includes("OPUS_COCOA_LIVE_RESIZE_GUARD_V10_MAIN_QUEUE_INSTALL")
    ) {
      throw new Error(
        `Embedded Cocoa live-resize guard integrity mismatch: ${artifact.file}`,
      );
    }
  }
}

const bootstrap = records.get("bootstrap");
if (
  !bootstrap ||
  typeof bootstrap.file !== "string" ||
  !bootstrap.file.startsWith("opus-bootstrap-") ||
  !bootstrap.file.endsWith(".jar")
) {
  throw new Error("Runtime manifest has an invalid bootstrap record");
}
const bootstrapPath = join(bootstrapRoot, bootstrap.file);
if (!statSync(bootstrapPath).isFile()) {
  throw new Error(`Runtime bootstrap artifact is missing: ${bootstrapPath}`);
}

console.log(
  "Runtime launch contract verified: Core Mod + LWJGL macOS compatibility + native UI + OpusConfig + Elementa + UniversalCraft",
);
