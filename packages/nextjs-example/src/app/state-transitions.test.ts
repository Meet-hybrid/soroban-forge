import test from "node:test";
import assert from "node:assert/strict";

import {
  initialTransactionState,
  transactionReducer,
  evaluatePrerequisites,
  transitionDemoEscrow,
  DEMO_ACCOUNTS,
  type DemoEscrowRecord,
  type TransactionState,
} from "./state.ts";
import {
  classifyTransactionStatus,
  createPendingReceipt,
  isSafeToResume,
  loadReceipts,
  MAX_RECEIPT_ATTEMPTS,
  RECEIPT_TTL_MS,
  receiptStatusLabel,
  recordPollAttempt,
  removeReceipt,
  restoreReceipts,
  saveReceipts,
  upsertReceipt,
  validateReceipt,
  type PendingReceipt,
  type ReceiptStorage,
} from "./receipt-recovery.ts";

function memoryStorage(initial: string | null = null): ReceiptStorage {
  let value = initial;
  return {
    getItem: () => value,
    setItem: (_key, next) => { value = next; },
    removeItem: () => { value = null; },
  };
}

const receipt: PendingReceipt = {
  hash: "a".repeat(64),
  network: "testnet",
  account: "GACCOUNT",
  action: "create_escrow",
  timestamp: 1_700_000_000_000,
  status: "pending",
  attempts: 0,
};

test("initialTransactionState provides clean default values", () => {
  const demoInit = initialTransactionState("demo");
  assert.equal(demoInit.stage, "idle");
  assert.equal(demoInit.mode, "demo");
  assert.equal(demoInit.action, null);
  assert.equal(demoInit.hash, null);
  assert.equal(demoInit.error, null);

  const testnetInit = initialTransactionState("testnet");
  assert.equal(testnetInit.stage, "idle");
  assert.equal(testnetInit.mode, "testnet");
});

test("transactionReducer handles full happy path: idle -> simulating -> signing -> submitting -> confirmed", () => {
  let state: TransactionState = initialTransactionState("testnet");

  // Step 1: Start simulation
  state = transactionReducer(state, {
    type: "START_SIMULATION",
    action: "create_escrow",
    mode: "testnet",
  });
  assert.equal(state.stage, "simulating");
  assert.equal(state.action, "create_escrow");
  assert.equal(state.mode, "testnet");
  assert.ok(state.timestamp !== null);

  // Step 2: Start signing
  state = transactionReducer(state, { type: "START_SIGNING" });
  assert.equal(state.stage, "signing");
  assert.equal(state.action, "create_escrow");

  // Step 3: Start submission
  state = transactionReducer(state, {
    type: "START_SUBMISSION",
    hash: "a1b2c3d4e5f67890",
  });
  assert.equal(state.stage, "submitting");
  assert.equal(state.hash, "a1b2c3d4e5f67890");

  // Step 4: Confirm transaction
  state = transactionReducer(state, {
    type: "CONFIRM",
    hash: "a1b2c3d4e5f67890",
    details: "Escrow #42 created successfully on testnet",
  });
  assert.equal(state.stage, "confirmed");
  assert.equal(state.hash, "a1b2c3d4e5f67890");
  assert.equal(state.details, "Escrow #42 created successfully on testnet");
  assert.equal(state.error, null);

  // Step 5: Reset
  state = transactionReducer(state, { type: "RESET" });
  assert.equal(state.stage, "idle");
  assert.equal(state.action, null);
  assert.equal(state.mode, "testnet");
});

test("transactionReducer handles failure transitions and error retention", () => {
  let state: TransactionState = initialTransactionState("demo");

  // Fail during simulation
  state = transactionReducer(state, {
    type: "START_SIMULATION",
    action: "deposit",
    mode: "demo",
  });
  assert.equal(state.stage, "simulating");

  state = transactionReducer(state, {
    type: "FAIL",
    error: "Account has insufficient balance for deposit.",
  });
  assert.equal(state.stage, "failed");
  assert.equal(state.action, "deposit");
  assert.equal(state.error, "Account has insufficient balance for deposit.");

  // Fail during signing
  state = transactionReducer(state, {
    type: "START_SIMULATION",
    action: "release",
    mode: "testnet",
  });
  state = transactionReducer(state, { type: "START_SIGNING" });
  assert.equal(state.stage, "signing");

  state = transactionReducer(state, {
    type: "FAIL",
    error: "User rejected wallet signing request.",
  });
  assert.equal(state.stage, "failed");
  assert.equal(state.error, "User rejected wallet signing request.");
});

