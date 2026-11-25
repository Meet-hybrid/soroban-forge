import test from "node:test";
import assert from "node:assert/strict";

import {
  decodeContractResult,
  decodeForgeReceipt,
  pollTransactionStatus,
  type TransactionPoller,
} from "../../dist/index.js";

test("decodeContractResult normalizes success and registry contract errors", () => {
  assert.deepEqual(decodeContractResult({ ok: true, value: 42n }), { ok: true, value: 42n });
  const decoded = decodeContractResult({ ok: false, error: { code: 3 } });
  assert.equal(decoded.ok, false);
  if (!decoded.ok) {
    assert.equal(decoded.error.code, 3);
    assert.equal(decoded.error.name, "InvalidInput");
  }
});

test("decodeForgeReceipt distinguishes success, contract failure, timeout, and malformed data", () => {
  assert.equal(decodeForgeReceipt({ status: "SUCCESS", result: { ok: true, value: "paid" }, events: ["x"] }).stage, "success");
  assert.equal(decodeForgeReceipt({ status: "FAILED", result: { ok: false, error: { code: 13 } } }).stage, "contract_error");
  assert.equal(decodeForgeReceipt({ status: "NOT_FOUND", txHash: "abc" }).stage, "timeout");
  assert.equal(decodeForgeReceipt("bad receipt").stage, "malformed_receipt");
});

test("pollTransactionStatus waits for NOT_FOUND then succeeds", async () => {
  let attempts = 0;
  const poller: TransactionPoller = {
    async getTransaction() {
      attempts += 1;
      return attempts === 1
        ? ({ status: "NOT_FOUND", txHash: "fixture", latestLedger: 1, latestLedgerCloseTime: 1, oldestLedger: 1 } as never)
        : ({ status: "SUCCESS", txHash: "fixture", latestLedger: 2, latestLedgerCloseTime: 2, oldestLedger: 1, events: [], ledger: 2, createdAt: 1, applicationOrder: 1, feeBump: false, envelopeXdr: "", resultXdr: "", resultMetaXdr: "", returnValue: "ok" } as never);
    },
  };
  const receipt = await pollTransactionStatus(poller, "fixture", { intervalMs: 0, sleep: async () => {} });
  assert.equal(receipt.status, "SUCCESS");
  assert.equal(attempts, 2);
});

test("pollTransactionStatus times out after bounded attempts", async () => {
  const poller: TransactionPoller = {
    async getTransaction() {
      return { status: "NOT_FOUND", txHash: "fixture", latestLedger: 1, latestLedgerCloseTime: 1, oldestLedger: 1 } as never;
    },
  };
  await assert.rejects(
    pollTransactionStatus(poller, "fixture", { maxAttempts: 2, intervalMs: 0, sleep: async () => {} }),
    (error: unknown) => error instanceof Error && error.name === "ForgeSdkError" && /timed out/.test(error.message),
  );
});
