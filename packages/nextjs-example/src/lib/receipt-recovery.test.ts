import assert from "node:assert/strict";
import test from "node:test";
import {
  checkReceiptContextMatch,
  createMemoryReceiptStorage,
  createRpcGetTransactionPoller,
  dismissPendingReceipt,
  loadPendingReceipt,
  PENDING_RECEIPT_STORAGE_KEY,
  pollReceiptOutcome,
  RECEIPT_SCHEMA_VERSION,
  ReceiptValidationError,
  savePendingReceipt,
  validatePersistedReceiptRecord,
} from "./receipt-recovery.ts";

const TESTNET_PASSPHRASE = "Test SDF Network ; September 2015";
const MAINNET_PASSPHRASE = "Public Global Stellar Network ; September 2015";
const CONTRACT_ID =
  "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB";
const ACCOUNT_A =
  "GAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN7";
const ACCOUNT_B =
  "GBDEVU63Y6NTHJQQZIKVTC23NWLQVP3WJ2RI2OTSJTNYOIGICST6DUXR";
const SAMPLE_TX_HASH =
  "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

test("persists and reloads a pending receipt with hash, network, action, and timestamp", () => {
  const storage = createMemoryReceiptStorage();
  const saved = savePendingReceipt(
    {
      txHash: SAMPLE_TX_HASH,
      networkPassphrase: TESTNET_PASSPHRASE,
      contractId: CONTRACT_ID,
      account: ACCOUNT_A,
      action: "deposit",
      escrowId: "12",
      submittedAtMs: 1_700_000_000_000,
    },
    storage
  );

  assert.equal(saved.schemaVersion, RECEIPT_SCHEMA_VERSION);
  assert.equal(saved.txHash, SAMPLE_TX_HASH);
  assert.equal(saved.lastOutcome, "pending");

  const reloaded = loadPendingReceipt(storage);
  assert.equal(reloaded.error, undefined);
  assert.deepEqual(reloaded.record, saved);
});

test("rejects secrets, signed transaction payloads, and malformed hashes", () => {
  const secretSeed = "SAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

  assert.throws(
    () =>
      savePendingReceipt({
        txHash: SAMPLE_TX_HASH,
        networkPassphrase: TESTNET_PASSPHRASE,
        contractId: CONTRACT_ID,
        account: secretSeed,
        action: "create_escrow",
      }),
    (err: unknown) =>
      err instanceof ReceiptValidationError &&
      err.code === "SENSITIVE_DATA_REJECTED"
  );

  assert.throws(
    () =>
      validatePersistedReceiptRecord({
        schemaVersion: RECEIPT_SCHEMA_VERSION,
        txHash: SAMPLE_TX_HASH,
        networkPassphrase: TESTNET_PASSPHRASE,
        contractId: CONTRACT_ID,
        action: "deposit",
        submittedAtMs: 1_700_000_000_000,
        signedTxXdr: "AAAAAgAAAAB...",
      }),
    (err: unknown) =>
      err instanceof ReceiptValidationError &&
      err.code === "SENSITIVE_DATA_REJECTED"
  );

  assert.throws(
    () =>
      savePendingReceipt({
        txHash: "not-a-valid-64-char-hex-hash",
        networkPassphrase: TESTNET_PASSPHRASE,
        contractId: CONTRACT_ID,
        action: "release",
      }),
    (err: unknown) =>
      err instanceof ReceiptValidationError &&
      err.code === "INVALID_RECEIPT_FIELD"
  );
});

test("reload recovery resumes bounded polling and resolves confirmed, failed, expired, and unknown outcomes", async () => {
  const storage = createMemoryReceiptStorage();
  const baseTime = 1_700_000_000_000;
  const initial = savePendingReceipt(
    {
      txHash: SAMPLE_TX_HASH,
      networkPassphrase: TESTNET_PASSPHRASE,
      contractId: CONTRACT_ID,
      account: ACCOUNT_A,
      action: "release",
      escrowId: "7",
      submittedAtMs: baseTime,
    },
    storage
  );

  // 1. Confirmed after 2 attempts (NOT_FOUND -> SUCCESS)
  let callCount = 0;
  const confirmedResult = await pollReceiptOutcome({
    record: initial,
    rpcUrl: "https://soroban-testnet.stellar.org",
    expectedNetworkPassphrase: TESTNET_PASSPHRASE,
    expectedContractId: CONTRACT_ID,
    connectedAccount: ACCOUNT_A,
    maxAttempts: 4,
    pollIntervalMs: 0,
    nowFn: () => baseTime + 5_000,
    storage,
    getTransaction: async () => {
      callCount += 1;
      if (callCount < 2) {
        return { status: "NOT_FOUND" };
      }
      return { status: "SUCCESS", ledger: 812_345 };
    },
  });

  assert.equal(confirmedResult.outcome, "confirmed");
  assert.equal(confirmedResult.attempts, 2);
  assert.equal(confirmedResult.record.ledger, 812_345);
  assert.equal(loadPendingReceipt(storage).record?.lastOutcome, "confirmed");

  // 2. Failed outcome
  const failedResult = await pollReceiptOutcome({
    record: initial,
    rpcUrl: "https://soroban-testnet.stellar.org",
    expectedNetworkPassphrase: TESTNET_PASSPHRASE,
    expectedContractId: CONTRACT_ID,
    connectedAccount: ACCOUNT_A,
    maxAttempts: 3,
    pollIntervalMs: 0,
    nowFn: () => baseTime + 5_000,
    storage,
    getTransaction: async () => ({
      status: "FAILED",
      ledger: 812_346,
      errorDetail: "HostError: Error(Contract, #2)",
    }),
  });

  assert.equal(failedResult.outcome, "failed");
  assert.equal(failedResult.attempts, 1);
  assert.match(failedResult.explanation, /failed on-chain/);

  // 3. Unknown outcome when NOT_FOUND persists through bounded maxAttempts
  const unknownResult = await pollReceiptOutcome({
    record: initial,
    rpcUrl: "https://soroban-testnet.stellar.org",
    expectedNetworkPassphrase: TESTNET_PASSPHRASE,
    expectedContractId: CONTRACT_ID,
    connectedAccount: ACCOUNT_A,
    maxAttempts: 3,
    pollIntervalMs: 0,
    ttlMs: 60_000,
    nowFn: () => baseTime + 10_000,
    storage,
    getTransaction: async () => ({ status: "NOT_FOUND" }),
  });

  assert.equal(unknownResult.outcome, "unknown");
  assert.equal(unknownResult.attempts, 3);
  assert.match(unknownResult.explanation, /unknown/);

  // 4. Expired outcome when elapsed time exceeds ttlMs
  const expiredResult = await pollReceiptOutcome({
    record: initial,
    rpcUrl: "https://soroban-testnet.stellar.org",
    expectedNetworkPassphrase: TESTNET_PASSPHRASE,
    expectedContractId: CONTRACT_ID,
    connectedAccount: ACCOUNT_A,
    maxAttempts: 3,
    pollIntervalMs: 0,
    ttlMs: 30_000,
    nowFn: () => baseTime + 45_000,
    storage,
    getTransaction: async () => ({ status: "NOT_FOUND" }),
  });

  assert.equal(expiredResult.outcome, "expired");
  assert.equal(expiredResult.attempts, 0);
  assert.match(expiredResult.explanation, /expired/);
});

