import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { Address, StrKey, nativeToScVal, xdr } from "@stellar/stellar-sdk";

import {
  FORGE_EVENT_SPECS,
  decodeContractEvent,
  decodeDaoEvent,
  decodeEscrowEvent,
  decodeMarketplaceEvent,
  decodeMultisigEvent,
  decodeSubscriptionEvent,
  decodeVestingEvent,
  eventTopicSymbol,
  type EscrowContractEvent,
  type EventDecodeError,
  type EventDecodeResult,
  type ForgeContractName,
} from "../event-decoders.js";
import { createIndexer, normalizeEvents } from "../indexer.js";

// ---------------------------------------------------------------- helpers

const show = (value: unknown) =>
  JSON.stringify(value, (_key, v) => (typeof v === "bigint" ? `${v}n` : v));

function okEvent<E>(result: EventDecodeResult<E>): E {
  assert.equal(result.ok, true, show(result));
  return (result as { ok: true; event: E }).event;
}

function errorOf(result: EventDecodeResult<unknown>): EventDecodeError {
  assert.equal(result.ok, false, show(result));
  return (result as { ok: false; error: EventDecodeError }).error;
}

const sym = (name: string) => xdr.ScVal.scvSymbol(name);
const u64 = (n: bigint) => nativeToScVal(n, { type: "u64" });
const i128 = (n: bigint) => nativeToScVal(n, { type: "i128" });
const u32 = (n: number) => xdr.ScVal.scvU32(n);
const contractId = (n: number) => StrKey.encodeContract(Buffer.alloc(32, n));
const addr = (n: number) => new Address(contractId(n)).toScVal();
const map = (entries: Array<[string, xdr.ScVal]>) =>
  xdr.ScVal.scvMap(
    entries.map(([key, val]) => new xdr.ScMapEntry({ key: sym(key), val })),
  );
const raw = (topics: xdr.ScVal[], value: xdr.ScVal) => ({
  topic: topics.map((topic) => topic.toXDR("base64")),
  value: value.toXDR("base64"),
});

function fixtureEvents(file: string): Array<{ topic: string[]; value: string }> {
  const url = new URL(`../../../../indexer/fixtures/${file}`, import.meta.url);
  return JSON.parse(readFileSync(url, "utf8")).events;
}

// Compile-time check that the union narrows on `event`.
function escrowId(event: EscrowContractEvent): bigint {
  switch (event.event) {
    case "DeadlineExtended":
      return event.fields.new_deadline;
    case "ReleaseRequested":
      return event.fields.release_at;
    default:
      return event.fields.escrow_id;
  }
}

// ------------------------------------------------- ABI fixture sync

for (const contract of Object.keys(FORGE_EVENT_SPECS) as ForgeContractName[]) {
  test(`${contract}: decoder specs match the checked-in ABI fixture`, () => {
    const abi = JSON.parse(
      readFileSync(
        new URL(`../../src/abi-fixtures/${contract}.json`, import.meta.url),
        "utf8",
      ),
    );
    const specs = FORGE_EVENT_SPECS[contract] as Record<
      string,
      { topics: number; fields: Record<string, string> }
    >;
    assert.deepEqual(
      Object.keys(specs).sort(),
      abi.events.map((event: { name: string }) => event.name).sort(),
      `${contract}: event names differ from the ABI fixture`,
    );
    for (const event of abi.events) {
      const spec = specs[event.name];
      assert.equal(event.dataFormat, "scSpecEventDataFormatMap");
      assert.deepEqual(
        Object.entries(spec.fields).map(([name, type]) => ({ name, type })),
        event.params,
        `${contract}.${event.name}: fields differ from the ABI fixture`,
      );
      assert.ok(spec.topics >= 1 && spec.topics <= Object.keys(spec.fields).length);
    }
  });
}

test("topic symbols are the snake_case of the event name", () => {
  assert.equal(eventTopicSymbol("EscrowCreated"), "escrow_created");
  assert.equal(eventTopicSymbol("TxSubmitted"), "tx_submitted");
  assert.equal(eventTopicSymbol("BeneficiaryReassignedFrom"), "beneficiary_reassigned_from");
  assert.equal(eventTopicSymbol("Proposed"), "proposed");
});

// ------------------------------------------ real indexer fixture events

