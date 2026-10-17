import assert from "node:assert/strict";
import test from "node:test";
import {
  createEmptyDemoState,
  createMemoryBrowserStorage,
  DEFAULT_DEMO_TOKEN,
  DEMO_STATE_LABEL,
  DEMO_STATE_SCHEMA_VERSION,
  DEMO_STATE_STORAGE_KEY,
  DemoStateValidationError,
  exportDemoStateJson,
  isDemoStateEmpty,
  loadDemoStateFromStorage,
  resetDemoStateInStorage,
  saveDemoStateToStorage,
  validateDemoStateDocument,
  type DemoStatePayload,
} from "./demo-state.ts";

function samplePopulatedState(): DemoStatePayload {
  return {
    draft: {
      buyer: "GAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN7",
      seller: "GBDEVU63Y6NTHJQQZIKVTC23NWLQVP3WJ2RI2OTSJTNYOIGICST6DUXR",
      arbiter: "GCFXHS4GXL6BVUCXBWXGTITROWLVYXQKQLF4YH5O5JT3YZXCYPAFBJZB",
      token: DEFAULT_DEMO_TOKEN,
      amount: "25000000",
      timeoutSecs: "7200",
    },
    createdId: "42",
    actionEscrowId: "42",
    readEscrowId: "42",
    resolveFavor: "buyer",
    log: [
      {
        id: 1,
        time: "12:00:00",
        text: "Escrow created — id 42 (Pending)",
        kind: "ok",
      },
    ],
  };
}

test("exports and validates a versioned JSON demo-state document free of secrets", () => {
  const state = samplePopulatedState();
  const exportedAt = "2026-03-30T12:00:00.000Z";
  const json = exportDemoStateJson(state, exportedAt);

  const parsed = JSON.parse(json) as Record<string, unknown>;
  assert.equal(parsed.schemaVersion, DEMO_STATE_SCHEMA_VERSION);
  assert.equal(parsed.scope, DEMO_STATE_LABEL);
  assert.equal(parsed.exportedAt, exportedAt);

  const validated = validateDemoStateDocument(json);
  assert.equal(validated.schemaVersion, DEMO_STATE_SCHEMA_VERSION);
  assert.equal(validated.scope, DEMO_STATE_LABEL);
  assert.equal(validated.exportedAt, exportedAt);
  assert.deepEqual(validated.state, state);
});

test("detects empty baseline state vs populated demo state", () => {
  const empty = createEmptyDemoState();
  assert.equal(isDemoStateEmpty(empty), true);

  const populated = samplePopulatedState();
  assert.equal(isDemoStateEmpty(populated), false);
});

test("rejects malformed JSON and missing schemaVersion with MALFORMED_DATA", () => {
  assert.throws(
    () => validateDemoStateDocument(""),
    (err: unknown) =>
      err instanceof DemoStateValidationError && err.code === "MALFORMED_DATA",
  );

  assert.throws(
    () => validateDemoStateDocument("{not-valid-json"),
    (err: unknown) =>
      err instanceof DemoStateValidationError && err.code === "MALFORMED_DATA",
  );

  assert.throws(
    () => validateDemoStateDocument(JSON.stringify({ state: samplePopulatedState() })),
    (err: unknown) =>
      err instanceof DemoStateValidationError && err.code === "MALFORMED_DATA",
  );
});

test("rejects unsupported schemaVersion with VERSION_MISMATCH", () => {
  const badVersionDoc = {
    schemaVersion: "soroban-forge/demo-state@999",
    scope: DEMO_STATE_LABEL,
    exportedAt: "2026-03-30T12:00:00.000Z",
    state: samplePopulatedState(),
  };

  assert.throws(
    () => validateDemoStateDocument(JSON.stringify(badVersionDoc)),
    (err: unknown) =>
      err instanceof DemoStateValidationError && err.code === "VERSION_MISMATCH",
  );
});

