export const DEMO_STATE_STORAGE_KEY = "soroban-forge:demo-state:v1" as const;
export const DEMO_STATE_SCHEMA_VERSION = "soroban-forge/demo-state@1" as const;
export const DEMO_STATE_LABEL = "LOCAL_DEMO_STATE_ONLY" as const;

export const DEFAULT_DEMO_TOKEN =
  "CBJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O";

export type DemoStateValidationErrorCode =
  | "MALFORMED_DATA"
  | "VERSION_MISMATCH"
  | "SENSITIVE_DATA_REJECTED"
  | "INVALID_STATE_SHAPE"
  | "CONFIRMATION_REQUIRED";

export interface DemoEscrowDraft {
  buyer: string;
  seller: string;
  arbiter: string;
  token: string;
  amount: string;
  timeoutSecs: string;
}

export interface DemoLogEntry {
  id: number;
  time: string;
  text: string;
  kind: "info" | "ok" | "err";
}

export interface DemoStatePayload {
  draft: DemoEscrowDraft;
  createdId: string | null;
  actionEscrowId: string;
  readEscrowId: string;
  resolveFavor: "seller" | "buyer";
  log: DemoLogEntry[];
}

export interface VersionedDemoStateDocument {
  schemaVersion: string;
  scope: typeof DEMO_STATE_LABEL;
  exportedAt: string;
  state: DemoStatePayload;
}

export interface BrowserStorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export class DemoStateValidationError extends Error {
  readonly code: DemoStateValidationErrorCode;

  constructor(code: DemoStateValidationErrorCode, message: string) {
    super(`[${code}] ${message}`);
    this.name = "DemoStateValidationError";
    this.code = code;
  }
}

export function createEmptyDemoState(
  defaultToken: string = DEFAULT_DEMO_TOKEN,
): DemoStatePayload {
  return {
    draft: {
      buyer: "",
      seller: "",
      arbiter: "",
      token: defaultToken,
      amount: "10000000",
      timeoutSecs: "3600",
    },
    createdId: null,
    actionEscrowId: "",
    readEscrowId: "",
    resolveFavor: "seller",
    log: [],
  };
}

export function isDemoStateEmpty(
  state: DemoStatePayload,
  defaultToken: string = DEFAULT_DEMO_TOKEN,
): boolean {
  const baseline = createEmptyDemoState(defaultToken);
  return (
    state.draft.buyer === baseline.draft.buyer &&
    state.draft.seller === baseline.draft.seller &&
    state.draft.arbiter === baseline.draft.arbiter &&
    state.draft.token === baseline.draft.token &&
    state.draft.amount === baseline.draft.amount &&
    state.draft.timeoutSecs === baseline.draft.timeoutSecs &&
    state.createdId === null &&
    state.actionEscrowId === "" &&
    state.readEscrowId === "" &&
    state.resolveFavor === "seller" &&
    state.log.length === 0
  );
}

const STELLAR_SECRET_SEED_REGEX = /\bS[A-Z2-7]{55}\b/;
const FORBIDDEN_SECRET_KEYS = new Set([
  "secret",
  "secretKey",
  "secret_key",
  "privateKey",
  "private_key",
  "seed",
  "mnemonic",
  "signedTxXdr",
  "signed_tx_xdr",
  "signedEnvelope",
]);

function assertNoSensitiveMaterial(value: unknown, path = "root"): void {
  if (typeof value === "string") {
    if (STELLAR_SECRET_SEED_REGEX.test(value)) {
      throw new DemoStateValidationError(
        "SENSITIVE_DATA_REJECTED",
        `Demo state at '${path}' contains a Stellar secret seed. Demo exports must remain free of secrets.`,
      );
    }
    return;
  }

  if (Array.isArray(value)) {
    value.forEach((entry, idx) =>
      assertNoSensitiveMaterial(entry, `${path}[${idx}]`),
    );
    return;
  }

  if (value && typeof value === "object") {
    for (const [key, nested] of Object.entries(
      value as Record<string, unknown>,
    )) {
      if (FORBIDDEN_SECRET_KEYS.has(key)) {
        throw new DemoStateValidationError(
          "SENSITIVE_DATA_REJECTED",
          `Demo state field '${path}.${key}' is forbidden because secret keys and signed transaction payloads cannot be stored or exported.`,
        );
      }
      assertNoSensitiveMaterial(nested, `${path}.${key}`);
    }
  }
}