test("transactionReducer switches execution mode cleanly", () => {
  let state: TransactionState = initialTransactionState("demo");
  state = transactionReducer(state, {
    type: "START_SIMULATION",
    action: "dispute",
    mode: "demo",
  });
  assert.equal(state.stage, "simulating");

  // Switching mode resets any in-flight transaction
  state = transactionReducer(state, { type: "SET_MODE", mode: "testnet" });
  assert.equal(state.stage, "idle");
  assert.equal(state.mode, "testnet");
  assert.equal(state.action, null);
});

test("evaluatePrerequisites correctly validates wallet and network states", () => {
  const expectedPassphrase = "Test SDF Network ; September 2015";

  // Scenario 1: Unmounted, disconnected
  const unmountedPrereqs = evaluatePrerequisites({
    walletMounted: false,
    connectedAccount: null,
    networkPassphrase: expectedPassphrase,
    expectedPassphrase,
  });
  assert.equal(unmountedPrereqs[0].satisfied, false);
  assert.equal(unmountedPrereqs[1].satisfied, false);
  assert.equal(unmountedPrereqs[2].satisfied, true);

  // Scenario 2: Mounted, connected, correct network
  const readyPrereqs = evaluatePrerequisites({
    walletMounted: true,
    connectedAccount: "GBUYER1234567890",
    networkPassphrase: expectedPassphrase,
    expectedPassphrase,
  });
  assert.ok(readyPrereqs.every((p) => p.satisfied));

  // Scenario 3: Mainnet passphrase warning
  const wrongNetwork = evaluatePrerequisites({
    walletMounted: true,
    connectedAccount: "GBUYER1234567890",
    networkPassphrase: "Public Global Stellar Network ; September 2015",
    expectedPassphrase,
  });
  const networkPrereq = wrongNetwork.find((p) => p.id === "network-target");
  assert.ok(networkPrereq);
  assert.equal(networkPrereq.satisfied, false);
});

test("transitionDemoEscrow enforces valid escrow lifecycle transitions", () => {
  const initialEscrow: DemoEscrowRecord = {
    escrow_id: BigInt(1),
    buyer: DEMO_ACCOUNTS.buyer.address,
    seller: DEMO_ACCOUNTS.seller.address,
    arbiter: DEMO_ACCOUNTS.arbiter.address,
    token: "CBJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O",
    amount: BigInt(10000000),
    timeout: BigInt(3600),
    created_at: BigInt(1700000000),
    status: { tag: "Pending", values: undefined },
  };

  // Deposit: Pending -> Funded
  const depositRes = transitionDemoEscrow(initialEscrow, "deposit");
  assert.ok(depositRes.success);
  assert.equal(depositRes.updated.status.tag, "Funded");

  // Invalid: double deposit
  const doubleDeposit = transitionDemoEscrow(depositRes.updated, "deposit");
  assert.equal(doubleDeposit.success, false);

  // Release: Funded -> Completed
  const releaseRes = transitionDemoEscrow(depositRes.updated, "release");
  assert.ok(releaseRes.success);
  assert.equal(releaseRes.updated.status.tag, "Completed");

  // Cancel: Pending -> Cancelled
  const cancelRes = transitionDemoEscrow(initialEscrow, "cancel");
  assert.ok(cancelRes.success);
  assert.equal(cancelRes.updated.status.tag, "Cancelled");

  // Cannot cancel funded escrow
  const cancelFunded = transitionDemoEscrow(depositRes.updated, "cancel");
  assert.equal(cancelFunded.success, false);

  // Dispute: Funded -> Disputed (by valid party)
  const disputeRes = transitionDemoEscrow(depositRes.updated, "dispute", {
    claimant: DEMO_ACCOUNTS.buyer.address,
  });
  assert.ok(disputeRes.success);
  assert.equal(disputeRes.updated.status.tag, "Disputed");

  // Dispute: fails with unauthorized claimant
  const invalidDispute = transitionDemoEscrow(depositRes.updated, "dispute", {
    claimant: "GUNAUTHORIZED_OUTSIDER_ADDRESS",
  });
  assert.equal(invalidDispute.success, false);

  // Resolve in favor of seller: Disputed -> Completed
  const resolveSeller = transitionDemoEscrow(disputeRes.updated, "resolve", {
    in_favor_of_seller: true,
  });
  assert.ok(resolveSeller.success);
  assert.equal(resolveSeller.updated.status.tag, "Completed");

  // Resolve in favor of buyer: Disputed -> Refunded
  const resolveBuyer = transitionDemoEscrow(disputeRes.updated, "resolve", {
    in_favor_of_seller: false,
  });
  assert.ok(resolveBuyer.success);
  assert.equal(resolveBuyer.updated.status.tag, "Refunded");
});

