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
