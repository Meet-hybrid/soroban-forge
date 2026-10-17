import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  CHECKPOINT_SCHEMA_VERSION,
  CheckpointValidationError,
  checkCheckpointCompatibility,
  createCheckpoint,
  createIndexer,
  createMemoryCheckpointStore,
  deriveEventType,
  discardCheckpoint,
  filterEvents,
  loadAndValidateCheckpoint,
  loadFixtureEvents,
  normalizeEvents,
  rebuildCheckpoint,
  resumeIndexerFromCheckpoint,
  saveCheckpoint,
  validateCheckpoint,
  type ContractEventRecord,
  type IndexerRuntimeConfig,
} from "../indexer.js";

const ESCROW_CONTRACT_ID =
  "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHK3M";
const SECOND_CONTRACT_ID =
  "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC";

test("fixture events can be loaded and normalized deterministically", () => {
  const events = loadFixtureEvents("escrow");
  assert.ok(events.length > 0, "fixture must contain at least one event");

  const normalized = normalizeEvents(events);
  assert.deepEqual(
    normalized.map((event) => event.id),
    events.map((event) => event.id),
  );
  assert.ok(
    normalized.some((event) => event.contractId === ESCROW_CONTRACT_ID),
    "fixture should include the escrow contract events",
  );
});

test("indexer filters by ledger and contract metadata", () => {
  const events = loadFixtureEvents("escrow");
  const indexer = createIndexer({ events });

  const filtered = indexer.filter({
    contractId: ESCROW_CONTRACT_ID,
    fromLedger: 1001,
    toLedger: 1004,
  });

  assert.ok(
    filtered.length > 0,
    "should retain escrow events in the requested ledger range",
  );
  assert.ok(
    filtered.every((event) => event.ledger >= 1001 && event.ledger <= 1004),
  );
});

test("event type derivation recognizes lifecycle event names", () => {
  const event = loadFixtureEvents("escrow")[1] as ContractEventRecord;
  assert.equal(deriveEventType(event), "escrow_created");
  assert.ok(
    filterEvents(loadFixtureEvents("escrow"), { eventType: "escrow_created" })
      .length >= 1,
  );
});

test("valid checkpoint resumes from stored cursor without duplicating already indexed events", () => {
  const allEvents = filterEvents(loadFixtureEvents("escrow"), {
    contractId: ESCROW_CONTRACT_ID,
  });
  assert.ok(allEvents.length >= 3, "expected at least 3 escrow fixture events");

  const runtime: IndexerRuntimeConfig = {
    network: "testnet",
    contracts: [ESCROW_CONTRACT_ID],
  };
  const store = createMemoryCheckpointStore();

  // First batch: index the first 2 escrow events and persist a checkpoint.
  const firstBatch = allEvents.slice(0, 2);
  const initialRun = resumeIndexerFromCheckpoint({
    store,
    runtime,
    incomingEvents: firstBatch,
    updatedAt: "2026-01-01T00:00:00.000Z",
  });
  assert.equal(initialRun.events.length, 2);
  assert.deepEqual(initialRun.checkpoint.cursor, {
    ledger: firstBatch[1]!.ledger,
    eventIndex: firstBatch[1]!.eventIndex,
    pagingToken: firstBatch[1]!.pagingToken,
  });

  // Restart and replay the full stream (including already-indexed events).
  const resumedRun = resumeIndexerFromCheckpoint({
    store,
    runtime,
    existingEvents: initialRun.events,
    incomingEvents: allEvents,
    updatedAt: "2026-01-01T00:05:00.000Z",
  });

  // Only events strictly after the checkpoint cursor should be appended.
  assert.equal(
    resumedRun.appendedEvents.length,
    allEvents.length - firstBatch.length,
  );
  assert.deepEqual(
    resumedRun.events.map((e) => e.id),
    allEvents.map((e) => e.id),
  );
  assert.deepEqual(resumedRun.checkpoint.cursor, {
    ledger: allEvents[allEvents.length - 1]!.ledger,
    eventIndex: allEvents[allEvents.length - 1]!.eventIndex,
    pagingToken: allEvents[allEvents.length - 1]!.pagingToken,
  });
});

test("checkpoint validation fails closed on wrong network with actionable error", () => {
  const store = createMemoryCheckpointStore();
  const saved = createCheckpoint(
    { network: "testnet", contracts: [ESCROW_CONTRACT_ID] },
    { ledger: 1002, eventIndex: 1, pagingToken: "1002-1" },
  );
  saveCheckpoint(store, saved);

  assert.throws(
    () =>
      loadAndValidateCheckpoint(store, {
        network: "futurenet",
        contracts: [ESCROW_CONTRACT_ID],
      }),
    (err: unknown) => {
      assert.ok(err instanceof CheckpointValidationError);
      assert.equal(err.code, "NETWORK_MISMATCH");
      assert.match(err.message, /testnet/);
      assert.match(err.message, /futurenet/);
      assert.match(err.remediation, /discardCheckpoint|rebuildCheckpoint/);
      return true;
    },
  );
});