test("receipt persistence reload and cleanup validate minimal stored data", () => {
  const storage = memoryStorage();
  saveReceipts(storage, [receipt]);
  assert.deepEqual(loadReceipts(storage), [receipt]);
  assert.equal(validateReceipt({ ...receipt, network: "mainnet" }), null);
  assert.deepEqual(removeReceipt(storage, receipt.hash), []);
});

test("network and account mismatch prevent unsafe receipt continuation", () => {
  assert.match(isSafeToResume(receipt, "mainnet", "GACCOUNT") ?? "", /Network mismatch/);
  assert.match(isSafeToResume(receipt, "testnet", "GOTHER") ?? "", /Account mismatch/);
  assert.equal(isSafeToResume(receipt, "testnet", "GACCOUNT"), null);
});

test("receipt statuses support confirmed failed expired and unknown outcomes", () => {
  for (const status of ["confirmed", "failed", "expired", "unknown"] as const) {
    assert.ok(validateReceipt({ ...receipt, status }));
    assert.notEqual(receiptStatusLabel(status), "");
  }
});

test("reload recovery expires stale pending receipts and keeps fresh ones", () => {
  const storage = memoryStorage();
  const fresh = createPendingReceipt(
    { hash: "b".repeat(64), account: "GACCOUNT", action: "release" },
    RECEIPT_TTL_MS,
  );
  const stale = { ...receipt, timestamp: 0 };
  saveReceipts(storage, [fresh, stale]);

  const restored = restoreReceipts(storage, RECEIPT_TTL_MS + 1);
  assert.equal(restored.find((item) => item.hash === fresh.hash)?.status, "pending");
  assert.equal(restored.find((item) => item.hash === stale.hash)?.status, "expired");
  assert.equal(loadReceipts(storage).length, 2);
});

test("upsert keeps receipts bounded and dismisses only the requested hash", () => {
  const storage = memoryStorage();
  for (let i = 0; i < 15; i += 1) {
    upsertReceipt(storage, {
      ...receipt,
      hash: `${i}`.padStart(64, "0"),
    });
  }
  const stored = loadReceipts(storage);
  assert.equal(stored.length, 10);
  assert.equal(removeReceipt(storage, stored[0].hash).length, 9);
});

test("poll outcomes classify and bound retries without exceeding the attempt limit", () => {
  assert.equal(classifyTransactionStatus("SUCCESS"), "confirmed");
  assert.equal(classifyTransactionStatus("FAILED"), "failed");
  assert.equal(classifyTransactionStatus("NOT_FOUND"), "pending");
  assert.equal(classifyTransactionStatus("SOMETHING_NEW"), "unknown");

  let pending = createPendingReceipt(
    { hash: "c".repeat(64), account: "GACCOUNT", action: "deposit" },
    0,
  );
  for (let i = 0; i < MAX_RECEIPT_ATTEMPTS - 1; i += 1) {
    pending = recordPollAttempt(pending, "NOT_FOUND", 1_000);
    assert.equal(pending.status, "pending");
  }
  pending = recordPollAttempt(pending, "NOT_FOUND", 1_000);
  assert.equal(pending.status, "expired");
  assert.equal(pending.attempts, MAX_RECEIPT_ATTEMPTS);
});

test("poll stops early on confirmed and failed outcomes", () => {
  const pending = createPendingReceipt(
    { hash: "d".repeat(64), account: "GACCOUNT", action: "refund" },
    0,
  );
  const confirmed = recordPollAttempt(pending, "SUCCESS", 1_000);
  assert.equal(confirmed.status, "confirmed");
  assert.equal(confirmed.attempts, 1);
  const failed = recordPollAttempt(pending, "FAILED", 1_000);
  assert.equal(failed.status, "failed");
});
