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

test("ledger boundaries are inclusive and field filters compose independently", () => {
  const events = loadFixtureEvents("escrow");
  const atLower = events.find((event) => event.ledger === 1001)!;
  const atUpper = events.find((event) => event.ledger === 1004)!;

  const lowerInclusive = filterEvents(events, { fromLedger: atLower.ledger });
  assert.ok(lowerInclusive.every((event) => event.ledger >= atLower.ledger));
  assert.ok(lowerInclusive.some((event) => event.ledger === atLower.ledger));

  const upperInclusive = filterEvents(events, { toLedger: atUpper.ledger });
  assert.ok(upperInclusive.every((event) => event.ledger <= atUpper.ledger));
  assert.ok(upperInclusive.some((event) => event.ledger === atUpper.ledger));

  const independent = [
    ["contractId", atLower.contractId],
    ["eventType", atLower.eventType],
    ["txHash", atLower.transactionHash],
    ["entityId", atLower.entityId],
  ] as const;
  for (const [field, value] of independent) {
    if (value === undefined) continue;
    const filtered = filterEvents(events, { [field]: value });
    assert.ok(filtered.length > 0, `${field} should match fixture records`);
  }

  const combined = filterEvents(events, {
    contractId: atLower.contractId,
    eventType: atLower.eventType,
    txHash: atLower.transactionHash,
    entityId: atLower.entityId,
    fromLedger: atLower.ledger,
    toLedger: atUpper.ledger,
  });
  assert.ok(
    combined.every(
      (event) =>
        event.contractId === atLower.contractId &&
        event.eventType === atLower.eventType &&
        event.transactionHash === atLower.transactionHash &&
        event.ledger >= atLower.ledger &&
        event.ledger <= atUpper.ledger &&
        (atLower.entityId === undefined || event.entityId === atLower.entityId),
    ),
  );
});

test("omitted filters retain matching records and cursor paging has no gaps or duplicates", () => {
  const events = loadFixtureEvents("escrow");
  assert.deepEqual(filterEvents(events).map((event) => event.id), events.map((event) => event.id));

  const sorted = [...events].sort(
    (left, right) =>
      left.ledger - right.ledger ||
      left.eventIndex - right.eventIndex ||
      left.pagingToken.localeCompare(right.pagingToken),
  );
  const resumeAt = sorted[3];
  assert.ok(resumeAt);
  const firstPage = filterEvents(sorted, {
    toLedger: resumeAt.ledger,
  }).slice(0, 4);
  const secondPage = filterEvents(sorted, { after: {
    ledger: firstPage[lastPageIndex(firstPage)].ledger,
    eventIndex: firstPage[lastPageIndex(firstPage)].eventIndex,
  } });

  const ids = [...firstPage, ...secondPage].map((event) => event.id);
  assert.equal(new Set(ids).size, ids.length);
  assert.deepEqual(
    [...ids].sort(),
    [...sorted.map((event) => event.id)].sort(),
  );
  assert.ok(secondPage.every((event) => event.ledger >= resumeAt.ledger));
});

function lastPageIndex(page: readonly unknown[]): number {
  return page.length - 1;
}

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