const FIXTURES: Array<[ForgeContractName, string, string[]]> = [
  ["escrow", "escrow-events.json", ["EscrowCreated"]],
  ["vesting", "vesting-events.json", ["BeneficiaryReassignedFrom", "BeneficiaryReassignedTo"]],
  ["multisig", "multi-sig-wallet-events.json", ["TxSubmitted", "TxConfirmed"]],
  ["subscription", "subscription-payments-events.json", ["Charged", "Cancelled"]],
  ["marketplace", "marketplace-royalties-events.json", ["RoyaltyConfigured"]],
  ["dao", "dao-governance-events.json", ["Proposed"]],
];

for (const [contract, file, expected] of FIXTURES) {
  test(`${contract}: every event in ${file} decodes or is a clean unknown_event`, () => {
    const events = fixtureEvents(file);
    assert.ok(events.length > 0);
    const decoded: string[] = [];
    for (const event of events) {
      const result = decodeContractEvent(contract, event);
      if (result.ok) {
        assert.equal(result.event.contract, contract);
        decoded.push(result.event.event);
      } else {
        // Token transfer/mint events published by other contracts are not ours.
        assert.equal(result.error.code, "unknown_event", show(result));
      }
    }
    for (const name of expected) {
      assert.ok(decoded.includes(name), `${name} not decoded; got ${show(decoded)}`);
    }
  });
}

test("fixture values: escrow, multisig, vesting, subscription, marketplace, dao", () => {
  const find = (file: string, decode: (e: never) => EventDecodeResult<{ event: string; fields: any }>, name: string) => {
    for (const event of fixtureEvents(file)) {
      const result = decode(event as never);
      if (result.ok && result.event.event === name) return result.event.fields;
    }
    throw new Error(`${name} not found in ${file}`);
  };
  const strkey = /^[CG][A-Z2-7]{55}$/;

  const escrow = find("escrow-events.json", decodeEscrowEvent as never, "EscrowCreated");
  assert.equal(escrow.escrow_id, 1n);
  assert.equal(escrow.data.escrow_id, 1n);
  assert.equal(escrow.data.timeout, 1000n);
  assert.equal(typeof escrow.data.amount, "bigint");

  const submitted = find("multi-sig-wallet-events.json", decodeMultisigEvent as never, "TxSubmitted");
  assert.equal(submitted.tx_id, 1n);
  assert.equal(typeof submitted.payload_len, "number");
  assert.match(submitted.submitter, strkey);

  const reassigned = find("vesting-events.json", decodeVestingEvent as never, "BeneficiaryReassignedFrom");
  assert.equal(reassigned.schedule_id, 1n);
  assert.match(reassigned.beneficiary, strkey);
  assert.match(reassigned.new_beneficiary, strkey);
  assert.equal(typeof reassigned.vested_unclaimed, "bigint");
  assert.equal(typeof reassigned.reassignment_count, "number");

  const charged = find("subscription-payments-events.json", decodeSubscriptionEvent as never, "Charged");
  assert.equal(charged.subscription_id, 1n);
  assert.equal(typeof charged.amount, "bigint");
  assert.equal(typeof charged.next_charge_at, "bigint");

  const royalty = find("marketplace-royalties-events.json", decodeMarketplaceEvent as never, "RoyaltyConfigured");
  assert.match(royalty.collection, strkey);
  assert.match(royalty.recipient, strkey);
  assert.ok(royalty.bps > 0);

  const proposed = find("dao-governance-events.json", decodeDaoEvent as never, "Proposed");
  assert.equal(proposed.proposal_id, 1n);
  assert.equal(proposed.data.proposal_id, 1n);
});

// -------------------------- hand-built XDR: one event per contract

test("escrow: DeadlineExtended decodes and narrows", () => {
  const event = okEvent(
    decodeEscrowEvent(
      raw(
        [sym("deadline_extended"), u64(7n)],
        map([["new_deadline", u64(300n)], ["old_deadline", u64(200n)]]),
      ),
    ),
  );
  assert.deepEqual(event, {
    contract: "escrow",
    event: "DeadlineExtended",
    fields: { escrow_id: 7n, old_deadline: 200n, new_deadline: 300n },
  });
  assert.equal(escrowId(event), 300n);
});

