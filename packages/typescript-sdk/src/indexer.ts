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

export const INDEXER_CHECKPOINT_SCHEMA_VERSION = 1;

export interface IndexerCheckpoint {
  schemaVersion: number;
  network: string;
  contractIds: string[];
  cursor: IndexerCursor;
}

export interface IndexerIdentity {
  network: string;
  contractIds: ReadonlyArray<string>;
}

function checkpointError(reason: string): Error {
  return new Error(
    `Invalid indexer checkpoint: ${reason}. Refusing to resume. Deliberately discard the checkpoint and rebuild from the required starting ledger, or restore a compatible checkpoint.`,
  );
}

function canonicalContractIds(contractIds: ReadonlyArray<unknown>): string[] {
  if (!Array.isArray(contractIds) || contractIds.length === 0) {
    throw checkpointError("contractIds must contain at least one contract ID");
  }
  const normalized: string[] = [];
  for (const contractId of contractIds) {
    if (typeof contractId !== "string" || contractId.trim().length === 0) {
      throw checkpointError(
        "contractIds must contain only non-empty contract IDs",
      );
    }
    normalized.push(contractId.trim());
  }
  return Array.from(new Set(normalized)).sort();
}

function validateCheckpoint(
  value: unknown,
  identity: IndexerIdentity,
): IndexerCheckpoint {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw checkpointError("checkpoint must be a JSON object");
  }

  const checkpoint = value as Record<string, unknown>;
  if (checkpoint.schemaVersion !== INDEXER_CHECKPOINT_SCHEMA_VERSION) {
    throw checkpointError(
      `schema version ${String(checkpoint.schemaVersion)} is unsupported; expected ${INDEXER_CHECKPOINT_SCHEMA_VERSION}`,
    );
  }

  if (
    typeof identity.network !== "string" ||
    identity.network.trim().length === 0
  ) {
    throw checkpointError("the configured network identity is empty");
  }
  const expectedContractIds = canonicalContractIds(identity.contractIds);

  if (typeof checkpoint.network !== "string" || checkpoint.network.length === 0) {
    throw checkpointError("network identity is missing or malformed");
  }
  if (checkpoint.network !== identity.network) {
    throw checkpointError(
      `network mismatch (checkpoint: '${checkpoint.network}', configured: '${identity.network}')`,
    );
  }
  if (!Array.isArray(checkpoint.contractIds)) {
    throw checkpointError("contractIds are missing or malformed");
  }
  const storedContractIds = canonicalContractIds(checkpoint.contractIds);
  if (
    storedContractIds.length !== expectedContractIds.length ||
    storedContractIds.some(
      (contractId, index) => contractId !== expectedContractIds[index],
    )
  ) {
    throw checkpointError(
      `contract set mismatch (checkpoint: [${storedContractIds.join(", ")}], configured: [${expectedContractIds.join(", ")}])`,
    );
  }

  const cursor = checkpoint.cursor;
  if (typeof cursor !== "object" || cursor === null || Array.isArray(cursor)) {
    throw checkpointError("cursor is missing or malformed");
  }
  const storedCursor = cursor as Record<string, unknown>;
  if (
    !Number.isSafeInteger(storedCursor.ledger) ||
    (storedCursor.ledger as number) < 0 ||
    !Number.isSafeInteger(storedCursor.eventIndex) ||
    (storedCursor.eventIndex as number) < 0 ||
    (storedCursor.pagingToken !== undefined &&
      (typeof storedCursor.pagingToken !== "string" ||
        storedCursor.pagingToken.length === 0))
  ) {
    throw checkpointError(
      "cursor must contain valid non-negative ledger and eventIndex values",
    );
  }

  return {
    schemaVersion: INDEXER_CHECKPOINT_SCHEMA_VERSION,
    network: identity.network,
    contractIds: expectedContractIds,
    cursor: {
      ledger: storedCursor.ledger as number,
      eventIndex: storedCursor.eventIndex as number,
      ...(storedCursor.pagingToken === undefined
        ? {}
        : { pagingToken: storedCursor.pagingToken as string }),
    },
  };
}

export function serializeIndexerCheckpoint(
  checkpoint: IndexerCheckpoint,
): string {
  if (typeof checkpoint !== "object" || checkpoint === null) {
    throw checkpointError("checkpoint must be a JSON object");
  }
  const value = checkpoint as unknown as Record<string, unknown>;
  if (typeof value.network !== "string") {
    throw checkpointError("network identity is missing or malformed");
  }
  const validated = validateCheckpoint(checkpoint, {
    network: value.network,
    contractIds: canonicalContractIds(
      Array.isArray(value.contractIds) ? value.contractIds : [],
    ),
  });
  return JSON.stringify(validated);
}

