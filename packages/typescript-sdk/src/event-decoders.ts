/**
 * Typed decoders for the events emitted by the six Soroban Forge contracts.
 *
 * A raw event (the base64 XDR `topic` array and `value` returned by Soroban
 * RPC `getEvents`) is decoded into a discriminated union keyed by event name.
 * Decoding never throws: unknown or malformed events produce
 * `{ ok: false, error }`.
 *
 * The event layouts below mirror `src/abi-fixtures/*.json`, which are generated
 * from the contract specs. `src/tests/event-decoders.test.ts` fails if the two
 * drift apart, so these decoders stay in sync with the generated clients.
 *
 * Wire format (soroban-sdk `#[contractevent]`):
 * - `topic[0]` is the snake_case event name as a symbol.
 * - `topic[1..]` are the `#[topic]` fields, in declaration order.
 * - `value` is a map of the remaining fields keyed by symbol.
 */
import { scValToNative, xdr } from "@stellar/stellar-sdk";
import type { ContractEventRecord } from "./indexer.js";

/** A decoded user-defined struct (for example `EscrowData`). */
export type ForgeStruct = { readonly [field: string]: unknown };
/** A decoded vector (for example `Vec<MetricQuota>`). */
export type ForgeVec = ReadonlyArray<unknown>;
/** A decoded user-defined enum: `["Variant", ...payload]` or a plain integer. */
export type ForgeEnum = ReadonlyArray<unknown> | number;

/** Maps the ABI type names used by the fixtures to their decoded TS types. */
export interface ForgeAbiTypeMap {
  u32: number;
  u64: bigint;
  i128: bigint;
  bool: boolean;
  address: string;
  symbol: string;
  "udt:EscrowData": ForgeStruct;
  "udt:BasketEscrowData": ForgeStruct;
  "udt:Proposal": ForgeStruct;
  "udt:ProposalState": ForgeEnum;
  "vec<udt:MetricQuota>": ForgeVec;
}
export type ForgeAbiType = keyof ForgeAbiTypeMap;

/** Layout of one event: how many leading fields are topics, and all fields in ABI order. */
export interface ForgeEventSpec {
  readonly topics: number;
  readonly fields: { readonly [name: string]: ForgeAbiType };
}
export type ForgeEventSpecs = { readonly [event: string]: ForgeEventSpec };

const ESCROW_DATA = { escrow_id: "u64", data: "udt:EscrowData" } as const;
const BASKET_DATA = { escrow_id: "u64", data: "udt:BasketEscrowData" } as const;
const SUBSCRIPTION_BALANCE = {
  subscription_id: "u64",
  amount: "i128",
  balance_after: "i128",
} as const;

export const ESCROW_EVENT_SPECS = {
  ArbiterReplaced: {
    topics: 1,
    fields: { escrow_id: "u64", old_arbiter: "address", new_arbiter: "address" },
  },
  BasketCancelled: { topics: 1, fields: BASKET_DATA },
  BasketCreated: { topics: 1, fields: BASKET_DATA },
  BasketDeposited: { topics: 1, fields: BASKET_DATA },
  BasketDisputed: { topics: 1, fields: BASKET_DATA },
  BasketPartiallyReleased: {
    topics: 1,
    fields: {
      escrow_id: "u64",
      token: "address",
      partial_amount: "i128",
      data: "udt:BasketEscrowData",
    },
  },
  BasketRefunded: { topics: 1, fields: BASKET_DATA },
  BasketReleased: { topics: 1, fields: BASKET_DATA },
  BasketResolved: {
    topics: 1,
    fields: {
      escrow_id: "u64",
      data: "udt:BasketEscrowData",
      in_favor_of_seller: "bool",
    },
  },
  Cancelled: { topics: 1, fields: ESCROW_DATA },
  DeadlineExtended: {
    topics: 1,
    fields: { escrow_id: "u64", old_deadline: "u64", new_deadline: "u64" },
  },
  Deposited: { topics: 1, fields: ESCROW_DATA },
  DisputeResolvedWithFee: {
    topics: 1,
    fields: {
      escrow_id: "u64",
      arbiter_fee: "i128",
      seller_award: "i128",
      buyer_award: "i128",
    },
  },
  Disputed: { topics: 1, fields: ESCROW_DATA },
  EscrowCreated: { topics: 1, fields: ESCROW_DATA },
  PartiallyReleased: {
    topics: 1,
    fields: {
      escrow_id: "u64",
      partial_amount: "i128",
      data: "udt:EscrowData",
    },
  },
  RefundExpired: {
    topics: 1,
    fields: { escrow_id: "u64", refunded_amount: "i128", timestamp: "u64" },
  },
  Refunded: { topics: 1, fields: ESCROW_DATA },
  ReleaseRequested: {
    topics: 1,
    fields: { escrow_id: "u64", amount: "i128", release_at: "u64" },
  },
  Released: { topics: 1, fields: ESCROW_DATA },
  Resolved: {
    topics: 1,
    fields: {
      escrow_id: "u64",
      data: "udt:EscrowData",
      in_favor_of_seller: "bool",
    },
  },
  ResolvedSplit: {
    topics: 1,
    fields: {
      escrow_id: "u64",
      data: "udt:EscrowData",
      seller_bps: "u32",
      seller_share: "i128",
      buyer_share: "i128",
    },
  },
} as const;

