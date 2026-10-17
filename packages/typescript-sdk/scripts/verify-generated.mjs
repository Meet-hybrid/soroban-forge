import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const sdkDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const rootDir = resolve(sdkDir, "../..");
const generatedPaths = [
  "packages/typescript-sdk/src/clients",
  "packages/typescript-sdk/src/abi-fixtures",
  "packages/vesting-client/src/index.ts",
  "packages/multi-sig-wallet-client/src/index.ts",
  "packages/dao-governance-client/src/index.ts",
  "packages/subscription-payments-client/src/index.ts",
  "packages/marketplace-royalties-client/src/index.ts",
];

const result = spawnSync(
  "git",
  ["diff", "--exit-code", "--", ...generatedPaths],
  { cwd: rootDir, stdio: "inherit" },
);
if (result.error) {
  throw result.error;
}
if (result.status !== 0) {
  console.error(
    "Generated clients or ABI fixtures differ from the contract artifacts. Run `npm run generate`, review the output, and commit the changes.",
  );
  process.exit(result.status ?? 1);
}
console.log("Generated clients and ABI fixtures match the contract artifacts.");
