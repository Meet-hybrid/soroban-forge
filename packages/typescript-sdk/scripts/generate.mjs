import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { xdr } from "@stellar/stellar-sdk";

const sdkDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const rootDir = resolve(sdkDir, "../..");
const target = "wasm32v1-none";
const releaseDir = join(rootDir, "target", target, "release");
const generatedDir = join(rootDir, "target", "generated-typescript-clients");
const sdkClientsDir = join(sdkDir, "src", "clients");

const escrowPaginationHelper = `

/**
 * Automatically pages through participant escrows using a page provider function.
 * Stops cleanly when next_cursor is null/undefined and preserves on-chain creation order.
 */
export async function paginateParticipantEscrows(
  fetchPage: (cursor: u32, limit: u32) => Promise<ParticipantEscrowsPage>,
  limit: u32 = 50,
): Promise<Array<u64>> {
  const ids: Array<u64> = [];
  let cursor: u32 | undefined = 0;

  while (cursor !== undefined && cursor !== null) {
    const page = await fetchPage(cursor, limit);
    ids.push(...page.ids);
    if (page.next_cursor === null || page.next_cursor === undefined) {
      break;
    }
    cursor = page.next_cursor;
  }

  return ids;
}
`;

const contracts = [
  {
    crate: "soroban-forge-escrow",
    wasm: "soroban_forge_escrow.wasm",
    module: "escrow",
  },
  {
    crate: "soroban-forge-vesting",
    wasm: "soroban_forge_vesting.wasm",
    module: "vesting",
    package: "vesting-client",
  },
  {
    crate: "soroban-forge-multi-sig-wallet",
    wasm: "soroban_forge_multi_sig_wallet.wasm",
    module: "multisig",
    package: "multi-sig-wallet-client",
  },
  {
    crate: "soroban-forge-marketplace-royalties",
    wasm: "soroban_forge_marketplace_royalties.wasm",
    module: "marketplace",
    package: "marketplace-royalties-client",
  },
  {
    crate: "soroban-forge-subscription-payments",
    wasm: "soroban_forge_subscription_payments.wasm",
    module: "subscription",
    package: "subscription-payments-client",
  },
  {
    crate: "soroban-forge-dao-governance",
    wasm: "soroban_forge_dao_governance.wasm",
    module: "dao",
    package: "dao-governance-client",
  },
];

function run(command, args, cwd = rootDir) {
  const result = spawnSync(command, args, { cwd, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} exited with status ${result.status}`);
  }
}

function compareEncoded(left, right) {
  return left < right ? -1 : left > right ? 1 : 0;
}

function specEntryName(entry, path) {
  const kind = entry.switch().name;
  if (kind === "scSpecEntryFunctionV0") {
    return entry.functionV0().name().toString();
  }
  if (kind === "scSpecEntryUdtStructV0") {
    return entry.udtStructV0().name().toString();
  }
  if (kind === "scSpecEntryUdtEnumV0") {
    return entry.udtEnumV0().name().toString();
  }
  if (kind === "scSpecEntryUdtUnionV0") {
    return entry.udtUnionV0().name().toString();
  }
  if (kind === "scSpecEntryUdtErrorEnumV0") {
    return entry.udtErrorEnumV0().name().toString();
  }
  if (kind === "scSpecEntryEventV0") {
    return entry.eventV0().name().toString();
  }
  throw new Error(`Unsupported generated ContractSpec entry ${kind} in ${path}`);
}

function specEntryRank(kind, path) {
  if (
    kind === "scSpecEntryUdtStructV0" ||
    kind === "scSpecEntryUdtEnumV0" ||
    kind === "scSpecEntryUdtUnionV0"
  ) {
    return 0;
  }
  if (kind === "scSpecEntryFunctionV0") return 1;
  if (kind === "scSpecEntryEventV0") return 2;
  if (kind === "scSpecEntryUdtErrorEnumV0") return 3;
  throw new Error(`Unsupported generated ContractSpec entry ${kind} in ${path}`);
}

function compareSpecEntries(left, right, path) {
  return (
    specEntryRank(left.kind, path) - specEntryRank(right.kind, path) ||
    compareEncoded(left.name, right.name) ||
    compareEncoded(left.encoded, right.encoded)
  );
}

function canonicalizeSpec(source, path) {
  const pattern = /new ContractSpec\(\s*\[([\s\S]*?)\]\s*\)/;
  const match = source.match(pattern);
  if (!match) {
    throw new Error(`Unable to find generated ContractSpec in ${path}`);
  }

  const encodedEntries = [...match[1].matchAll(/"([A-Za-z0-9+/=]+)"/g)].map(
    ([, encoded]) => encoded,
  );
  const remaining = match[1]
    .replace(/"([A-Za-z0-9+/=]+)"/g, "")
    .replace(/[\s,]/g, "");
  if (encodedEntries.length === 0 || remaining.length > 0) {
    throw new Error(`Unexpected generated ContractSpec format in ${path}`);
  }

  const entries = encodedEntries.map((encoded) => {
    const entry = xdr.ScSpecEntry.fromXDR(encoded, "base64");
    const kind = entry.switch().name;
    return { encoded, kind, name: specEntryName(entry, path) };
  });
  const replacement = `new ContractSpec([ ${entries
    .sort((left, right) => compareSpecEntries(left, right, path))
    .map(({ encoded }) => `"${encoded}"`)
    .join(",\n        ")} ])`;
  return source.replace(pattern, () => replacement);
}

console.log(`==> Building contract WASMs for target '${target}'`);
run("stellar", [
  "contract",
  "build",
  "--locked",
  "--profile",
  "release",
  "--optimize=false",
]);

mkdirSync(sdkClientsDir, { recursive: true });

for (const contract of contracts) {
  const wasmPath = join(releaseDir, contract.wasm);
  if (!existsSync(wasmPath)) {
    throw new Error(`Expected WASM missing: ${wasmPath}`);
  }

  const outputDir = join(generatedDir, contract.module);
  rmSync(outputDir, { recursive: true, force: true });
  mkdirSync(outputDir, { recursive: true });
  console.log(`==> Generating ${contract.module} client from ${wasmPath}`);
  run("stellar", [
    "contract",
    "bindings",
    "typescript",
    "--wasm",
    wasmPath,
    "--output-dir",
    outputDir,
    "--overwrite",
  ]);

  const generatedSource = join(outputDir, "src", "index.ts");
  if (!existsSync(generatedSource)) {
    throw new Error(`Expected generated binding missing: ${generatedSource}`);
  }

  const generatedText = canonicalizeSpec(
    readFileSync(generatedSource, "utf8").replace(/[ \t]+(?=\r?$)/gm, ""),
    generatedSource,
  );
  if (!generatedText.includes("export class Client extends ContractClient {")) {
    throw new Error(`Expected generated Client class in ${generatedSource}`);
  }
  const finalText =
    contract.module === "escrow"
      ? `${generatedText}${escrowPaginationHelper}`
      : generatedText;
  writeFileSync(generatedSource, finalText);
  cpSync(generatedSource, join(sdkClientsDir, `${contract.module}.ts`));

  if (contract.package) {
    const packageSourceDir = join(rootDir, "packages", contract.package, "src");
    mkdirSync(packageSourceDir, { recursive: true });
    cpSync(generatedSource, join(packageSourceDir, "index.ts"));
  }
}

console.log(`==> All six clients generated in ${sdkClientsDir} and standalone packages.`);