export const VESTING_EVENT_SPECS = {
  BeneficiaryReassignedFrom: {
    topics: 2,
    fields: {
      schedule_id: "u64",
      beneficiary: "address",
      new_beneficiary: "address",
      vested_unclaimed: "i128",
      reassignment_count: "u32",
    },
  },
  BeneficiaryReassignedTo: {
    topics: 2,
    fields: {
      schedule_id: "u64",
      beneficiary: "address",
      old_beneficiary: "address",
      vested_unclaimed: "i128",
      reassignment_count: "u32",
    },
  },
  RevokedRefunded: {
    topics: 2,
    fields: { schedule_id: "u64", funder: "address", amount: "i128" },
  },
  ScheduleCreated: {
    topics: 1,
    fields: {
      schedule_id: "u64",
      beneficiary: "address",
      total_amount: "i128",
      cliff: "u64",
      duration: "u64",
    },
  },
} as const;

export const MULTISIG_EVENT_SPECS = {
  TxCancelled: { topics: 1, fields: { tx_id: "u64", submitter: "address" } },
  TxConfirmed: {
    topics: 1,
    fields: { tx_id: "u64", signer: "address", confirmations_count: "u32" },
  },
  TxExecuted: {
    topics: 1,
    fields: { tx_id: "u64", confirmations_count: "u32", threshold: "u32" },
  },
  TxRejected: { topics: 1, fields: { tx_id: "u64", rejector: "address" } },
  TxSubmitted: {
    topics: 1,
    fields: { tx_id: "u64", submitter: "address", payload_len: "u32" },
  },
  WalletPaused: { topics: 1, fields: { wallet: "address", timestamp: "u64" } },
  WalletResumed: { topics: 1, fields: { wallet: "address", timestamp: "u64" } },
  WithdrawalLimitRemoved: { topics: 1, fields: { token: "address" } },
  WithdrawalLimitSet: {
    topics: 1,
    fields: { token: "address", amount: "i128", window_seconds: "u64" },
  },
} as const;

export const SUBSCRIPTION_EVENT_SPECS = {
  AutoTopupTriggered: { topics: 1, fields: SUBSCRIPTION_BALANCE },
  BalanceDebited: { topics: 1, fields: SUBSCRIPTION_BALANCE },
  BalanceRefunded: { topics: 1, fields: SUBSCRIPTION_BALANCE },
  Cancelled: {
    topics: 1,
    fields: { subscription_id: "u64", subscriber: "address" },
  },
  Charged: {
    topics: 1,
    fields: {
      subscription_id: "u64",
      amount: "i128",
      last_charged: "u64",
      next_charge_at: "u64",
    },
  },
  Deposited: { topics: 1, fields: SUBSCRIPTION_BALANCE },
  PlanChanged: {
    topics: 1,
    fields: {
      subscription_id: "u64",
      old_amount: "i128",
      new_amount: "i128",
      credit: "i128",
    },
  },
  QuotasSet: {
    topics: 1,
    fields: { subscription_id: "u64", quotas: "vec<udt:MetricQuota>" },
  },
  UsageRecorded: {
    topics: 2,
    fields: {
      subscription_id: "u64",
      metric: "symbol",
      units: "u64",
      period_units: "u64",
      period_start: "u64",
    },
  },
} as const;

export const MARKETPLACE_EVENT_SPECS = {
  RoyaltyConfigured: {
    topics: 1,
    fields: { collection: "address", recipient: "address", bps: "u32" },
  },
  SaleSettled: {
    topics: 1,
    fields: {
      collection: "address",
      token: "address",
      payer: "address",
      seller: "address",
      royalty_recipient: "address",
      gross_amount: "i128",
      seller_net: "i128",
      royalty_share: "i128",
    },
  },
} as const;

