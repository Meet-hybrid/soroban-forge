/**
 * Centralized verification for the shared ForgeError registry.
 *
 * Verifies that all 13 documented ForgeError codes (1–13) are consistently
 * registered across:
 *   1. `FORGE_ERRORS` registry map and `forgeErrorName()` helper
 *   2. The runtime `ForgeError` object emitted in contract clients
 *   3. The root `errors.json` machine-readable specification
 *
 * Run:
 *   npm test
 */

import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { fileURLToPath } from "node:url";

import {
  FORGE_ERRORS,
  forgeErrorName,
  ForgeError,
} from "../../dist/index.js";

const EXPECTED_FORGE_ERROR_CODES = [
  1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13,
];

const EXPECTED_FORGE_ERROR_MAP: Record<number, string> = {
  1: "Unauthorized",
  2: "NotFound",
  3: "InvalidInput",
  4: "InsufficientFunds",
  5: "AlreadyInitialized",
  6: "NotInitialized",
  7: "DeadlineReached",
  8: "InsufficientAllowance",
  9: "ArithmeticOverflow",
  10: "Custom",
  11: "TokenTransferFailed",
  12: "ContractInvocationFailed",
  13: "WithdrawalLimitExceeded",
};

test("FORGE_ERRORS map exposes exactly the 13 documented error codes (1–13)", () => {
  const codes = Object.keys(FORGE_ERRORS)
    .map(Number)
    .sort((a, b) => a - b);
  assert.deepEqual(
    codes,
    EXPECTED_FORGE_ERROR_CODES,
    "FORGE_ERRORS keys must match codes 1 through 13",
  );
});

test("FORGE_ERRORS maps each code to its canonical enum name", () => {
  for (const [codeStr, expectedName] of Object.entries(EXPECTED_FORGE_ERROR_MAP)) {
    const code = Number(codeStr);
    assert.equal(
      FORGE_ERRORS[code],
      expectedName,
      `FORGE_ERRORS[${code}] must be "${expectedName}"`,
    );
  }
});

test("forgeErrorName() resolves names for valid codes and returns undefined for unknowns", () => {
  for (const [codeStr, expectedName] of Object.entries(EXPECTED_FORGE_ERROR_MAP)) {
    const code = Number(codeStr);
    assert.equal(
      forgeErrorName(code),
      expectedName,
      `forgeErrorName(${code}) should return "${expectedName}"`,
    );
  }

  // Unknown codes
  assert.equal(forgeErrorName(0), undefined, "code 0 (reserved by host) must be undefined");
  assert.equal(forgeErrorName(14), undefined, "code 14 must be undefined");
  assert.equal(forgeErrorName(999), undefined, "code 999 must be undefined");
});

test("Runtime ForgeError object is exported and is a non-null, non-array object", () => {
  assert.equal(
    typeof ForgeError,
    "object",
    "ForgeError must be a plain runtime object",
  );
  assert.ok(ForgeError !== null);
  assert.ok(!Array.isArray(ForgeError));
});

test("Runtime ForgeError object exposes all 13 error codes with matching messages", () => {
  const codes = Object.keys(ForgeError)
    .map(Number)
    .sort((a, b) => a - b);
  assert.deepEqual(codes, EXPECTED_FORGE_ERROR_CODES);

  for (const [codeStr, expectedName] of Object.entries(EXPECTED_FORGE_ERROR_MAP)) {
    const code = Number(codeStr);
    const entry = ForgeError[code as keyof typeof ForgeError];
    assert.ok(entry, `ForgeError[${code}] entry must exist`);
    assert.equal(
      entry.message,
      expectedName,
      `ForgeError[${code}].message mismatch`,
    );
  }
});

test("FORGE_ERRORS matches errors.json registry if present", () => {
  const possiblePaths = [
    new URL("../../../../errors.json", import.meta.url),
    new URL("../../../errors.json", import.meta.url),
  ];

  let rawJson: string | null = null;
  for (const p of possiblePaths) {
    const filePath = fileURLToPath(p);
    if (fs.existsSync(filePath)) {
      rawJson = fs.readFileSync(filePath, "utf8");
      break;
    }
  }

  if (rawJson) {
    const cleanJson = rawJson.replace(/\r/g, "");
    const parsed = JSON.parse(cleanJson) as {
      schema: number;
      errors: Array<{ code: number; name: string; doc: string }>;
    };
    assert.equal(parsed.errors.length, 13, "errors.json should define 13 error codes");
    for (const err of parsed.errors) {
      assert.equal(
        FORGE_ERRORS[err.code],
        err.name,
        `errors.json code ${err.code} name "${err.name}" must match FORGE_ERRORS`,
      );
    }
  }
});
