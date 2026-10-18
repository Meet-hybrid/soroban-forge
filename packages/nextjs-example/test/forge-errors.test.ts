import test from "node:test";
import assert from "node:assert/strict";
import {
  FORGE_ERRORS,
  forgeErrorName,
  parseForgeError,
  decodeContractEvents,
} from "../src/lib/forge-errors.ts";

test("FORGE_ERRORS defines all 13 standard ForgeError codes", () => {
  assert.equal(FORGE_ERRORS[1].name, "Unauthorized");
  assert.equal(FORGE_ERRORS[2].name, "NotFound");
  assert.equal(FORGE_ERRORS[3].name, "InvalidInput");
  assert.equal(FORGE_ERRORS[4].name, "InsufficientFunds");
  assert.equal(FORGE_ERRORS[5].name, "AlreadyInitialized");
  assert.equal(FORGE_ERRORS[6].name, "NotInitialized");
  assert.equal(FORGE_ERRORS[7].name, "DeadlineReached");
  assert.equal(FORGE_ERRORS[8].name, "InsufficientAllowance");
  assert.equal(FORGE_ERRORS[9].name, "ArithmeticOverflow");
  assert.equal(FORGE_ERRORS[10].name, "Custom");
  assert.equal(FORGE_ERRORS[11].name, "TokenTransferFailed");
  assert.equal(FORGE_ERRORS[12].name, "ContractInvocationFailed");
  assert.equal(FORGE_ERRORS[13].name, "WithdrawalLimitExceeded");

  assert.equal(forgeErrorName(3), "InvalidInput");
  assert.equal(forgeErrorName(999), undefined);
});

test("parseForgeError extracts Soroban host error Error(Contract, #N)", () => {
  const err1 = new Error("HostError: Error(Contract, #1)");
  const parsed1 = parseForgeError(err1);
  assert.equal(parsed1.code, 1);
  assert.equal(parsed1.name, "Unauthorized");

  const err2 = "Transaction simulation failed with Error(Contract, #2)";
  const parsed2 = parseForgeError(err2);
  assert.equal(parsed2.code, 2);
  assert.equal(parsed2.name, "NotFound");
});

test("parseForgeError identifies named ForgeError in strings", () => {
  const err = new Error("Invocation reverted due to InsufficientFunds in contract custody");
  const parsed = parseForgeError(err);
  assert.equal(parsed.code, 4);
  assert.equal(parsed.name, "InsufficientFunds");
});

test("parseForgeError handles unmapped error gracefully", () => {
  const err = new Error("Network timeout after 30 seconds");
  const parsed = parseForgeError(err);
  assert.equal(parsed.code, undefined);
  assert.equal(parsed.name, undefined);
  assert.equal(parsed.rawMessage, "Network timeout after 30 seconds");
});

test("decodeContractEvents formats event topics and data cleanly", () => {
  const rawEvents = [
    {
      type: "contract",
      topic: ["escrow", "created"],
      value: { _value: "10000000" },
    },
    {
      topic: ["dispute"],
      value: "resolved",
    },
  ];

  const decoded = decodeContractEvents(rawEvents);
  assert.equal(decoded.length, 2);
  assert.equal(decoded[0].topic, "escrow / created");
  assert.equal(decoded[0].value, "10000000");
  assert.equal(decoded[1].topic, "dispute");
  assert.equal(decoded[1].value, "resolved");
});
