import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Spec } from "@stellar/stellar-sdk/contract";

const sdkDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const clientsDir = join(sdkDir, "src", "clients");
const fixturesDir = join(sdkDir, "src", "abi-fixtures");

const contracts = [
  ["escrow", "soroban-forge-escrow"],
  ["vesting", "soroban-forge-vesting"],
  ["multisig", "soroban-forge-multi-sig-wallet"],
  ["dao", "soroban-forge-dao-governance"],
  ["subscription", "soroban-forge-subscription-payments"],
  ["marketplace", "soroban-forge-marketplace-royalties"],
];

function compareName(left, right) {
  return left < right ? -1 : left > right ? 1 : 0;
}

function parseSpec(source, path) {
  const match = source.match(/new ContractSpec\(\s*\[([\s\S]*?)\]\s*\)/);
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
  return new Spec(encodedEntries);
}

function typeName(type) {
  const kind = type.switch().name;
  const simpleName = kind.replace(/^scSpecType/, "");

  if (simpleName === "Udt") {
    return `udt:${type.udt().name().toString()}`;
  }
  if (simpleName === "Vec") {
    return `vec<${typeName(type.vec().elementType())}>`;
  }
  if (simpleName === "Option") {
    return `option<${typeName(type.option().valueType())}>`;
  }
  if (simpleName === "Result") {
    const result = type.result();
    return `result<${typeName(result.okType())},${typeName(result.errorType())}>`;
  }
  if (simpleName === "Map") {
    const map = type.map();
    return `map<${typeName(map.keyType())},${typeName(map.valueType())}>`;
  }
  if (simpleName === "Tuple") {
    return `tuple<${type.tuple().valueTypes().map(typeName).join(",")}>`;
  }
  return simpleName[0].toLowerCase() + simpleName.slice(1);
}

function normalizeFunction(fn) {
  return {
    name: fn.name().toString(),
    inputs: fn.inputs().map((input) => ({
      name: input.name().toString(),
      type: typeName(input.type()),
    })),
    outputs: fn.outputs().map(typeName),
  };
}

function normalizeEvent(event) {
  return {
    name: event.name().toString(),
    dataFormat: event.dataFormat().name,
    params: event.params().map((param) => ({
      name: param.name().toString(),
      type: typeName(param.type()),
    })),
  };
}

function normalizeUdt(entry) {
  const entryKind = entry.switch().name;
  const udt = entry.value();
  const name = udt.name().toString();

  if (entryKind === "scSpecEntryUdtStructV0") {
    return {
      kind: "struct",
      name,
      fields: udt.fields().map((field) => ({
        name: field.name().toString(),
        type: typeName(field.type()),
      })),
    };
  }

  if (entryKind === "scSpecEntryUdtEnumV0" || entryKind === "scSpecEntryUdtErrorEnumV0") {
    return {
      kind: entryKind === "scSpecEntryUdtErrorEnumV0" ? "error-enum" : "enum",
      name,
      variants: udt.cases().map((variant) => ({
        name: variant.name().toString(),
        value: Number(variant.value().toString()),
      })),
    };
  }

  if (entryKind === "scSpecEntryUdtUnionV0") {
    return {
      kind: "union",
      name,
      variants: udt.cases().map((variant) => {
        const variantKind = variant.switch().name;
        if (variantKind === "scSpecUdtUnionCaseVoidV0") {
          return { name: variant.voidCase().name().toString(), types: [] };
        }
        if (variantKind === "scSpecUdtUnionCaseTupleV0") {
          const tupleCase = variant.tupleCase();
          return {
            name: tupleCase.name().toString(),
            types: tupleCase.type().map(typeName),
          };
        }
        throw new Error(`Unsupported union variant ${variantKind} in ${name}`);
      }),
    };
  }

  throw new Error(`Unsupported contract spec entry ${entryKind}`);
}

function createFixture(module, contract) {
  const sourcePath = join(clientsDir, `${module}.ts`);
  const spec = parseSpec(readFileSync(sourcePath, "utf8"), sourcePath);
  const functions = spec.funcs()
    .map(normalizeFunction)
    .sort((left, right) => compareName(left.name, right.name));
  const events = spec.events()
    .map(normalizeEvent)
    .sort((left, right) => compareName(left.name, right.name));
  const types = spec.entries
    .filter((entry) => entry.switch().name.startsWith("scSpecEntryUdt"))
    .map(normalizeUdt)
    .sort((left, right) => compareName(left.name, right.name));
  const errors = spec.errorCases()
    .map((error) => ({
      name: error.name().toString(),
      code: Number(error.value().toString()),
    }))
    .sort((left, right) => left.code - right.code);

  if (functions.length === 0 || types.length === 0 || errors.length === 0) {
    throw new Error(`Incomplete generated contract spec for ${contract}`);
  }
  return {
    schema: "soroban-forge/contract-abi@1",
    contract,
    functions,
    events,
    types,
    errors,
  };
}

mkdirSync(fixturesDir, { recursive: true });
for (const [module, contract] of contracts) {
  const fixturePath = join(fixturesDir, `${module}.json`);
  const fixture = createFixture(module, contract);
  writeFileSync(fixturePath, `${JSON.stringify(fixture, null, 2)}\n`);
  console.log(`Wrote ${fixturePath}`);
}
