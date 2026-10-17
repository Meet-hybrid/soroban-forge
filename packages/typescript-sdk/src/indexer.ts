import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { xdr } from "@stellar/stellar-sdk";

export interface IndexerCursor {
  ledger: number;
  eventIndex: number;
  pagingToken?: string;
}

export interface EventFilter {
  contractId?: string;
  eventType?: string;
  txHash?: string;
  entityId?: string;
  fromLedger?: number;
  toLedger?: number;
  after?: IndexerCursor;
  includeMalformed?: boolean;
}

export interface ContractEventRecord {
  type: "contract" | string;
  contractId: string;
  ledger: number;
  ledgerClosedAt: string;
  transactionHash: string;
  operationIndex: number;
  eventIndex: number;
  id: string;
  pagingToken: string;
  inSuccessfulContractCall: boolean;
  topic: string[];
  value: string;
  decodedTopic: unknown[];
  decodedValue: unknown;
  eventType: string;
  entityId?: string;
}

function asNumber(value: unknown, fallback = 0): number {
  const num = typeof value === "number" ? value : Number(value ?? fallback);
  if (!Number.isFinite(num)) {
    return fallback;
  }
  return num;
}

function decodeScVal(raw: string): unknown {
  if (!raw) {
    return undefined;
  }

  try {
    const value = xdr.ScVal.fromXDR(raw, "base64");
    return scValToJson(value);
  } catch {
    return raw;
  }
}

function scValToJson(value: unknown): unknown {
  if (value == null) {
    return value;
  }

  const scValType = xdr.ScValType as unknown as
    | Record<string, (() => unknown) | undefined>
    | undefined;
  const scValType = xdr.ScValType as unknown as Record<string, unknown> | undefined;
  const maybe = value as { switch?: () => unknown; value?: () => unknown };
  if (typeof maybe.switch === "function") {
    const kind = maybe.switch();

    const boolKind =
      typeof scValType?.scvBool === "function"
        ? scValType.scvBool()
        : undefined;
    if (kind === boolKind) {
      return Boolean((maybe as { value?: () => boolean }).value?.());
    }

    const ints = [
      typeof scValType?.scvI32 === "function" ? scValType.scvI32() : undefined,
      typeof scValType?.scvU32 === "function" ? scValType.scvU32() : undefined,
      typeof scValType?.scvI64 === "function" ? scValType.scvI64() : undefined,
      typeof scValType?.scvU64 === "function" ? scValType.scvU64() : undefined,
      typeof scValType?.scvI128 === "function"
        ? scValType.scvI128()
        : undefined,
      typeof scValType?.scvU128 === "function"
        ? scValType.scvU128()
        : undefined,
      typeof scValType?.scvI256 === "function"
        ? scValType.scvI256()
        : undefined,
      typeof scValType?.scvU256 === "function"
        ? scValType.scvU256()
        : undefined,
    ];
    if (ints.includes(kind)) {
      return String((maybe as { value?: () => unknown }).value?.());
    }

    const symbolKind =
      typeof scValType?.scvSymbol === "function"
        ? scValType.scvSymbol()
        : undefined;
    if (kind === symbolKind) {
      const sym = (maybe as { sym?: () => { toString: () => string } }).sym?.();
      return sym
        ? sym.toString()
        : String((maybe as { value?: () => unknown }).value?.());
    }

    const stringKind =
      typeof scValType?.scvString === "function"
        ? scValType.scvString()
        : undefined;
    if (kind === stringKind) {
      const s = (maybe as { str?: () => { toString: () => string } }).str?.();
      return s
        ? s.toString()
        : String((maybe as { value?: () => unknown }).value?.());
    }

    const addressKind =
      typeof scValType?.scvAddress === "function"
        ? scValType.scvAddress()
        : undefined;
    if (kind === addressKind) {
      const address = (
        maybe as { address?: () => { toString: () => string } }
      ).address?.();
      return address
        ? address.toString()
        : String((maybe as { value?: () => unknown }).value?.());
    }

    const vecKind =
      typeof scValType?.scvVec === "function" ? scValType.scvVec() : undefined;
    if (kind === vecKind) {
      const vec = (maybe as { vec?: () => unknown[] }).vec?.() ?? [];
      return Array.from(vec).map((entry) => scValToJson(entry));
    }

    const mapKind =
      typeof scValType?.scvMap === "function" ? scValType.scvMap() : undefined;
    if (kind === mapKind) {
      const map =
        (
          maybe as { map?: () => Array<{ key: unknown; val: unknown }> }
        ).map?.() ?? [];
      return Object.fromEntries(
        map.map((entry) => [
          scValToJson((entry as { key: unknown }).key),
          scValToJson((entry as { val: unknown }).val),
        ]),
      );
    }

    const bytesKind =
      typeof scValType?.scvBytes === "function"
        ? scValType.scvBytes()
        : undefined;
    if (kind === bytesKind) {
      const bytes = (maybe as { bytes?: () => Uint8Array }).bytes?.();
      return bytes ? Buffer.from(bytes).toString("hex") : undefined;
    }

    const voidKind =
      typeof scValType?.scvVoid === "function"
        ? scValType.scvVoid()
        : undefined;
    const errorKind =
      typeof scValType?.scvError === "function"
        ? scValType.scvError()
        : undefined;
    if (kind === voidKind || kind === errorKind) {
      return undefined;
    }
  }

  if (Array.isArray(value)) {
    return value.map((item) => scValToJson(item));
  }

  if (typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([key, nested]) => [
        key,
        scValToJson(nested),
      ]),
    );
  }

  return value;
}