function validateDraft(raw: unknown): DemoEscrowDraft {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    throw new DemoStateValidationError(
      "INVALID_STATE_SHAPE",
      "Demo state `draft` must be an object.",
    );
  }
  const d = raw as Record<string, unknown>;
  const fields = [
    "buyer",
    "seller",
    "arbiter",
    "token",
    "amount",
    "timeoutSecs",
  ] as const;

  for (const field of fields) {
    if (typeof d[field] !== "string") {
      throw new DemoStateValidationError(
        "INVALID_STATE_SHAPE",
        `Demo state 'draft.${field}' must be a string.`,
      );
    }
  }

  return {
    buyer: d.buyer as string,
    seller: d.seller as string,
    arbiter: d.arbiter as string,
    token: d.token as string,
    amount: d.amount as string,
    timeoutSecs: d.timeoutSecs as string,
  };
}

function validateLogEntries(raw: unknown): DemoLogEntry[] {
  if (!Array.isArray(raw)) {
    throw new DemoStateValidationError(
      "INVALID_STATE_SHAPE",
      "Demo state `log` must be an array.",
    );
  }

  return raw.map((item, idx) => {
    if (!item || typeof item !== "object" || Array.isArray(item)) {
      throw new DemoStateValidationError(
        "INVALID_STATE_SHAPE",
        `Demo state 'log[${idx}]' must be an object.`,
      );
    }
    const entry = item as Record<string, unknown>;
    if (typeof entry.id !== "number" || !Number.isFinite(entry.id)) {
      throw new DemoStateValidationError(
        "INVALID_STATE_SHAPE",
        `Demo state 'log[${idx}].id' must be a finite number.`,
      );
    }
    if (typeof entry.time !== "string" || typeof entry.text !== "string") {
      throw new DemoStateValidationError(
        "INVALID_STATE_SHAPE",
        `Demo state 'log[${idx}]' must have string 'time' and 'text' fields.`,
      );
    }
    if (
      entry.kind !== "info" &&
      entry.kind !== "ok" &&
      entry.kind !== "err"
    ) {
      throw new DemoStateValidationError(
        "INVALID_STATE_SHAPE",
        `Demo state 'log[${idx}].kind' must be 'info', 'ok', or 'err'.`,
      );
    }
    return {
      id: entry.id,
      time: entry.time,
      text: entry.text,
      kind: entry.kind,
    };
  });
}

function validateStatePayload(raw: unknown): DemoStatePayload {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    throw new DemoStateValidationError(
      "INVALID_STATE_SHAPE",
      "Demo state payload must be an object.",
    );
  }

  const s = raw as Record<string, unknown>;
  const draft = validateDraft(s.draft);

  if (s.createdId !== null && typeof s.createdId !== "string") {
    throw new DemoStateValidationError(
      "INVALID_STATE_SHAPE",
      "Demo state `createdId` must be a string or null.",
    );
  }
  if (typeof s.actionEscrowId !== "string") {
    throw new DemoStateValidationError(
      "INVALID_STATE_SHAPE",
      "Demo state `actionEscrowId` must be a string.",
    );
  }
  if (typeof s.readEscrowId !== "string") {
    throw new DemoStateValidationError(
      "INVALID_STATE_SHAPE",
      "Demo state `readEscrowId` must be a string.",
    );
  }
  if (s.resolveFavor !== "seller" && s.resolveFavor !== "buyer") {
    throw new DemoStateValidationError(
      "INVALID_STATE_SHAPE",
      "Demo state `resolveFavor` must be 'seller' or 'buyer'.",
    );
  }

  const log = validateLogEntries(s.log ?? []);

  return {
    draft,
    createdId: s.createdId,
    actionEscrowId: s.actionEscrowId,
    readEscrowId: s.readEscrowId,
    resolveFavor: s.resolveFavor,
    log,
  };
}