test("checkpoint validation fails closed on changed contract set", () => {
  const checkpoint = createCheckpoint(
    { network: "testnet", contracts: [ESCROW_CONTRACT_ID] },
    { ledger: 1002, eventIndex: 0 },
  );

  // Order-insensitive deduplication should pass when the set is identical.
  const reordered = validateCheckpoint(
    {
      ...checkpoint,
      contracts: [SECOND_CONTRACT_ID, ESCROW_CONTRACT_ID, ESCROW_CONTRACT_ID],
    },
    {
      network: "testnet",
      contracts: [ESCROW_CONTRACT_ID, SECOND_CONTRACT_ID],
    },
  );
  assert.equal(reordered.contracts.length, 2);

  // Adding or removing a contract must fail closed with CONTRACT_SET_MISMATCH.
  const compatibility = checkCheckpointCompatibility(checkpoint, {
    network: "testnet",
    contracts: [ESCROW_CONTRACT_ID, SECOND_CONTRACT_ID],
  });
  assert.equal(compatibility.valid, false);
  assert.ok(compatibility.error instanceof CheckpointValidationError);
  assert.equal(compatibility.error.code, "CONTRACT_SET_MISMATCH");
});

test("checkpoint validation fails closed on schema-version mismatch", () => {
  const checkpoint = createCheckpoint(
    { network: "testnet", contracts: [ESCROW_CONTRACT_ID] },
    { ledger: 1001, eventIndex: 0 },
  );

  assert.throws(
    () =>
      validateCheckpoint(
        { ...checkpoint, schemaVersion: "soroban-forge/indexer-checkpoint@0" },
        { network: "testnet", contracts: [ESCROW_CONTRACT_ID] },
      ),
    (err: unknown) => {
      assert.ok(err instanceof CheckpointValidationError);
      assert.equal(err.code, "SCHEMA_VERSION_MISMATCH");
      assert.match(err.remediation, /rebuildCheckpoint|discardCheckpoint/);
      return true;
    },
  );
});

test("checkpoint validation rejects malformed JSON, missing fields, and invalid cursor coordinates", () => {
  const runtime: IndexerRuntimeConfig = {
    network: "testnet",
    contracts: [ESCROW_CONTRACT_ID],
  };

  for (const malformed of [
    "",
    "{not-valid-json",
    "[]",
    JSON.stringify({ schemaVersion: CHECKPOINT_SCHEMA_VERSION }),
    JSON.stringify({
      schemaVersion: CHECKPOINT_SCHEMA_VERSION,
      network: "testnet",
      contracts: [],
    }),
  ]) {
    assert.throws(
      () => validateCheckpoint(malformed, runtime),
      (err: unknown) =>
        err instanceof CheckpointValidationError &&
        err.code === "MALFORMED_CHECKPOINT",
    );
  }

  assert.throws(
    () =>
      validateCheckpoint(
        {
          schemaVersion: CHECKPOINT_SCHEMA_VERSION,
          network: "testnet",
          contracts: [ESCROW_CONTRACT_ID],
          cursor: { ledger: -1, eventIndex: 0 },
        },
        runtime,
      ),
    (err: unknown) =>
      err instanceof CheckpointValidationError && err.code === "INVALID_CURSOR",
  );
});

test("operators can explicitly discard or rebuild an invalid persisted checkpoint on disk", () => {
  const tempDir = mkdtempSync(join(tmpdir(), "soroban-forge-checkpoint-"));
  const checkpointPath = join(tempDir, "checkpoint.json");

  try {
    const oldRuntime: IndexerRuntimeConfig = {
      network: "standalone",
      contracts: [ESCROW_CONTRACT_ID],
    };
    const newRuntime: IndexerRuntimeConfig = {
      network: "testnet",
      contracts: [ESCROW_CONTRACT_ID],
    };

    saveCheckpoint(
      checkpointPath,
      createCheckpoint(oldRuntime, { ledger: 999, eventIndex: 0 }),
    );

    // Fails closed before operator repair.
    assert.throws(
      () => loadAndValidateCheckpoint(checkpointPath, newRuntime),
      (err: unknown) =>
        err instanceof CheckpointValidationError &&
        err.code === "NETWORK_MISMATCH",
    );

    // Operator explicitly discards the incompatible checkpoint.
    discardCheckpoint(checkpointPath);
    assert.equal(
      loadAndValidateCheckpoint(checkpointPath, newRuntime),
      undefined,
    );

    // Operator explicitly rebuilds a fresh checkpoint from fixture events.
    const fixtureEvents = loadFixtureEvents("escrow");
    const rebuilt = rebuildCheckpoint(
      checkpointPath,
      newRuntime,
      fixtureEvents,
      "2026-02-01T00:00:00.000Z",
    );
    assert.equal(rebuilt.network, "testnet");
    assert.deepEqual(rebuilt.contracts, [ESCROW_CONTRACT_ID]);
    assert.equal(
      rebuilt.cursor?.ledger,
      fixtureEvents[fixtureEvents.length - 1]!.ledger,
    );

    const reloaded = loadAndValidateCheckpoint(checkpointPath, newRuntime);
    assert.deepEqual(reloaded, rebuilt);
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
});