export const DAO_EVENT_SPECS = {
  BondPosted: {
    topics: 1,
    fields: { proposal_id: "u64", token: "address", amount: "i128" },
  },
  BondReleased: {
    topics: 1,
    fields: {
      proposal_id: "u64",
      token: "address",
      amount: "i128",
      to: "address",
      forfeited: "bool",
    },
  },
  Finalised: {
    topics: 1,
    fields: {
      proposal_id: "u64",
      state: "udt:ProposalState",
      for_votes: "i128",
      against_votes: "i128",
    },
  },
  Proposed: {
    topics: 1,
    fields: { proposal_id: "u64", data: "udt:Proposal" },
  },
  VoteCast: {
    topics: 1,
    fields: {
      proposal_id: "u64",
      voter: "address",
      support: "bool",
      weight: "i128",
    },
  },
} as const;

/** Contract names; they match the unified SDK namespaces and `abi-fixtures/<name>.json`. */
export const FORGE_CONTRACT_NAMES = [
  "escrow",
  "vesting",
  "multisig",
  "subscription",
  "marketplace",
  "dao",
] as const;
export type ForgeContractName = (typeof FORGE_CONTRACT_NAMES)[number];

export const FORGE_EVENT_SPECS = {
  escrow: ESCROW_EVENT_SPECS,
  vesting: VESTING_EVENT_SPECS,
  multisig: MULTISIG_EVENT_SPECS,
  subscription: SUBSCRIPTION_EVENT_SPECS,
  marketplace: MARKETPLACE_EVENT_SPECS,
  dao: DAO_EVENT_SPECS,
} as const;

type FieldTypes<F> = {
  readonly [K in keyof F]: F[K] extends ForgeAbiType
    ? ForgeAbiTypeMap[F[K]]
    : never;
};

/** Builds the discriminated union of typed events for one contract's specs. */
export type ForgeEventsOf<C extends string, S extends ForgeEventSpecs> = {
  [N in keyof S & string]: {
    readonly contract: C;
    readonly event: N;
    readonly fields: FieldTypes<S[N]["fields"]>;
  };
}[keyof S & string];

export type EscrowContractEvent = ForgeEventsOf<"escrow", typeof ESCROW_EVENT_SPECS>;
export type VestingContractEvent = ForgeEventsOf<"vesting", typeof VESTING_EVENT_SPECS>;
export type MultisigContractEvent = ForgeEventsOf<"multisig", typeof MULTISIG_EVENT_SPECS>;
export type SubscriptionContractEvent = ForgeEventsOf<"subscription", typeof SUBSCRIPTION_EVENT_SPECS>;
export type MarketplaceContractEvent = ForgeEventsOf<"marketplace", typeof MARKETPLACE_EVENT_SPECS>;
export type DaoContractEvent = ForgeEventsOf<"dao", typeof DAO_EVENT_SPECS>;

/** Any decoded event from any of the six contracts (discriminate on `contract` then `event`). */
export type ForgeContractEvent =
  | EscrowContractEvent
  | VestingContractEvent
  | MultisigContractEvent
  | SubscriptionContractEvent
  | MarketplaceContractEvent
  | DaoContractEvent;

/** The raw event shape accepted by the decoders (`ContractEventRecord` satisfies it). */
export interface RawContractEvent {
  readonly topic: ReadonlyArray<string>;
  readonly value: string;
}

export type EventDecodeErrorCode =
  | "invalid_input"
  | "malformed_topic"
  | "unknown_event"
  | "malformed_value"
  | "missing_field"
  | "invalid_field";

export interface EventDecodeError {
  readonly code: EventDecodeErrorCode;
  readonly message: string;
  /** The contract decoder that produced this error. */
  readonly contract: string;
  /** The topic symbol or event name when it could be determined. */
  readonly eventName?: string;
  /** The offending field for `missing_field` and `invalid_field`. */
  readonly field?: string;
}

export type EventDecodeResult<E> =
  | { readonly ok: true; readonly event: E }
  | { readonly ok: false; readonly error: EventDecodeError };

export type ForgeEventDecodeResult = EventDecodeResult<ForgeContractEvent>;

const SCVAL_KINDS: Record<ForgeAbiType, ReadonlyArray<string>> = {
  u32: ["scvU32"],
  u64: ["scvU64"],
  i128: ["scvI128"],
  bool: ["scvBool"],
  address: ["scvAddress"],
  symbol: ["scvSymbol"],
  "udt:EscrowData": ["scvMap"],
  "udt:BasketEscrowData": ["scvMap"],
  "udt:Proposal": ["scvMap"],
  "udt:ProposalState": ["scvVec", "scvU32"],
  "vec<udt:MetricQuota>": ["scvVec"],
};

