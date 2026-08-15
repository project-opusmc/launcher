import { createHash } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
} from "node:fs";
import { basename, join, resolve } from "node:path";

const [runtimeRootArgument, destinationArgument] = process.argv.slice(2);
if (!runtimeRootArgument || !destinationArgument) {
  throw new Error(
    "Usage: node stage-runtime-artifacts.mjs <runtime-output> <destination>",
  );
}

const runtimeRoot = resolve(runtimeRootArgument);
const destination = resolve(destinationArgument);
const manifestPath = join(runtimeRoot, "runtime-manifest.json");
const checksumsPath = join(runtimeRoot, "runtime-checksums.json");

const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const checksums = JSON.parse(readFileSync(checksumsPath, "utf8"));
const expectedRoles = new Set([
  "bootstrap",
  "runtime-legacy-1.8.9",
  "client-legacy-1.8.9",
]);

if (
  manifest.schemaVersion !== 1 ||
  manifest.protocolVersion !== 1 ||
  manifest.minecraftVersion !== "1.8.9" ||
  typeof manifest.runtimeVersion !== "string" ||
  !Array.isArray(manifest.artifacts) ||
  manifest.artifacts.length !== expectedRoles.size
) {
  throw new Error("Unsupported OPUS Runtime manifest contract");
}
if (
  checksums.schemaVersion !== 1 ||
  checksums.algorithm !== "SHA-256" ||
  typeof checksums.files !== "object" ||
  checksums.files === null
) {
  throw new Error("Unsupported OPUS Runtime checksum contract");
}

mkdirSync(destination, { recursive: true });
for (const entry of readdirSync(destination, { withFileTypes: true })) {
  if (entry.isFile() && (entry.name.endsWith(".jar") || entry.name.startsWith("runtime-"))) {
    rmSync(join(destination, entry.name));
  }
}

const seenRoles = new Set();
for (const artifact of manifest.artifacts) {
  if (
    typeof artifact !== "object" ||
    artifact === null ||
    !expectedRoles.has(artifact.role) ||
    seenRoles.has(artifact.role) ||
    typeof artifact.file !== "string" ||
    basename(artifact.file) !== artifact.file ||
    !artifact.file.startsWith("opus-") ||
    !artifact.file.endsWith(".jar") ||
    !Number.isSafeInteger(artifact.size) ||
    artifact.size <= 0 ||
    typeof artifact.sha256 !== "string" ||
    !/^[0-9a-f]{64}$/.test(artifact.sha256)
  ) {
    throw new Error("Invalid OPUS Runtime artifact record");
  }

  const source = join(runtimeRoot, "artifacts", artifact.file);
  if (!existsSync(source) || !statSync(source).isFile()) {
    throw new Error(`Missing OPUS Runtime artifact: ${source}`);
  }
  const bytes = readFileSync(source);
  const actualSha256 = createHash("sha256").update(bytes).digest("hex");
  if (
    bytes.length !== artifact.size ||
    actualSha256 !== artifact.sha256 ||
    checksums.files[artifact.file] !== actualSha256
  ) {
    throw new Error(`OPUS Runtime artifact integrity mismatch: ${artifact.file}`);
  }

  copyFileSync(source, join(destination, artifact.file));
  seenRoles.add(artifact.role);
}

if (seenRoles.size !== expectedRoles.size) {
  throw new Error("OPUS Runtime artifact roles are incomplete");
}

copyFileSync(manifestPath, join(destination, "runtime-manifest.json"));
copyFileSync(checksumsPath, join(destination, "runtime-checksums.json"));