function decodeEventTopic(raw: string): unknown {
  return decodeScVal(raw);
}

function decodeEventValue(raw: string): unknown {
  return decodeScVal(raw);
}

function deriveEntityId(
  rawEvent: Partial<ContractEventRecord>,
): string | undefined {
  const rawTopics = Array.isArray(rawEvent.topic) ? rawEvent.topic : [];
  if (rawTopics.length >= 2) {
    const id = decodeEventTopic(rawTopics[1]);
    if (typeof id === "string" && id.length > 0) {
      return id;
    }
  }
  const value = rawEvent.decodedValue;
  if (value && typeof value === "object") {
    const candidate =
      (value as Record<string, unknown>).escrow_id ??
      (value as Record<string, unknown>).subscription_id ??
      (value as Record<string, unknown>).proposal_id ??
      (value as Record<string, unknown>).id;
    if (candidate != null) {
      return String(candidate);
    }
  }
  return undefined;
}

export function deriveEventType(
  rawEvent: Partial<ContractEventRecord>,
): string {
  const rawTopics = Array.isArray(rawEvent.topic) ? rawEvent.topic : [];
  if (rawTopics.length > 0) {
    const name = decodeEventTopic(rawTopics[0]);
    if (typeof name === "string" && name.length > 0) {
      return name;
    }
  }

  if (rawEvent.type && typeof rawEvent.type === "string") {
    return rawEvent.type;
  }

  return "Unknown";
}

function normalizeEventRecord(raw: unknown): ContractEventRecord {
  const record = raw as Partial<ContractEventRecord> & Record<string, unknown>;
  const contractId = String(record.contractId ?? "unknown");
  const ledger = asNumber(record.ledger, 0);
  const eventIndex = asNumber(record.eventIndex, 0);
  const operationIndex = asNumber(record.operationIndex, 0);
  const pagingToken = String(record.pagingToken ?? `${ledger}-${eventIndex}`);
  const id = String(record.id ?? pagingToken);
  const topic = Array.isArray(record.topic)
    ? record.topic.map((entry) => String(entry))
    : [];
  const value = typeof record.value === "string" ? record.value : "";
  const decodedTopic = topic.map((entry) => decodeEventTopic(entry));
  const decodedValue = decodeEventValue(value);
  const eventType = deriveEventType({
    ...record,
    topic,
    value,
    decodedValue,
  });

  return {
    type: String(record.type ?? "contract"),
    contractId,
    ledger,
    ledgerClosedAt: String(record.ledgerClosedAt ?? "1970-01-01T00:00:00.000Z"),
    transactionHash: String(record.transactionHash ?? "unknown"),
    operationIndex,
    eventIndex,
    id,
    pagingToken,
    inSuccessfulContractCall: Boolean(record.inSuccessfulContractCall ?? true),
    topic,
    value,
    decodedTopic,
    decodedValue,
    eventType,
    entityId: deriveEntityId({
      ...record,
      topic,
      value,
      decodedValue,
    }),
  };
}