test("vesting: RevokedRefunded has two topic fields", () => {
  const event = okEvent(
    decodeVestingEvent(
      raw([sym("revoked_refunded"), u64(3n), addr(5)], map([["amount", i128(500n)]])),
    ),
  );
  assert.deepEqual(event, {
    contract: "vesting",
    event: "RevokedRefunded",
    fields: { schedule_id: 3n, funder: contractId(5), amount: 500n },
  });
});

test("vesting: TrancheRevoked decodes topic and refund fields", () => {
  const event = okEvent(
    decodeVestingEvent(
      raw(
        [sym("tranche_revoked"), u64(4n), addr(5)],
        map([["refunded_to_funder", i128(750n)], ["unlocked_at_revocation", i128(250n)]]),
      ),
    ),
  );
  assert.deepEqual(event, {
    contract: "vesting",
    event: "TrancheRevoked",
    fields: {
      schedule_id: 4n,
      funder: contractId(5),
      unlocked_at_revocation: 250n,
      refunded_to_funder: 750n,
    },
  });
});

test("multisig: WithdrawalLimitRemoved accepts a void or empty-map value", () => {
  const expected = {
    contract: "multisig",
    event: "WithdrawalLimitRemoved",
    fields: { token: contractId(6) },
  };
  for (const value of [xdr.ScVal.scvVoid(), map([])]) {
    assert.deepEqual(
      okEvent(decodeMultisigEvent(raw([sym("withdrawal_limit_removed"), addr(6)], value))),
      expected,
    );
  }
});

test("subscription: UsageRecorded decodes a symbol topic field", () => {
  const event = okEvent(
    decodeSubscriptionEvent(
      raw(
        [sym("usage_recorded"), u64(2n), sym("api_calls")],
        map([["period_start", u64(100n)], ["period_units", u64(40n)], ["units", u64(10n)]]),
      ),
    ),
  );
  assert.deepEqual(event, {
    contract: "subscription",
    event: "UsageRecorded",
    fields: { subscription_id: 2n, metric: "api_calls", units: 10n, period_units: 40n, period_start: 100n },
  });
});

test("marketplace: SaleSettled decodes addresses and amounts", () => {
  const event = okEvent(
    decodeMarketplaceEvent(
      raw(
        [sym("sale_settled"), addr(1)],
        map([
          ["gross_amount", i128(1000n)],
          ["payer", addr(2)],
          ["royalty_recipient", addr(3)],
          ["royalty_share", i128(100n)],
          ["seller", addr(4)],
          ["seller_net", i128(900n)],
          ["token", addr(5)],
        ]),
      ),
    ),
  );
  assert.deepEqual(event, {
    contract: "marketplace",
    event: "SaleSettled",
    fields: {
      collection: contractId(1),
      token: contractId(5),
      payer: contractId(2),
      seller: contractId(4),
      royalty_recipient: contractId(3),
      gross_amount: 1000n,
      seller_net: 900n,
      royalty_share: 100n,
    },
  });
});

test("dao: Finalised decodes an enum state", () => {
  const event = okEvent(
    decodeDaoEvent(
      raw(
        [sym("finalised"), u64(4n)],
        map([
          ["against_votes", i128(3n)],
          ["for_votes", i128(9n)],
          ["state", xdr.ScVal.scvVec([sym("Passed")])],
        ]),
      ),
    ),
  );
  assert.deepEqual(event, {
    contract: "dao",
    event: "Finalised",
    fields: { proposal_id: 4n, state: ["Passed"], for_votes: 9n, against_votes: 3n },
  });
});

test("decodeContractEvent dispatches by contract name", () => {
  const event = raw([sym("tx_cancelled"), u64(9n)], map([["submitter", addr(1)]]));
  assert.equal(okEvent(decodeContractEvent("multisig", event)).event, "TxCancelled");
  assert.equal(errorOf(decodeContractEvent("escrow", event)).code, "unknown_event");
  assert.equal(errorOf(decodeContractEvent("nope" as never, event)).code, "invalid_input");
});

// ---------------------------------- unknown and malformed events

const submitted = (value: xdr.ScVal, topics: xdr.ScVal[] = [sym("tx_submitted"), u64(1n)]) =>
  raw(topics, value);