test("network, contract, or account mismatch prevents unsafe continuation and explains the issue", async () => {
  const storage = createMemoryReceiptStorage();
  const record = savePendingReceipt(
    {
      txHash: SAMPLE_TX_HASH,
      networkPassphrase: TESTNET_PASSPHRASE,
      contractId: CONTRACT_ID,
      account: ACCOUNT_A,
      action: "deposit",
      submittedAtMs: 1_700_000_000_000,
    },
    storage
  );

  // Network mismatch
  const netMismatch = await pollReceiptOutcome({
    record,
    rpcUrl: "https://soroban-testnet.stellar.org",
    expectedNetworkPassphrase: MAINNET_PASSPHRASE,
    expectedContractId: CONTRACT_ID,
    connectedAccount: ACCOUNT_A,
    storage,
    getTransaction: async () => {
      throw new Error("Should not poll RPC when network mismatches");
    },
  });

  assert.equal(netMismatch.outcome, "mismatch");
  assert.equal(netMismatch.attempts, 0);
  assert.match(netMismatch.explanation, /Network mismatch/);

  // Account mismatch
  const acctCheck = checkReceiptContextMatch({
    record,
    expectedNetworkPassphrase: TESTNET_PASSPHRASE,
    expectedContractId: CONTRACT_ID,
    connectedAccount: ACCOUNT_B,
  });
  assert.equal(acctCheck.matches, false);
  assert.equal(acctCheck.mismatchKind, "account");
  assert.match(acctCheck.explanation ?? "", /Account mismatch/);

  const acctMismatchPoll = await pollReceiptOutcome({
    record,
    rpcUrl: "https://soroban-testnet.stellar.org",
    expectedNetworkPassphrase: TESTNET_PASSPHRASE,
    expectedContractId: CONTRACT_ID,
    connectedAccount: ACCOUNT_B,
    storage,
    getTransaction: async () => {
      throw new Error("Should not poll RPC when account mismatches");
    },
  });
  assert.equal(acctMismatchPoll.outcome, "mismatch");
  assert.equal(acctMismatchPoll.attempts, 0);
});

test("dismissPendingReceipt cleans up persisted receipt state while preserving other storage keys", () => {
  const storage = createMemoryReceiptStorage({
    "other-app-key": "keep-me",
  });
  savePendingReceipt(
    {
      txHash: SAMPLE_TX_HASH,
      networkPassphrase: TESTNET_PASSPHRASE,
      contractId: CONTRACT_ID,
      account: ACCOUNT_A,
      action: "cancel",
    },
    storage
  );

  assert.notEqual(storage.getItem(PENDING_RECEIPT_STORAGE_KEY), null);
  dismissPendingReceipt(storage);
  assert.equal(storage.getItem(PENDING_RECEIPT_STORAGE_KEY), null);
  assert.equal(storage.getItem("other-app-key"), "keep-me");
});

test("createRpcGetTransactionPoller parses JSON-RPC getTransaction response", async () => {
  const fakeFetch: typeof fetch = async () =>
    new Response(
      JSON.stringify({
        jsonrpc: "2.0",
        id: "1",
        result: { status: "SUCCESS", ledger: 901_234 },
      }),
      { status: 200 }
    );

  const poller = createRpcGetTransactionPoller(fakeFetch);
  const res = await poller(SAMPLE_TX_HASH, "https://soroban-testnet.stellar.org");
  assert.equal(res.status, "SUCCESS");
  assert.equal(res.ledger, 901_234);
});
