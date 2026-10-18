// Mapping of ForgeError codes from crates/shared-utils/src/errors.rs
export const FORGE_ERRORS: Record<number, { name: string; description: string }> = {
  1: {
    name: "Unauthorized",
    description: "The caller is not permitted to perform this action.",
  },
  2: {
    name: "NotFound",
    description: "The requested entity (escrow, proposal, subscription, schedule, etc.) does not exist.",
  },
  3: {
    name: "InvalidInput",
    description: "One or more arguments failed validation (e.g. zero amount, invalid address, or invalid parameters).",
  },
  4: {
    name: "InsufficientFunds",
    description: "The contract or caller does not hold enough balance to satisfy the operation.",
  },
  5: {
    name: "AlreadyInitialized",
    description: "The entity was already initialized; re-initialization is rejected.",
  },
  6: {
    name: "NotInitialized",
    description: "The entity was expected to be initialized but was not.",
  },
  7: {
    name: "DeadlineReached",
    description: "An operation was attempted after its deadline elapsed.",
  },
  8: {
    name: "InsufficientAllowance",
    description: "A required token allowance was lower than the amount being spent.",
  },
  9: {
    name: "ArithmeticOverflow",
    description: "An arithmetic operation overflowed.",
  },
  10: {
    name: "Custom",
    description: "Contract-specific error that does not map to the categories above.",
  },
  11: {
    name: "TokenTransferFailed",
    description: "A SEP-41 token invocation failed (insufficient balance, missing trustline, etc.).",
  },
  12: {
    name: "ContractInvocationFailed",
    description: "A cross-contract invocation failed (target reverted or host abort).",
  },
  13: {
    name: "WithdrawalLimitExceeded",
    description: "The requested withdrawal would push a token's rolling-window total past its configured limit.",
  },
};

/**
 * Returns the name and description of a ForgeError by code.
 */
export function forgeErrorName(code: number): string | undefined {
  return FORGE_ERRORS[code]?.name;
}

/**
 * Parsed error details containing friendly ForgeError info when available.
 */
export interface ParsedError {
  code?: number;
  name?: string;
  description?: string;
  rawMessage: string;
}

/**
 * Parse an unknown error (from simulation, submission, or unwrap failure) into a human-readable structure.
 */
export function parseForgeError(error: unknown): ParsedError {
  const rawMessage =
    error instanceof Error ? error.message : typeof error === "string" ? error : JSON.stringify(error);

  // Check for Soroban host error format: Error(Contract, #123) or Error(WasmVm, ...) or error code in message
  const contractCodeMatch = rawMessage.match(/Error\(Contract,\s*#?(\d+)\)/i);
  if (contractCodeMatch) {
    const code = parseInt(contractCodeMatch[1], 10);
    if (FORGE_ERRORS[code]) {
      return {
        code,
        name: FORGE_ERRORS[code].name,
        description: FORGE_ERRORS[code].description,
        rawMessage,
      };
    }
  }

  // Check for "#<number>" or "code <number>" or ForgeError name mentions
  const genericCodeMatch = rawMessage.match(/(?:ForgeError|Error code|code)[:\s]+#?(\d+)/i);
  if (genericCodeMatch) {
    const code = parseInt(genericCodeMatch[1], 10);
    if (FORGE_ERRORS[code]) {
      return {
        code,
        name: FORGE_ERRORS[code].name,
        description: FORGE_ERRORS[code].description,
        rawMessage,
      };
    }
  }

  // Check for error names directly mentioned (e.g. "Unauthorized", "NotFound", etc.)
  for (const [codeStr, item] of Object.entries(FORGE_ERRORS)) {
    const code = Number(codeStr);
    const regex = new RegExp(`\\b${item.name}\\b`, "i");
    if (regex.test(rawMessage)) {
      return {
        code,
        name: item.name,
        description: item.description,
        rawMessage,
      };
    }
  }

  return {
    rawMessage,
  };
}

export interface DecodedEvent {
  topic: string;
  value: string;
  type?: string;
  inSuccessfulContractCall?: boolean;
}

/**
 * Decode contract diagnostic or transaction events.
 */
export function decodeContractEvents(events: unknown[]): DecodedEvent[] {
  if (!Array.isArray(events) || events.length === 0) {
    return [];
  }

  return events.map((ev: any, idx: number) => {
    try {
      let topicStr = `Event #${idx + 1}`;
      let valStr = "";

      if (ev.topic && Array.isArray(ev.topic)) {
        topicStr = ev.topic
          .map((t: any) => {
            if (typeof t === "string") return t;
            if (t?._value !== undefined) return String(t._value);
            return JSON.stringify(t);
          })
          .join(" / ");
      } else if (ev.type) {
        topicStr = String(ev.type);
      }

      if (ev.value !== undefined) {
        if (typeof ev.value === "string") {
          valStr = ev.value;
        } else if (ev.value?._value !== undefined) {
          valStr = String(ev.value._value);
        } else {
          valStr = JSON.stringify(ev.value);
        }
      } else if (ev.data !== undefined) {
        valStr = typeof ev.data === "string" ? ev.data : JSON.stringify(ev.data);
      } else {
        valStr = JSON.stringify(ev);
      }

      return {
        topic: topicStr,
        value: valStr,
        type: ev.type ? String(ev.type) : undefined,
        inSuccessfulContractCall: ev.inSuccessfulContractCall,
      };
    } catch {
      return {
        topic: `Event #${idx + 1}`,
        value: String(ev),
      };
    }
  });
}
