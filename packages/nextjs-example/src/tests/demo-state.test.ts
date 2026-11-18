import assert from "node:assert/strict";
import test from "node:test";

import {
  DEMO_STATE_SCHEMA,
  DEMO_STATE_STORAGE_KEY,
  DEMO_STATE_VERSION,
  DemoStateValidationError,
  emptyDemoState,
  loadDemoState,
  parseDemoStateExport,
  parseStoredDemoState,
  resetDemoState,
  saveDemoState,
  serializeDemoStateExport,
  type DemoState,
  type DemoStorage,
} from "../lib/demo-state";

class MemoryStorage implements DemoStorage {
  private readonly values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }
}

function sampleState(): DemoState {
  return {
    buyer: "GBUYEREXAMPLE0000000000000000000000000000000000000000000000000",
    seller: "GSELLEREXAMPLE00000000000000000000000000000000000000000000000",
    arbiter: "GARBiterEXAMPLE0000000000000000000000000000000000000000000000",
    token:
      "CBJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O",
    amount: "25000000",
    timeoutSecs: "7200",
    createdId: "42",
    actionEscrowId: "42",
    readEscrowId: "42",
    resolveFavor: "buyer",
  };
}

function expectsCode(code: DemoStateValidationError["code"]) {
  return (error: unknown): boolean =>
    error instanceof DemoStateValidationError && error.code === code;
}

test("creates and parses a valid versioned local demo state export", () => {
  const state = sampleState();
  const json = serializeDemoStateExport(
    state,
    () => new Date("2026-11-19T12:34:56.000Z")
  );
  const document = JSON.parse(json) as Record<string, unknown>;

  assert.equal(document.schema, DEMO_STATE_SCHEMA);
  assert.equal(document.version, DEMO_STATE_VERSION);
  assert.equal(document.kind, "local-demo-state");
  assert.equal(document.exportedAt, "2026-11-19T12:34:56.000Z");
  assert.match(String(document.label), /local browser demo state/i);
  assert.deepEqual(parseDemoStateExport(json), state);
  assert.doesNotMatch(json, /signedTxXdr|privateKey|secretKey|mnemonic/i);
});

test("validates stored legacy state before use", () => {
  const storage = new MemoryStorage();
  storage.setItem(DEMO_STATE_STORAGE_KEY, JSON.stringify(sampleState()));

  assert.deepEqual(loadDemoState(storage), sampleState());
});

test("rejects malformed stored and exported data", () => {
  assert.throws(
    () => parseStoredDemoState("{not json"),
    expectsCode("invalid_json")
  );
  assert.throws(
    () => parseStoredDemoState('{"buyer":42}'),
    expectsCode("invalid_state")
  );
  assert.throws(
    () => parseStoredDemoState('{"buyer":"x","privateKey":"y"}'),
    expectsCode("invalid_state")
  );
  assert.throws(
    () => parseDemoStateExport('{"schema":"other","version":1,"state":null}'),
    expectsCode("schema_mismatch")
  );
});

test("rejects an unsupported export version", () => {
  const document = {
    schema: DEMO_STATE_SCHEMA,
    kind: "local-demo-state",
    version: DEMO_STATE_VERSION + 1,
    state: null,
  };

  assert.throws(
    () => parseDemoStateExport(JSON.stringify(document)),
    expectsCode("version_mismatch")
  );
});

test("reset clears stored demo state", () => {
  const storage = new MemoryStorage();
  saveDemoState(storage, sampleState());
  assert.deepEqual(loadDemoState(storage), sampleState());

  resetDemoState(storage);

  assert.equal(storage.getItem(DEMO_STATE_STORAGE_KEY), null);
  assert.equal(loadDemoState(storage), null);
});

test("handles empty state", () => {
  const storage = new MemoryStorage();
  const json = serializeDemoStateExport(null);
  const document = JSON.parse(json) as Record<string, unknown>;

  assert.equal(document.state, null);
  assert.equal(parseDemoStateExport(json), null);
  assert.equal(loadDemoState(storage), null);
  assert.deepEqual(emptyDemoState().createdId, null);
});
