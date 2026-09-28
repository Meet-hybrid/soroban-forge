import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { contract } from "@stellar/stellar-sdk";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// Base directory for typescript-sdk
const sdkDir = path.resolve(__dirname, "..");
const generatedDir = path.join(sdkDir, "src", "generated");
const fixturesDir = path.join(sdkDir, "src", "fixtures");

if (!fs.existsSync(fixturesDir)) {
  fs.mkdirSync(fixturesDir, { recursive: true });
}

export interface ContractFixture {
  contract: string;
  functions: Array<{
    name: string;
    inputs: Array<{
      name: string;
      type: string;
    }>;
  }>;
  events: Array<string>;
}

const CONTRACT_NAMES = [
  "escrow",
  "subscription-payments",
  "multi-sig-wallet",
  "marketplace-royalties",
  "dao-governance",
  "vesting",
] as const;

export function extractSpecEntries(code: string): Array<string> {
  const match = code.match(/new ContractSpec\(\[\s*([\s\S]*?)\s*\]\)/);
  if (!match || !match[1]) {
    throw new Error("Failed to find 'new ContractSpec([...])' in generated source");
  }
  // Safely parse the string array literals
  const entries: Array<string> = (0, eval)(`[${match[1]}]`);
  return entries;
}

export function generateFixtureForContract(name: string): ContractFixture {
  const sourcePath = path.join(generatedDir, `${name}.ts`);
  if (!fs.existsSync(sourcePath)) {
    throw new Error(`Generated contract file not found: ${sourcePath}`);
  }
  const code = fs.readFileSync(sourcePath, "utf8");
  const entries = extractSpecEntries(code);
  const spec = new contract.Spec(entries);

  const fixture: ContractFixture = {
    contract: name,
    functions: spec.funcs().map((fn) => ({
      name: fn.name().toString(),
      inputs: fn.inputs().map((input) => ({
        name: input.name().toString(),
        type: input.type().switch().name,
      })),
    })),
    events: spec.events().map((e) => e.name().toString()),
  };

  return fixture;
}

export function regenerateAllFixtures(): void {
  console.log("==> Regenerating contract fixtures...");
  for (const name of CONTRACT_NAMES) {
    const fixture = generateFixtureForContract(name);
    const targetFile = path.join(fixturesDir, `${name}.fixture.json`);
    fs.writeFileSync(targetFile, JSON.stringify(fixture, null, 2) + "\n", "utf8");
    console.log(`    Wrote ${path.relative(sdkDir, targetFile)} (${fixture.functions.length} functions, ${fixture.events.length} events)`);
  }
  console.log("==> All fixtures successfully regenerated.");
}

// Execute when invoked directly
regenerateAllFixtures();