/** `EscrowCreated` -> `escrow_created`: the symbol a contract publishes in `topic[0]`. */
export function eventTopicSymbol(eventName: string): string {
  return eventName.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();
}

const SYMBOL_TABLES = {} as Record<ForgeContractName, Map<string, string>>;
for (const contract of FORGE_CONTRACT_NAMES) {
  const table = new Map<string, string>();
  for (const name of Object.keys(FORGE_EVENT_SPECS[contract])) {
    table.set(eventTopicSymbol(name), name);
  }
  SYMBOL_TABLES[contract] = table;
}

function scvKind(value: xdr.ScVal): string {
  return (value.switch() as unknown as { name: string }).name;
}

function fail(
  contract: string,
  code: EventDecodeErrorCode,
  message: string,
  extra: { eventName?: string; field?: string } = {},
): { ok: false; error: EventDecodeError } {
  return { ok: false, error: { code, message, contract, ...extra } };
}

function convertField(
  field: string,
  abiType: ForgeAbiType,
  value: xdr.ScVal,
): { value: unknown } | { message: string } {
  const actual = scvKind(value);
  if (!SCVAL_KINDS[abiType].includes(actual)) {
    return {
      message: `field '${field}' (${abiType}) has unexpected ScVal type ${actual}`,
    };
  }
  try {
    return { value: scValToNative(value) };
  } catch (error) {
    return {
      message: `field '${field}' could not be decoded: ${error instanceof Error ? error.message : String(error)}`,
    };
  }
}

function decodeUnchecked(
  contract: ForgeContractName,
  specs: ForgeEventSpecs,
  raw: RawContractEvent,
): EventDecodeResult<unknown> {
  if (
    raw === null ||
    typeof raw !== "object" ||
    !Array.isArray(raw.topic) ||
    typeof raw.value !== "string"
  ) {
    return fail(
      contract,
      "invalid_input",
      "event must be an object with a `topic` string array and a `value` string",
    );
  }
  if (raw.topic.length === 0) {
    return fail(contract, "malformed_topic", "event has no topics");
  }

  const topics: xdr.ScVal[] = [];
  for (let i = 0; i < raw.topic.length; i++) {
    const encoded = raw.topic[i];
    if (typeof encoded !== "string") {
      return fail(contract, "malformed_topic", `topic[${i}] is not a string`);
    }
    try {
      topics.push(xdr.ScVal.fromXDR(encoded, "base64"));
    } catch {
      return fail(
        contract,
        "malformed_topic",
        `topic[${i}] is not valid base64 ScVal XDR`,
      );
    }
  }
  if (scvKind(topics[0]) !== "scvSymbol") {
    return fail(
      contract,
      "malformed_topic",
      `topic[0] must be a symbol event name but found ${scvKind(topics[0])}`,
    );
  }

  const symbol = String(scValToNative(topics[0]));
  const name = SYMBOL_TABLES[contract].get(symbol);
  if (name === undefined) {
    return fail(
      contract,
      "unknown_event",
      `'${symbol}' is not a known ${contract} event`,
      { eventName: symbol },
    );
  }

  const spec = specs[name];
  if (topics.length - 1 !== spec.topics) {
    return fail(
      contract,
      "malformed_topic",
      `${name} expects ${spec.topics} topic field(s) after the name but found ${topics.length - 1}`,
      { eventName: name },
    );
  }

  const fieldNames = Object.keys(spec.fields);
  const topicNames = fieldNames.slice(0, spec.topics);
  const dataNames = fieldNames.slice(spec.topics);
  const fields: Record<string, unknown> = {};

  for (let i = 0; i < topicNames.length; i++) {
    const field = topicNames[i];
    const converted = convertField(field, spec.fields[field], topics[i + 1]);
    if ("message" in converted) {
      return fail(contract, "invalid_field", converted.message, {
        eventName: name,
        field,
      });
    }
    fields[field] = converted.value;
  }

  let valueScVal: xdr.ScVal;
  try {
    valueScVal = xdr.ScVal.fromXDR(raw.value, "base64");
  } catch {
    return fail(contract, "malformed_value", "value is not valid base64 ScVal XDR", {
      eventName: name,
    });
  }

  const entries = new Map<string, xdr.ScVal>();
  const valueKind = scvKind(valueScVal);
  if (valueKind === "scvMap") {
    for (const entry of valueScVal.map() ?? []) {
      const key = entry.key();
      if (scvKind(key) !== "scvSymbol") {
        return fail(contract, "malformed_value", "value map contains a non-symbol key", {
          eventName: name,
        });
      }
      const keyName = String(scValToNative(key));
      if (entries.has(keyName)) {
        return fail(
          contract,
          "malformed_value",
          `value map contains duplicate key '${keyName}'`,
          { eventName: name },
        );
      }
      entries.set(keyName, entry.val());
    }
  } else if (!(valueKind === "scvVoid" && dataNames.length === 0)) {
    return fail(
      contract,
      "malformed_value",
      `value must be a map but found ${valueKind}`,
      { eventName: name },
    );
  }

  for (const field of dataNames) {
    const entry = entries.get(field);
    if (entry === undefined) {
      return fail(contract, "missing_field", `value is missing field '${field}'`, {
        eventName: name,
        field,
      });
    }
    const converted = convertField(field, spec.fields[field], entry);
    if ("message" in converted) {
      return fail(contract, "invalid_field", converted.message, {
        eventName: name,
        field,
      });
    }
    fields[field] = converted.value;
  }

  return { ok: true, event: { contract, event: name, fields } };
}

