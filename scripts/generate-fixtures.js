#!/usr/bin/env node
import { execSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const sdkDir = path.resolve(__dirname, "..", "packages", "typescript-sdk");

console.log("==> Running fixture regeneration in packages/typescript-sdk...");
execSync("npm run regen:fixtures", {
  cwd: sdkDir,
  stdio: "inherit",
});