test("rejects Stellar secret seeds and forbidden secret/signed payload fields with SENSITIVE_DATA_REJECTED", () => {
  const stateWithSecretSeed = samplePopulatedState();
  // 56-character Stellar secret seed starting with S and base32 chars
  stateWithSecretSeed.draft.buyer =
    "SAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

  assert.throws(
    () => exportDemoStateJson(stateWithSecretSeed),
    (err: unknown) =>
      err instanceof DemoStateValidationError &&
      err.code === "SENSITIVE_DATA_REJECTED",
  );

  const docWithSignedPayload = {
    schemaVersion: DEMO_STATE_SCHEMA_VERSION,
    scope: DEMO_STATE_LABEL,
    exportedAt: "2026-03-30T12:00:00.000Z",
    state: samplePopulatedState(),
    signedTxXdr: "AAAAAgAAAAB...",
  };

  assert.throws(
    () => validateDemoStateDocument(docWithSignedPayload),
    (err: unknown) =>
      err instanceof DemoStateValidationError &&
      err.code === "SENSITIVE_DATA_REJECTED",
  );
});

test("supports backward-compatible legacy unversioned DemoStatePayload objects", () => {
  const legacyPayload = samplePopulatedState();
  const migrated = validateDemoStateDocument(
    legacyPayload,
    "2026-03-30T00:00:00.000Z",
  );
  assert.equal(migrated.schemaVersion, DEMO_STATE_SCHEMA_VERSION);
  assert.equal(migrated.scope, DEMO_STATE_LABEL);
  assert.deepEqual(migrated.state, legacyPayload);
});

test("save, load, and reset demo state in storage while preserving live network and wallet keys", () => {
  const storage = createMemoryBrowserStorage({
    "stellar-wallets-kit:active-wallet": "freighter",
    "soroban-forge:live-network-config": "testnet",
  });

  // Initial load from empty demo key returns empty baseline without error
  const initialLoad = loadDemoStateFromStorage(storage);
  assert.equal(initialLoad.isEmpty, true);
  assert.equal(initialLoad.error, undefined);

  // Save populated demo state
  const populated = samplePopulatedState();
  saveDemoStateToStorage(populated, storage, "2026-03-30T12:00:00.000Z");

  const loaded = loadDemoStateFromStorage(storage);
  assert.equal(loaded.isEmpty, false);
  assert.equal(loaded.error, undefined);
  assert.deepEqual(loaded.state, populated);

  // Reset without explicit confirmation must fail
  assert.throws(
    () => resetDemoStateInStorage(storage, DEFAULT_DEMO_TOKEN, { confirmed: false }),
    (err: unknown) =>
      err instanceof DemoStateValidationError &&
      err.code === "CONFIRMATION_REQUIRED",
  );
  // Storage still has the demo state when confirmation was not granted
  assert.notEqual(storage.getItem(DEMO_STATE_STORAGE_KEY), null);

  // Reset with explicit confirmation clears demo state and preserves unrelated keys
  const afterReset = resetDemoStateInStorage(storage, DEFAULT_DEMO_TOKEN, {
    confirmed: true,
  });
  assert.equal(isDemoStateEmpty(afterReset), true);
  assert.equal(storage.getItem(DEMO_STATE_STORAGE_KEY), null);
  assert.equal(
    storage.getItem("stellar-wallets-kit:active-wallet"),
    "freighter",
  );
  assert.equal(
    storage.getItem("soroban-forge:live-network-config"),
    "testnet",
  );
});

test("loadDemoStateFromStorage returns empty state and structured error when stored value is corrupted", () => {
  const storage = createMemoryBrowserStorage({
    [DEMO_STATE_STORAGE_KEY]: "{corrupted-json",
  });

  const result = loadDemoStateFromStorage(storage);
  assert.equal(result.isEmpty, true);
  assert.ok(result.error instanceof DemoStateValidationError);
  assert.equal(result.error?.code, "MALFORMED_DATA");
});