export function parseIndexerCheckpoint(
  serialized: string,
  identity: IndexerIdentity,
): IndexerCheckpoint {
  let parsed: unknown;
  try {
    parsed = JSON.parse(serialized);
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw checkpointError(`checkpoint JSON could not be parsed (${detail})`);
  }
  return validateCheckpoint(parsed, identity);
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
  if (raw == null) {
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

  const scValType = xdr.ScValType as unknown as Record<string, unknown> | undefined;
  if (!scValType) {
    return value;
  }

  const maybe = value as { switch?: () => unknown; value?: () => unknown };
  if (typeof maybe.switch !== "function") {
    return value;
  }

  const kind = maybe.switch();

  const resolveInt = (): string | undefined => {
    const raw = (maybe as { value?: () => unknown }).value?.();
    if (raw == null) {
      return undefined;
    }
    if (typeof raw === "bigint") {
      return String(raw);
    }
    if (typeof raw === "number" && Number.isFinite(raw)) {
      return String(raw);
    }
    const num = Number(raw);
    return Number.isFinite(num) ? String(num) : undefined;
  };

  const boolKind =
    typeof scValType?.scvBool === "function" ? scValType.scvBool() : undefined;
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
    return resolveInt() ?? String((maybe as { value?: () => unknown }).value?.());
  }

  const timepointKind =
    typeof scValType?.scvTimepoint === "function"
      ? scValType.scvTimepoint()
      : undefined;
  if (kind === timepointKind) {
    return resolveInt() ?? String((maybe as { value?: () => unknown }).value?.());
  }

  const durationKind =
    typeof scValType?.scvDuration === "function"
      ? scValType.scvDuration()
      : undefined;
  if (kind === durationKind) {
    return resolveInt() ?? String((maybe as { value?: () => unknown }).value?.());
  }

  const symbolKind =
    typeof scValType?.scvSymbol === "function"
      ? scValType.scvSymbol()
      : undefined;
  if (kind === symbolKind) {
    const sym = (
      maybe as { sym?: () => { toString: () => string } }
    ).sym?.();
    return sym
      ? sym.toString()
      : String((maybe as { value?: () => unknown }).value?.());
  }

  const stringKind =
    typeof scValType?.scvString === "function"
      ? scValType.scvString()
      : undefined;
  if (kind === stringKind) {
    const s = (
      maybe as { str?: () => { toString: () => string } }
    ).str?.();
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

  const contractInstanceKind =
    typeof scValType?.scvContractInstance === "function"
      ? scValType.scvContractInstance()
      : undefined;
  if (kind === contractInstanceKind) {
    const instance = (
      maybe as { value?: () => { contractId?: string } }
    ).value?.();
    if (!instance || typeof instance !== "object") {
      return String(instance ?? "");
    }
    return instance.contractId ?? String(instance);
  }

  const ledgerKeyContractInstanceKind =
    typeof scValType?.scvLedgerKeyContractInstance === "function"
      ? scValType.scvLedgerKeyContractInstance()
      : undefined;
  if (kind === ledgerKeyContractInstanceKind) {
    const key = (
      maybe as { value?: () => { contractId?: string } }
    ).value?.();
    if (!key || typeof key !== "object") {
      return String(key ?? "");
    }
    return key.contractId ?? String(key);
  }

  const executableTagKind =
    typeof scValType?.scvExecutableTag === "function"
      ? scValType.scvExecutableTag()
      : undefined;
  if (kind === executableTagKind) {
    return String(
      (maybe as { value?: () => unknown }).value?.() ?? "",
    );
  }

  const objectKind =
    typeof scValType?.scvObject === "function" ? scValType.scvObject() : undefined;
  if (kind === objectKind) {
    const object = (
      maybe as { value?: () => Record<string, unknown> }
    ).value?.();
    if (!object || typeof object !== "object") {
      return undefined;
    }
    return Object.fromEntries(
      Object.entries(object).map(([key, nested]) => [
        key,
        scValToJson(nested),
      ]),
    );
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
  if (value && typeof value === "object"
) {
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
  options: {
    events?: ReadonlyArray<unknown>;
    network?: string;
    contractIds?: ReadonlyArray<string>;
    checkpoint?: IndexerCheckpoint | string;
  } = {},
) {
  const items = normalizeEvents(options.events ?? []);
  const identity =
    options.network === undefined || options.contractIds === undefined
      ? undefined
      : {
          network: options.network,
          contractIds: canonicalContractIds(options.contractIds),
        };
  let resumeCursor: IndexerCursor | undefined;

  if (options.checkpoint !== undefined) {
    if (!identity) {
      throw checkpointError(
        "network and contractIds must be configured before loading a checkpoint",
      );
    }
    const checkpoint =
      typeof options.checkpoint === "string"
        ? parseIndexerCheckpoint(options.checkpoint, identity)
        : validateCheckpoint(options.checkpoint, identity);
    resumeCursor = checkpoint.cursor;
  }

  return {
    events: items,
    filter(filter: EventFilter = {}) {
      return filterEvents(items, filter);
    },
    append(newEvents: ReadonlyArray<unknown>) {
      const currentCursor =
        items.length === 0 ? resumeCursor : this.cursor();
      const newItems = currentCursor
        ? filterEvents(newEvents, { after: currentCursor })
        : normalizeEvents(newEvents);
      const merged = normalizeEvents([...items, ...newItems]);
      return createIndexer({
        events: merged,
        ...(identity
          ? {
              network: identity.network,
              contractIds: identity.contractIds,
              ...(currentCursor
                ? {
                    checkpoint: {
                      schemaVersion: INDEXER_CHECKPOINT_SCHEMA_VERSION,
                      network: identity.network,
                      contractIds: identity.contractIds,
                      cursor: currentCursor,
                    },
                  }
                : {}),
            }
          : {}),
      });
    },
    cursor(): IndexerCursor | undefined {
      if (items.length === 0) {
        return resumeCursor;
      }
      const last = items[items.length - 1];
      const eventCursor = {
        ledger: last.ledger,
        eventIndex: last.eventIndex,
        pagingToken: last.pagingToken,
      };
      if (
        !resumeCursor ||
        eventCursor.ledger > resumeCursor.ledger ||
        (eventCursor.ledger === resumeCursor.ledger &&
          eventCursor.eventIndex >= resumeCursor.eventIndex)
      ) {
        return eventCursor;
      }
      return resumeCursor;
    },
    checkpoint(): IndexerCheckpoint | undefined {
      const cursor = this.cursor();
      if (!cursor) {
        return undefined;
      }
      if (!identity) {
        throw checkpointError(
          "network and contractIds must be configured before saving a checkpoint",
        );
      }
      return {
        schemaVersion: INDEXER_CHECKPOINT_SCHEMA_VERSION,
        network: identity.network,
        contractIds: identity.contractIds,
        cursor,
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

function getNodeModule<T = unknown>(name: string): T {
  const proc =
    typeof globalThis !== "undefined"
      ? (globalThis as { process?: { getBuiltinModule?: (m: string) => unknown } }).process
      : undefined;
  if (proc?.getBuiltinModule) {
    return proc.getBuiltinModule(name) as T;
  }
  try {
    const req = (globalThis as unknown as { require?: (m: string) => unknown }).require;
    if (typeof req === "function") {
      return req(name) as T;
    }
  } catch {
    // fallback if require fails
  }
  throw new Error(`Cannot load module '${name}' in this environment.`);
}

export function resolveFixturePath(nameOrPath?: string): string {
  const path = getNodeModule<{
    resolve: (...paths: string[]) => string;
    dirname: (p: string) => string;
  }>("node:path");
  const url = getNodeModule<{
    fileURLToPath: (url: string | URL) => string;
  }>("node:url");

  const candidate = (nameOrPath ?? "escrow").trim();
  const alias = FIXTURE_NAME_ALIASES[candidate.toLowerCase()] ?? candidate;
  const root = path.resolve(
    path.dirname(url.fileURLToPath(import.meta.url)),
    "../../../indexer/fixtures",
  );
  const target = alias.endsWith(".json") ? alias : `${alias}.json`;
  const absolute = `${root}/${target}`;
  return absolute;
}

export function loadFixtureEvents(nameOrPath?: string): ContractEventRecord[] {
  const fs = getNodeModule<{
    readFileSync: (path: string, encoding: string) => string;
  }>("node:fs");
  const absolute = resolveFixturePath(nameOrPath);
  const raw = fs.readFileSync(absolute, "utf8");
  const parsed = JSON.parse(raw) as { events?: unknown[] };
  if (!parsed || !Array.isArray(parsed.events)) {
    throw new Error(`Fixture '${absolute}' does not contain an events array.`);
  }
  return normalizeEvents(parsed.events);
}

export const readFixtureEvents = loadFixtureEvents;

export const indexer = {
  createIndexer,
  parseIndexerCheckpoint,
  serializeIndexerCheckpoint,
  normalizeEvents,
  filterEvents,
  loadFixtureEvents,
  readFixtureEvents,
  deriveEventType,
};

export default indexer;