test("unknown events return unknown_event with the symbol", () => {
  const error = errorOf(decodeEscrowEvent(raw([sym("totally_unknown"), u64(1n)], map([]))));
  assert.equal(error.code, "unknown_event");
  assert.equal(error.eventName, "totally_unknown");
  assert.equal(error.contract, "escrow");
  assert.equal(
    errorOf(decodeVestingEvent(raw([sym("escrow_created"), u64(1n)], map([])))).code,
    "unknown_event",
  );
});

test("malformed input never throws and returns typed errors", () => {
  const good = map([["payload_len", u32(7)], ["submitter", addr(1)]]);
  const cases: Array<[string, unknown, string]> = [
    ["null", null, "invalid_input"],
    ["undefined", undefined, "invalid_input"],
    ["empty object", {}, "invalid_input"],
    ["topic not an array", { topic: "x", value: "" }, "invalid_input"],
    ["value not a string", { topic: [], value: 1 }, "invalid_input"],
    ["no topics", { topic: [], value: good.toXDR("base64") }, "malformed_topic"],
    ["topic not a string", { topic: [42], value: good.toXDR("base64") }, "malformed_topic"],
    ["topic not XDR", { topic: ["@@@@"], value: good.toXDR("base64") }, "malformed_topic"],
    ["topic[0] not a symbol", submitted(good, [u64(1n), u64(1n)]), "malformed_topic"],
    ["too few topics", submitted(good, [sym("tx_submitted")]), "malformed_topic"],
    ["too many topics", submitted(good, [sym("tx_submitted"), u64(1n), u64(2n)]), "malformed_topic"],
    ["topic field wrong type", submitted(good, [sym("tx_submitted"), sym("one")]), "invalid_field"],
    ["value not XDR", { topic: submitted(good).topic, value: "@@@@" }, "malformed_value"],
    ["empty value", { topic: submitted(good).topic, value: "" }, "malformed_value"],
    ["value not a map", submitted(u64(7n)), "malformed_value"],
    ["void value with data fields", submitted(xdr.ScVal.scvVoid()), "malformed_value"],
    ["missing field", submitted(map([["submitter", addr(1)]])), "missing_field"],
    ["field wrong type", submitted(map([["payload_len", u64(7n)], ["submitter", addr(1)]])), "invalid_field"],
  ];
  for (const [label, input, code] of cases) {
    let result: EventDecodeResult<unknown> | undefined;
    assert.doesNotThrow(() => {
      result = decodeMultisigEvent(input as never);
    }, label);
    assert.equal(errorOf(result!).code, code, `${label}: ${show(result)}`);
  }
});

test("field errors name the offending field", () => {
  const error = errorOf(decodeMultisigEvent(submitted(map([["submitter", addr(1)]]))));
  assert.equal(error.field, "payload_len");
  assert.equal(error.eventName, "TxSubmitted");
});

test("extra unknown fields in the value map are tolerated", () => {
  const event = okEvent(
    decodeMultisigEvent(
      submitted(map([["extra", u32(1)], ["payload_len", u32(7)], ["submitter", addr(1)]])),
    ),
  );
  assert.equal(event.event, "TxSubmitted");
});

// ------------------------------------------------ indexer integration

test("createIndexer can optionally decode events with the typed decoders", () => {
  const a = contractId(1);
  const b = contractId(2);
  const payload = raw(
    [sym("deadline_extended"), u64(7n)],
    map([["new_deadline", u64(300n)], ["old_deadline", u64(200n)]]),
  );
  const eventA = { ...payload, contractId: a, ledger: 10, eventIndex: 0, id: "10-0", pagingToken: "10-0" };
  const eventB = { ...payload, contractId: b, ledger: 11, eventIndex: 0, id: "11-0", pagingToken: "11-0" };

  const indexer = createIndexer({ events: [eventA, eventB], decoders: { [a]: "escrow" } });
  const decoded = indexer.decoded();
  assert.equal(decoded.length, 2);
  assert.equal(
    okEvent(decoded[0].result as EventDecodeResult<{ event: string }>).event,
    "DeadlineExtended",
  );
  assert.equal(decoded[1].result, null);
  assert.equal(indexer.decoded({ contractId: a }).length, 1);
  assert.equal(indexer.append([]).decoded().length, 2);
  assert.equal(indexer.append([]).decoded()[0].result?.ok, true);

  const plain = createIndexer({ events: [eventA] });
  assert.equal(plain.decoded()[0].result, null);
  assert.deepEqual(indexer.events, normalizeEvents([eventA, eventB]));
});
