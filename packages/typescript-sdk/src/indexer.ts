import {
  existsSync,
  mkdirSync,
  readFileSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { xdr } from "@stellar/stellar-sdk";

export const CHECKPOINT_SCHEMA_VERSION =
  "soroban-forge/indexer-checkpoint@1" as const;

export type CheckpointErrorCode =
  | "MALFORMED_CHECKPOINT"
  | "SCHEMA_VERSION_MISMATCH"
  | "NETWORK_MISMATCH"
  | "CONTRACT_SET_MISMATCH"
  | "INVALID_CURSOR";

export interface IndexerCursor {
  ledger: number;
  eventIndex: number;
  pagingToken?: string;
}

export interface IndexerCheckpoint {
  schemaVersion: string;
  network: string;
  contracts: string[];
  cursor: IndexerCursor | null;
  updatedAt: string;
}

export interface IndexerRuntimeConfig {
  network: string;
  contracts: ReadonlyArray<string>;
  schemaVersion?: string;
}

export interface CheckpointStore {
  read(): string | null | undefined;
  write(serialized: string): void;
  clear(): void;
}

export class CheckpointValidationError extends Error {
  readonly code: CheckpointErrorCode;
  readonly remediation: string;
  readonly details?: Record<string, unknown>;

  constructor(
    code: CheckpointErrorCode,
    message: string,
    remediation: string,
    details?: Record<string, unknown>,
  ) {
    super(`[${code}] ${message} Remediation: ${remediation}`);
    this.name = "CheckpointValidationError";
    this.code = code;
    this.remediation = remediation;
    this.details = details;
  }
}

export interface CheckpointValidationResult {
  valid: boolean;
  checkpoint?: IndexerCheckpoint;
  error?: CheckpointValidationError;
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

export function normalizeContractSet(
  contracts: ReadonlyArray<string>,
): string[] {
  const unique = new Set<string>();
  for (const entry of contracts) {
    if (typeof entry !== "string") {
      continue;
    }
    const trimmed = entry.trim();
    if (trimmed.length > 0) {
      unique.add(trimmed);
    }
  }
  return Array.from(unique).sort((a, b) => a.localeCompare(b));
}

function normalizeNetworkIdentity(network: unknown): string {
  if (typeof network !== "string") {
    return "";
  }
  return network.trim();
}

function validateCursorShape(rawCursor: unknown): IndexerCursor | null {
  if (rawCursor == null) {
    return null;
  }
  if (typeof rawCursor !== "object" || Array.isArray(rawCursor)) {
    throw new CheckpointValidationError(
      "INVALID_CURSOR",
      "Checkpoint cursor must be an object with non-negative integer `ledger` and `eventIndex` fields, or `null`.",
      "Call `discardCheckpoint(store)` or `rebuildCheckpoint(store, config, events)` to repair the persisted checkpoint.",
    );
  }
  const candidate = rawCursor as Record<string, unknown>;
  const ledger = candidate.ledger;
  const eventIndex = candidate.eventIndex;
  if (
    typeof ledger !== "number" ||
    !Number.isInteger(ledger) ||
    ledger < 0 ||
    typeof eventIndex !== "number" ||
    !Number.isInteger(eventIndex) ||
    eventIndex < 0
  ) {
    throw new CheckpointValidationError(
      "INVALID_CURSOR",
      "Checkpoint cursor contains invalid `ledger` or `eventIndex` coordinates.",
      "Call `discardCheckpoint(store)` or `rebuildCheckpoint(store, config, events)` to reset the cursor.",
      { cursor: rawCursor },
    );
  }

  const pagingToken =
    candidate.pagingToken === undefined
      ? undefined
      : typeof candidate.pagingToken === "string" &&
          candidate.pagingToken.trim().length > 0
        ? candidate.pagingToken.trim()
        : undefined;

  if (
    candidate.pagingToken !== undefined &&
    typeof candidate.pagingToken !== "string"
  ) {
    throw new CheckpointValidationError(
      "INVALID_CURSOR",
      "Checkpoint cursor `pagingToken` must be a string when present.",
      "Call `discardCheckpoint(store)` or `rebuildCheckpoint(store, config, events)` to reset the cursor.",
    );
  }

  return pagingToken !== undefined
    ? { ledger, eventIndex, pagingToken }
    : { ledger, eventIndex };
}

export function parseCheckpoint(raw: unknown): IndexerCheckpoint {
  let decoded: unknown = raw;
  if (typeof raw === "string") {
    const trimmed = raw.trim();
    if (!trimmed) {
      throw new CheckpointValidationError(
        "MALFORMED_CHECKPOINT",
        "Persisted checkpoint payload is empty.",
        "Call `discardCheckpoint(store)` or `rebuildCheckpoint(store, config, events)` before restarting the indexer.",
      );
    }
    try {
      decoded = JSON.parse(trimmed);
    } catch (error) {
      throw new CheckpointValidationError(
        "MALFORMED_CHECKPOINT",
        `Persisted checkpoint is not valid JSON (${error instanceof Error ? error.message : String(error)}).`,
        "Call `discardCheckpoint(store)` or `rebuildCheckpoint(store, config, events)` to replace the corrupted file.",
      );
    }
  }

  if (!decoded || typeof decoded !== "object" || Array.isArray(decoded)) {
    throw new CheckpointValidationError(
      "MALFORMED_CHECKPOINT",
      "Persisted checkpoint must be a JSON object.",
      "Call `discardCheckpoint(store)` or `rebuildCheckpoint(store, config, events)` to replace the corrupted state.",
    );
  }

  const record = decoded as Record<string, unknown>;
  if (
    typeof record.schemaVersion !== "string" ||
    record.schemaVersion.trim().length === 0
  ) {
    throw new CheckpointValidationError(
      "MALFORMED_CHECKPOINT",
      "Checkpoint is missing required string field `schemaVersion`.",
      "Call `discardCheckpoint(store)` or `rebuildCheckpoint(store, config, events)` to recreate the checkpoint.",
    );
  }

  const network = normalizeNetworkIdentity(record.network);
  if (!network) {
    throw new CheckpointValidationError(
      "MALFORMED_CHECKPOINT",
      "Checkpoint is missing required non-empty `network` identity.",
      "Call `discardCheckpoint(store)` or `rebuildCheckpoint(store, config, events)` to recreate the checkpoint.",
    );
  }

  if (
    !Array.isArray(record.contracts) ||
    record.contracts.length === 0 ||
    record.contracts.some(
      (item) => typeof item !== "string" || item.trim().length === 0,
    )
  ) {
    throw new CheckpointValidationError(
      "MALFORMED_CHECKPOINT",
      "Checkpoint is missing a non-empty `contracts` array of contract identifiers.",
      "Call `discardCheckpoint(store)` or `rebuildCheckpoint(store, config, events)` to recreate the checkpoint.",
    );
  }

  const contracts = normalizeContractSet(record.contracts as string[]);
  const cursor = validateCursorShape(record.cursor ?? null);
  const updatedAt =
    typeof record.updatedAt === "string" && record.updatedAt.trim().length > 0
      ? record.updatedAt.trim()
      : "1970-01-01T00:00:00.000Z";

  return {
    schemaVersion: record.schemaVersion.trim(),
    network,
    contracts,
    cursor,
    updatedAt,
  };
}

export function validateCheckpoint(
  raw: unknown,
  expected: IndexerRuntimeConfig,
): IndexerCheckpoint {
  const checkpoint = parseCheckpoint(raw);
  const expectedSchema = expected.schemaVersion ?? CHECKPOINT_SCHEMA_VERSION;
  const expectedNetwork = normalizeNetworkIdentity(expected.network);
  const expectedContracts = normalizeContractSet(expected.contracts);

  if (!expectedNetwork) {
    throw new CheckpointValidationError(
      "MALFORMED_CHECKPOINT",
      "Indexer runtime configuration must specify a non-empty `network` identity.",
      "Provide `network` (for example `testnet`) in the indexer configuration.",
    );
  }

  if (expectedContracts.length === 0) {
    throw new CheckpointValidationError(
      "MALFORMED_CHECKPOINT",
      "Indexer runtime configuration must specify at least one contract in `contracts`.",
      "Provide a non-empty `contracts` list in the indexer configuration.",
    );
  }

  if (checkpoint.schemaVersion !== expectedSchema) {
    throw new CheckpointValidationError(
      "SCHEMA_VERSION_MISMATCH",
      `Checkpoint schema version '${checkpoint.schemaVersion}' is incompatible with expected '${expectedSchema}'.`,
      "Run `rebuildCheckpoint(store, config, events)` or `discardCheckpoint(store)` to migrate to the current checkpoint schema.",
      {
        checkpointSchemaVersion: checkpoint.schemaVersion,
        expectedSchemaVersion: expectedSchema,
      },
    );
  }

  if (checkpoint.network !== expectedNetwork) {
    throw new CheckpointValidationError(
      "NETWORK_MISMATCH",
      `Checkpoint was created for network '${checkpoint.network}', but indexer is configured for '${expectedNetwork}'.`,
      "Point the indexer back to the matching network, or explicitly run `discardCheckpoint(store)` / `rebuildCheckpoint(store, config, events)` to re-index on the new network.",
      {
        checkpointNetwork: checkpoint.network,
        expectedNetwork,
      },
    );
  }

  const sameContractSet =
    checkpoint.contracts.length === expectedContracts.length &&
    checkpoint.contracts.every(
      (contractId, idx) => contractId === expectedContracts[idx],
    );

  if (!sameContractSet) {
    throw new CheckpointValidationError(
      "CONTRACT_SET_MISMATCH",
      `Checkpoint contract set [${checkpoint.contracts.join(", ")}] does not match configured contracts [${expectedContracts.join(", ")}].`,
      "Explicitly run `rebuildCheckpoint(store, config, events)` or `discardCheckpoint(store)` so added or removed contracts are replayed from a clean boundary.",
      {
        checkpointContracts: checkpoint.contracts,
        expectedContracts,
      },
    );
  }

  return checkpoint;
}

export function checkCheckpointCompatibility(
  raw: unknown,
  expected: IndexerRuntimeConfig,
): CheckpointValidationResult {
  try {
    const checkpoint = validateCheckpoint(raw, expected);
    return { valid: true, checkpoint };
  } catch (error) {
    if (error instanceof CheckpointValidationError) {
      return { valid: false, error };
    }
    const wrapped = new CheckpointValidationError(
      "MALFORMED_CHECKPOINT",
      error instanceof Error ? error.message : String(error),
      "Call `discardCheckpoint(store)` or `rebuildCheckpoint(store, config, events)` to reset the checkpoint.",
    );
    return { valid: false, error: wrapped };
  }
}

export function createCheckpoint(
  config: IndexerRuntimeConfig,
  cursor: IndexerCursor | null = null,
  updatedAt = new Date().toISOString(),
): IndexerCheckpoint {
  const network = normalizeNetworkIdentity(config.network);
  const contracts = normalizeContractSet(config.contracts);
  const schemaVersion = config.schemaVersion ?? CHECKPOINT_SCHEMA_VERSION;
  const validatedCursor = validateCursorShape(cursor);

  return validateCheckpoint(
    {
      schemaVersion,
      network,
      contracts,
      cursor: validatedCursor,
      updatedAt,
    },
    config,
  );
}

export function serializeCheckpoint(checkpoint: IndexerCheckpoint): string {
  const normalized: IndexerCheckpoint = {
    schemaVersion: checkpoint.schemaVersion,
    network: normalizeNetworkIdentity(checkpoint.network),
    contracts: normalizeContractSet(checkpoint.contracts),
    cursor: validateCursorShape(checkpoint.cursor),
    updatedAt: checkpoint.updatedAt,
  };
  return `${JSON.stringify(normalized, null, 2)}\n`;
}

export function createMemoryCheckpointStore(
  initialSerialized: string | null = null,
): CheckpointStore {
  let state = initialSerialized;
  return {
    read() {
      return state;
    },
    write(serialized: string) {
      state = serialized;
    },
    clear() {
      state = null;
    },
  };
}

export function createFileCheckpointStore(filePath: string): CheckpointStore {
  return {
    read() {
      if (!existsSync(filePath)) {
        return null;
      }
      return readFileSync(filePath, "utf8");
    },
    write(serialized: string) {
      mkdirSync(dirname(filePath), { recursive: true });
      writeFileSync(filePath, serialized, "utf8");
    },
    clear() {
      if (existsSync(filePath)) {
        unlinkSync(filePath);
      }
    },
  };
}

function resolveStore(target: string | CheckpointStore): CheckpointStore {
  return typeof target === "string"
    ? createFileCheckpointStore(target)
    : target;
}

export function saveCheckpoint(
  target: string | CheckpointStore,
  checkpoint: IndexerCheckpoint,
): void {
  const store = resolveStore(target);
  store.write(serializeCheckpoint(checkpoint));
}

export function loadAndValidateCheckpoint(
  target: string | CheckpointStore,
  expected: IndexerRuntimeConfig,
): IndexerCheckpoint | undefined {
  const store = resolveStore(target);
  const raw = store.read();
  if (raw == null) {
    return undefined;
  }
  return validateCheckpoint(raw, expected);
}

export function discardCheckpoint(target: string | CheckpointStore): void {
  const store = resolveStore(target);
  store.clear();
}

export function rebuildCheckpoint(
  target: string | CheckpointStore,
  config: IndexerRuntimeConfig,
  events: ReadonlyArray<unknown> = [],
  updatedAt = new Date().toISOString(),
): IndexerCheckpoint {
  const store = resolveStore(target);
  store.clear();

  const contracts = normalizeContractSet(config.contracts);
  const contractSet = new Set(contracts);
  const matchingEvents = normalizeEvents(events).filter((event) =>
    contractSet.has(event.contractId),
  );
  const lastEvent =
    matchingEvents.length > 0
      ? matchingEvents[matchingEvents.length - 1]
      : undefined;
  const cursor: IndexerCursor | null = lastEvent
    ? {
        ledger: lastEvent.ledger,
        eventIndex: lastEvent.eventIndex,
        pagingToken: lastEvent.pagingToken,
      }
    : null;

  const rebuilt = createCheckpoint(config, cursor, updatedAt);
  store.write(serializeCheckpoint(rebuilt));
  return rebuilt;
}

export function createIndexer(
  options: {
    events?: ReadonlyArray<unknown>;
    runtime?: IndexerRuntimeConfig;
    checkpoint?: unknown;
    checkpointStore?: string | CheckpointStore;
  } = {},
) {
  let activeCheckpoint: IndexerCheckpoint | undefined;
  if (options.runtime) {
    if (options.checkpoint !== undefined) {
      activeCheckpoint = validateCheckpoint(
        options.checkpoint,
        options.runtime,
      );
    } else if (options.checkpointStore !== undefined) {
      activeCheckpoint = loadAndValidateCheckpoint(
        options.checkpointStore,
        options.runtime,
      );
    }
  } else if (options.checkpoint !== undefined) {
    activeCheckpoint = parseCheckpoint(options.checkpoint);
  }

  const allowedContracts = options.runtime
    ? new Set(normalizeContractSet(options.runtime.contracts))
    : undefined;

  const initialEvents = normalizeEvents(options.events ?? []).filter((event) =>
    allowedContracts ? allowedContracts.has(event.contractId) : true,
  );

  const items = activeCheckpoint?.cursor
    ? filterEvents(initialEvents, { after: activeCheckpoint.cursor })
    : initialEvents;

  const computeCurrentCursor = (): IndexerCursor | undefined => {
    if (items.length > 0) {
      const last = items[items.length - 1];
      return {
        ledger: last.ledger,
        eventIndex: last.eventIndex,
        pagingToken: last.pagingToken,
      };
    }
    return activeCheckpoint?.cursor ?? undefined;
  };

  return {
    events: items,
    filter(filter: EventFilter = {}) {
      return filterEvents(items, filter);
    },
    append(newEvents: ReadonlyArray<unknown>) {
      const candidateEvents = normalizeEvents(newEvents).filter((event) =>
        allowedContracts ? allowedContracts.has(event.contractId) : true,
      );
      const afterCursor = computeCurrentCursor();
      const unseen = afterCursor
        ? filterEvents(candidateEvents, { after: afterCursor })
        : candidateEvents;
      const merged = normalizeEvents([...items, ...unseen]);
      const nextCursor =
        merged.length > 0
          ? {
              ledger: merged[merged.length - 1]!.ledger,
              eventIndex: merged[merged.length - 1]!.eventIndex,
              pagingToken: merged[merged.length - 1]!.pagingToken,
            }
          : (afterCursor ?? null);

      let nextCheckpoint = activeCheckpoint;
      if (options.runtime) {
        nextCheckpoint = createCheckpoint(options.runtime, nextCursor);
        if (options.checkpointStore !== undefined) {
          saveCheckpoint(options.checkpointStore, nextCheckpoint);
        }
      }

      return createIndexer({
        events: merged,
        runtime: options.runtime,
        checkpoint: nextCheckpoint ? { ...nextCheckpoint, cursor: null } : undefined,
        checkpointStore: options.checkpointStore,
      });
    },
    cursor(): IndexerCursor | undefined {
      return computeCurrentCursor();
    },
    checkpoint(updatedAt = new Date().toISOString()): IndexerCheckpoint {
      if (!options.runtime) {
        throw new CheckpointValidationError(
          "MALFORMED_CHECKPOINT",
          "Cannot create a checkpoint without `runtime` configuration (`network` and `contracts`).",
          "Pass `runtime: { network, contracts }` when calling `createIndexer(...)`.",
        );
      }
      const next = createCheckpoint(
        options.runtime,
        computeCurrentCursor() ?? null,
        updatedAt,
      );
      if (options.checkpointStore !== undefined) {
        saveCheckpoint(options.checkpointStore, next);
      }
      return next;
    },
  };
}

export function resumeIndexerFromCheckpoint(options: {
  store: string | CheckpointStore;
  runtime: IndexerRuntimeConfig;
  existingEvents?: ReadonlyArray<unknown>;
  incomingEvents?: ReadonlyArray<unknown>;
  updatedAt?: string;
}) {
  const validated = loadAndValidateCheckpoint(options.store, options.runtime);
  const contracts = normalizeContractSet(options.runtime.contracts);
  const contractSet = new Set(contracts);

  const baseEvents = normalizeEvents(options.existingEvents ?? []).filter(
    (event) => contractSet.has(event.contractId),
  );
  const candidateIncoming = normalizeEvents(
    options.incomingEvents ?? [],
  ).filter((event) => contractSet.has(event.contractId));

  const newAfterCursor = validated?.cursor
    ? filterEvents(candidateIncoming, { after: validated.cursor })
    : candidateIncoming;

  const merged = normalizeEvents([...baseEvents, ...newAfterCursor]);
  const latest = merged.length > 0 ? merged[merged.length - 1] : undefined;
  const nextCursor: IndexerCursor | null = latest
    ? {
        ledger: latest.ledger,
        eventIndex: latest.eventIndex,
        pagingToken: latest.pagingToken,
      }
    : (validated?.cursor ?? null);

  const updatedCheckpoint = createCheckpoint(
    options.runtime,
    nextCursor,
    options.updatedAt,
  );
  saveCheckpoint(options.store, updatedCheckpoint);

  return {
    checkpoint: updatedCheckpoint,
    resumedFromCursor: validated?.cursor ?? null,
    appendedEvents: newAfterCursor,
    events: merged,
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
  CHECKPOINT_SCHEMA_VERSION,
  CheckpointValidationError,
  createIndexer,
  normalizeEvents,
  filterEvents,
  loadFixtureEvents,
  readFixtureEvents,
  deriveEventType,
  normalizeContractSet,
  parseCheckpoint,
  validateCheckpoint,
  checkCheckpointCompatibility,
  createCheckpoint,
  serializeCheckpoint,
  createMemoryCheckpointStore,
  createFileCheckpointStore,
  saveCheckpoint,
  loadAndValidateCheckpoint,
  discardCheckpoint,
  rebuildCheckpoint,
  resumeIndexerFromCheckpoint,
};

export default indexer;