function decodeWithSpecs(
  contract: ForgeContractName,
  specs: ForgeEventSpecs,
  raw: RawContractEvent,
): EventDecodeResult<unknown> {
  try {
    return decodeUnchecked(contract, specs, raw);
  } catch (error) {
    return fail(
      contract,
      "malformed_value",
      `unexpected decoding failure: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
}

export function decodeEscrowEvent(raw: RawContractEvent): EventDecodeResult<EscrowContractEvent> {
  return decodeWithSpecs("escrow", ESCROW_EVENT_SPECS, raw) as EventDecodeResult<EscrowContractEvent>;
}
export function decodeVestingEvent(raw: RawContractEvent): EventDecodeResult<VestingContractEvent> {
  return decodeWithSpecs("vesting", VESTING_EVENT_SPECS, raw) as EventDecodeResult<VestingContractEvent>;
}
export function decodeMultisigEvent(raw: RawContractEvent): EventDecodeResult<MultisigContractEvent> {
  return decodeWithSpecs("multisig", MULTISIG_EVENT_SPECS, raw) as EventDecodeResult<MultisigContractEvent>;
}
export function decodeSubscriptionEvent(raw: RawContractEvent): EventDecodeResult<SubscriptionContractEvent> {
  return decodeWithSpecs("subscription", SUBSCRIPTION_EVENT_SPECS, raw) as EventDecodeResult<SubscriptionContractEvent>;
}
export function decodeMarketplaceEvent(raw: RawContractEvent): EventDecodeResult<MarketplaceContractEvent> {
  return decodeWithSpecs("marketplace", MARKETPLACE_EVENT_SPECS, raw) as EventDecodeResult<MarketplaceContractEvent>;
}
export function decodeDaoEvent(raw: RawContractEvent): EventDecodeResult<DaoContractEvent> {
  return decodeWithSpecs("dao", DAO_EVENT_SPECS, raw) as EventDecodeResult<DaoContractEvent>;
}

export function isForgeContractName(value: unknown): value is ForgeContractName {
  return (
    typeof value === "string" &&
    (FORGE_CONTRACT_NAMES as ReadonlyArray<string>).includes(value)
  );
}

/** Decodes a raw event with the decoder for the named contract. Never throws. */
export function decodeContractEvent(
  contract: ForgeContractName,
  raw: RawContractEvent,
): ForgeEventDecodeResult {
  if (!isForgeContractName(contract)) {
    return fail(
      String(contract),
      "invalid_input",
      `unknown contract '${String(contract)}'; expected one of ${FORGE_CONTRACT_NAMES.join(", ")}`,
    );
  }
  return decodeWithSpecs(
    contract,
    FORGE_EVENT_SPECS[contract],
    raw,
  ) as ForgeEventDecodeResult;
}

/** Maps a deployed contract ID to the Soroban Forge contract it runs. */
export type ContractDecoderMap = Readonly<Record<string, ForgeContractName>>;

export interface DecodedIndexedEvent {
  readonly record: ContractEventRecord;
  /** `null` when no decoder is registered for the event's contract ID. */
  readonly result: ForgeEventDecodeResult | null;
}

/** Decodes indexed records using the decoder registered for each contract ID. */
export function decodeIndexedEvents(
  records: ReadonlyArray<ContractEventRecord>,
  decoders: ContractDecoderMap = {},
): DecodedIndexedEvent[] {
  return records.map((record) => {
    const contract = Object.prototype.hasOwnProperty.call(decoders, record.contractId)
      ? decoders[record.contractId]
      : undefined;
    if (contract === undefined) {
      return { record, result: null };
    }
    return { record, result: decodeContractEvent(contract, record) };
  });
}