export function normalizeEvents(
  events: ReadonlyArray<unknown>,
): ContractEventRecord[] {
  const seen = new Set<string>();
  const normalized: ContractEventRecord[] = [];

  for (const event of events) {
    const record = normalizeEventRecord(event);
    if (seen.has(record.id)) {
      continue;
    }
    seen.add(record.id);
    normalized.push(record);
  }

  return normalized.sort((left, right) => {
    if (left.ledger !== right.ledger) {
      return left.ledger - right.ledger;
    }
    if (left.eventIndex !== right.eventIndex) {
      return left.eventIndex - right.eventIndex;
    }
    return left.pagingToken.localeCompare(right.pagingToken);
  });
}

export function filterEvents(
  events: ReadonlyArray<unknown>,
  filter: EventFilter = {},
): ContractEventRecord[] {
  const normalized = normalizeEvents(events);

  return normalized.filter((event) => {
    if (filter.contractId && event.contractId !== filter.contractId) {
      return false;
    }
    if (filter.eventType && event.eventType !== filter.eventType) {
      return false;
    }
    if (filter.txHash && event.transactionHash !== filter.txHash) {
      return false;
    }
    if (filter.entityId && event.entityId !== filter.entityId) {
      return false;
    }
    if (
      typeof filter.fromLedger === "number" &&
      event.ledger < filter.fromLedger
    ) {
      return false;
    }
    if (typeof filter.toLedger === "number" && event.ledger > filter.toLedger) {
      return false;
    }
    if (filter.after) {
      if (event.ledger < filter.after.ledger) {
        return false;
      }
      if (
        event.ledger === filter.after.ledger &&
        event.eventIndex <= filter.after.eventIndex
      ) {
        return false;
      }
    }
    return true;
  });
}

export function createIndexer(
  options: { events?: ReadonlyArray<unknown> } = {},
) {
  const items = normalizeEvents(options.events ?? []);

  return {
    events: items,
    filter(filter: EventFilter = {}) {
      return filterEvents(items, filter);
    },
    append(newEvents: ReadonlyArray<unknown>) {
      const merged = normalizeEvents([...items, ...newEvents]);
      return createIndexer({ events: merged });
    },
    cursor(): IndexerCursor | undefined {
      if (items.length === 0) {
        return undefined;
      }
      const last = items[items.length - 1];
      return {
        ledger: last.ledger,
        eventIndex: last.eventIndex,
        pagingToken: last.pagingToken,
      };
    },
  };
}

const FIXTURE_NAME_ALIASES: Record<string, string> = {
  escrow: "escrow-events.json",
  "escrow-events": "escrow-events.json",
  marketplace: "marketplace-royalties-events.json",
  "marketplace-royalties": "marketplace-royalties-events.json",
  "marketplace-royalties-events": "marketplace-royalties-events.json",
  multisig: "multi-sig-wallet-events.json",
  "multi-sig-wallet": "multi-sig-wallet-events.json",
  "multi-sig-wallet-events": "multi-sig-wallet-events.json",
  dao: "dao-governance-events.json",
  "dao-governance": "dao-governance-events.json",
  "dao-governance-events": "dao-governance-events.json",
  subscription: "subscription-payments-events.json",
  "subscription-payments": "subscription-payments-events.json",
  "subscription-payments-events": "subscription-payments-events.json",
  vesting: "vesting-events.json",
  "vesting-events": "vesting-events.json",
};

export function resolveFixturePath(nameOrPath?: string): string {
  const candidate = (nameOrPath ?? "escrow").trim();
  const alias = FIXTURE_NAME_ALIASES[candidate.toLowerCase()] ?? candidate;
  const root = fileURLToPath(
    new URL("../../../indexer/fixtures", import.meta.url),
  );
  const target = alias.endsWith(".json") ? alias : `${alias}.json`;
  const absolute = `${root}/${target}`;
  return absolute;
}

export function loadFixtureEvents(nameOrPath?: string): ContractEventRecord[] {
  const absolute = resolveFixturePath(nameOrPath);
  const raw = readFileSync(absolute, "utf8");
  const parsed = JSON.parse(raw) as { events?: unknown[] };
  if (!parsed || !Array.isArray(parsed.events)) {
    throw new Error(`Fixture '${absolute}' does not contain an events array.`);
  }
  return normalizeEvents(parsed.events);
}

export const readFixtureEvents = loadFixtureEvents;

export const indexer = {
  createIndexer,
  normalizeEvents,
  filterEvents,
  loadFixtureEvents,
  readFixtureEvents,
  deriveEventType,
};

export default indexer;
