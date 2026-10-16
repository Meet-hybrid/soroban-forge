import test from "node:test";
import assert from "node:assert/strict";

import {
  createIndexer,
  filterEvents,
  loadFixtureEvents,
  normalizeEvents,
  deriveEventType,
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
    1,
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