export function validateDemoStateDocument(
  raw: unknown,
  fallbackExportedAt = "1970-01-01T00:00:00.000Z",
): VersionedDemoStateDocument {
  let parsed: unknown = raw;
  if (typeof raw === "string") {
    const trimmed = raw.trim();
    if (!trimmed) {
      throw new DemoStateValidationError(
        "MALFORMED_DATA",
        "Demo state JSON document cannot be empty.",
      );
    }
    try {
      parsed = JSON.parse(trimmed);
    } catch (error) {
      throw new DemoStateValidationError(
        "MALFORMED_DATA",
        `Demo state is not valid JSON (${error instanceof Error ? error.message : String(error)}).`,
      );
    }
  }

  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new DemoStateValidationError(
      "MALFORMED_DATA",
      "Demo state document must be a JSON object.",
    );
  }

  assertNoSensitiveMaterial(parsed);

  const doc = parsed as Record<string, unknown>;

  // Backward compatibility: allow legacy unversioned DemoStatePayload objects
  // that directly contain `draft` without `schemaVersion`.
  if (doc.schemaVersion === undefined && doc.state === undefined && doc.draft) {
    const state = validateStatePayload(doc);
    return {
      schemaVersion: DEMO_STATE_SCHEMA_VERSION,
      scope: DEMO_STATE_LABEL,
      exportedAt: fallbackExportedAt,
      state,
    };
  }

  if (typeof doc.schemaVersion !== "string" || !doc.schemaVersion.trim()) {
    throw new DemoStateValidationError(
      "MALFORMED_DATA",
      "Demo state document is missing required `schemaVersion` field.",
    );
  }

  if (doc.schemaVersion !== DEMO_STATE_SCHEMA_VERSION) {
    throw new DemoStateValidationError(
      "VERSION_MISMATCH",
      `Unsupported demo-state schemaVersion '${doc.schemaVersion}'. Expected '${DEMO_STATE_SCHEMA_VERSION}'.`,
    );
  }

  if (doc.scope !== undefined && doc.scope !== DEMO_STATE_LABEL) {
    throw new DemoStateValidationError(
      "INVALID_STATE_SHAPE",
      `Invalid demo-state scope '${String(doc.scope)}'. Expected '${DEMO_STATE_LABEL}'.`,
    );
  }

  const exportedAt =
    typeof doc.exportedAt === "string" && doc.exportedAt.trim().length > 0
      ? doc.exportedAt.trim()
      : fallbackExportedAt;

  const state = validateStatePayload(doc.state);

  return {
    schemaVersion: DEMO_STATE_SCHEMA_VERSION,
    scope: DEMO_STATE_LABEL,
    exportedAt,
    state,
  };
}

export function exportDemoStateJson(
  state: DemoStatePayload,
  exportedAt = new Date().toISOString(),
): string {
  const validated = validateDemoStateDocument(
    {
      schemaVersion: DEMO_STATE_SCHEMA_VERSION,
      scope: DEMO_STATE_LABEL,
      exportedAt,
      state,
    },
    exportedAt,
  );
  return `${JSON.stringify(validated, null, 2)}\n`;
}

export function saveDemoStateToStorage(
  state: DemoStatePayload,
  storage?: BrowserStorageLike | null,
  exportedAt = new Date().toISOString(),
): VersionedDemoStateDocument {
  const doc = validateDemoStateDocument(
    {
      schemaVersion: DEMO_STATE_SCHEMA_VERSION,
      scope: DEMO_STATE_LABEL,
      exportedAt,
      state,
    },
    exportedAt,
  );
  if (storage) {
    storage.setItem(DEMO_STATE_STORAGE_KEY, JSON.stringify(doc));
  }
  return doc;
}

export function loadDemoStateFromStorage(
  storage?: BrowserStorageLike | null,
  defaultToken: string = DEFAULT_DEMO_TOKEN,
): {
  state: DemoStatePayload;
  isEmpty: boolean;
  error?: DemoStateValidationError;
} {
  if (!storage) {
    const empty = createEmptyDemoState(defaultToken);
    return { state: empty, isEmpty: true };
  }

  const raw = storage.getItem(DEMO_STATE_STORAGE_KEY);
  if (raw === null || raw.trim() === "") {
    const empty = createEmptyDemoState(defaultToken);
    return { state: empty, isEmpty: true };
  }

  try {
    const doc = validateDemoStateDocument(raw);
    return {
      state: doc.state,
      isEmpty: isDemoStateEmpty(doc.state, defaultToken),
    };
  } catch (err) {
    const empty = createEmptyDemoState(defaultToken);
    const validationErr =
      err instanceof DemoStateValidationError
        ? err
        : new DemoStateValidationError(
            "MALFORMED_DATA",
            err instanceof Error ? err.message : String(err),
          );
    return {
      state: empty,
      isEmpty: true,
      error: validationErr,
    };
  }
}

export function resetDemoStateInStorage(
  storage?: BrowserStorageLike | null,
  defaultToken: string = DEFAULT_DEMO_TOKEN,
  options: { confirmed?: boolean } = {},
): DemoStatePayload {
  if (options.confirmed !== true) {
    throw new DemoStateValidationError(
      "CONFIRMATION_REQUIRED",
      "Resetting local demo state requires explicit confirmation (`confirmed: true`).",
    );
  }
  if (storage) {
    storage.removeItem(DEMO_STATE_STORAGE_KEY);
  }
  return createEmptyDemoState(defaultToken);
}

export function createMemoryBrowserStorage(
  initialEntries: Record<string, string> = {},
): BrowserStorageLike & { snapshot(): Record<string, string> } {
  const store = new Map<string, string>(Object.entries(initialEntries));
  return {
    getItem(key: string): string | null {
      return store.has(key) ? (store.get(key) ?? null) : null;
    },
    setItem(key: string, value: string): void {
      store.set(key, value);
    },
    removeItem(key: string): void {
      store.delete(key);
    },
    snapshot(): Record<string, string> {
      return Object.fromEntries(store.entries());
    },
  };
}
