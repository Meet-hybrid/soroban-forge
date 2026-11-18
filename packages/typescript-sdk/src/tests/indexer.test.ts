import test from "node:test";
import assert from "node:assert/strict";

import {
  createIndexer,
  filterEvents,
  loadFixtureEvents,
  normalizeEvents,
  deriveEventType,
  parseIndexerCheckpoint,
  serializeIndexerCheckpoint,
  type ContractEventRecord,
} from "../indexer.js";

test("fixture events can be loaded and normalized deterministically", () => {
  const events = loadFixtureEvents("escrow");
  assert.ok(events.length > 0, "fixture must contain at least one event");

  const normalized = normalizeEvents(events);
  assert.deepEqual(
    normalized.map((event) => event.id),
    events.map((event) => event.id),
  );
  assert.ok(
    normalized.some(
      (event) =>
        event.contractId ===
        "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHK3M",
    ),
    "fixture should include the escrow contract events",
  );
});

test("indexer filters by ledger and contract metadata", () => {
  const events = loadFixtureEvents("escrow");
  const indexer = createIndexer({ events });

  const filtered = indexer.filter({
    contractId: "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHK3M",
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
  assert.equal(
    filterEvents(loadFixtureEvents("escrow"), { eventType: "escrow_created" })
      .length,
    7,
  );
});

test("vesting fixture is loadable through the public indexer aliases", () => {
  const events = loadFixtureEvents("vesting");
  assert.ok(
    events.length > 0,
    "vesting fixture should contain at least one event",
  );
  assert.ok(
    events.every((event) => event.contractId.length > 0),
    "all vesting events should have a contract id",
  );
});

test("checkpoint resume preserves the cursor and avoids replaying indexed events", () => {
  const [first, second] = loadFixtureEvents("escrow");
  assert.ok(first);
  assert.ok(second);
  const identity = {
    network: "Test SDF Network ; September 2015",
    contractIds: [first.contractId],
  };
  const initial = createIndexer({ events: [first], ...identity });
  const serializedCheckpoint = serializeIndexerCheckpoint(
    initial.checkpoint()!,
  );
  const resumed = createIndexer({
    ...identity,
    checkpoint: serializedCheckpoint,
  });

  assert.deepEqual(resumed.cursor(), initial.cursor());
  const replayed = resumed.append([first, second]);
  assert.deepEqual(
    replayed.events.map((event) => event.id),
    [second.id],
  );
  assert.deepEqual(replayed.cursor(), {
    ledger: second.ledger,
    eventIndex: second.eventIndex,
    pagingToken: second.pagingToken,
  });
});

test("checkpoint rejects a different network", () => {
  const event = loadFixtureEvents("escrow")[0];
  assert.ok(event);
  const checkpoint = serializeIndexerCheckpoint(
    createIndexer({
      events: [event],
      network: "testnet",
      contractIds: [event.contractId],
    }).checkpoint()!,
  );

  assert.throws(
    () =>
      parseIndexerCheckpoint(checkpoint, {
        network: "mainnet",
        contractIds: [event.contractId],
      }),
    /network mismatch/,
  );
});

test("checkpoint rejects a changed contract set", () => {
  const event = loadFixtureEvents("escrow")[0];
  assert.ok(event);
  const checkpoint = serializeIndexerCheckpoint(
    createIndexer({
      events: [event],
      network: "testnet",
      contractIds: [event.contractId],
    }).checkpoint()!,
  );

  assert.throws(
    () =>
      parseIndexerCheckpoint(checkpoint, {
        network: "testnet",
        contractIds: [event.contractId, "COTHER"],
      }),
    /contract set mismatch/,
  );
});

test("checkpoint rejects malformed persisted data", () => {
  const identity = { network: "testnet", contractIds: ["CCONTRACT"] };
  assert.throws(
    () => parseIndexerCheckpoint("{", identity),
    /could not be parsed/,
  );
  assert.throws(
    () =>
      parseIndexerCheckpoint(
        JSON.stringify({
          schemaVersion: 1,
          network: "testnet",
          contractIds: ["CCONTRACT"],
          cursor: { ledger: -1, eventIndex: 0 },
        }),
        identity,
      ),
    /cursor must contain valid/,
  );
});

test("checkpoint rejects an unsupported schema version", () => {
  assert.throws(
    () =>
      parseIndexerCheckpoint(
        JSON.stringify({
          schemaVersion: 2,
          network: "testnet",
          contractIds: ["CCONTRACT"],
          cursor: { ledger: 1, eventIndex: 0 },
        }),
        { network: "testnet", contractIds: ["CCONTRACT"] },
      ),
    /schema version 2 is unsupported/,
  );
});
