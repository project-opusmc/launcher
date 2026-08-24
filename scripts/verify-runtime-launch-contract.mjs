import { createHash } from "node:crypto";
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
  manifest.artifacts.length !== 2 ||
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
  ["runtime-legacy-1.8.9", "coremod", "opus-runtime-legacy-1.8.9-"],
];
for (const [role, contract, namePrefix] of required) {
  const artifact = records.get(role);
  if (
    !artifact ||
    typeof artifact.file !== "string" ||
    !artifact.file.startsWith(namePrefix) ||
    !artifact.file.endsWith(".jar") ||
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
  `Runtime launch contract verified: ${expected.coremod.size} byte Core Mod`,
);
