export type ProposalLifecycleTag =
  | "Active"
  | "Succeeded"
  | "Defeated"
  | "Executed"
  | "Queued"
  | "Cancelled";

export type BondLifecycleTag = "Posted" | "Refunded" | "Forfeited";

export interface DaoProposalView {
  proposal_id: bigint;
  proposer: string;
  target: string;
  action: Uint8Array;
  for_votes: bigint;
  against_votes: bigint;
  voting_ends: bigint;
  state: { tag: ProposalLifecycleTag; values?: void };
  bond_token: string;
  bond_amount: bigint;
  bond_state: { tag: BondLifecycleTag; values?: void };
}

export type DaoErrorCategory =
  | "duplicate_vote"
  | "expired_proposal"
  | "unauthorized"
  | "not_initialized"
  | "token_transfer"
  | "execution_failed"
  | "validation"
  | "network";

export interface DaoErrorBanner {
  category: DaoErrorCategory;
  title: string;
  message: string;
  code: number | null;
}

export interface VoteEligibility {
  allowed: boolean;
  reason: string | null;
  errorCategory?: DaoErrorCategory;
}

export interface ExecuteEligibility {
  allowed: boolean;
  reason: string | null;
}

export interface VoteBreakdown {
  forVotes: bigint;
  againstVotes: bigint;
  totalVotes: bigint;
  forPct: number;
  againstPct: number;
  hasQuorumMajority: boolean;
}

export interface ProposalDraftInput {
  proposer: string;
  target: string;
  actionInput: string;
  actionEncoding: "utf8" | "hex";
  durationSecs: string;
}

export type ValidatedProposalDraft =
  | {
      ok: true;
      proposer: string;
      target: string;
      actionBytes: Uint8Array;
      duration: bigint;
    }
  | {
      ok: false;
      banner: DaoErrorBanner;
    };

const STELLAR_ADDRESS_RE = /^[GC][A-Z2-7]{55}$/;

export function isValidStellarAddress(address: string): boolean {
  return STELLAR_ADDRESS_RE.test(address.trim());
}

/**
 * Determine the effective lifecycle badge of a proposal given the current Unix timestamp (seconds).
 *
 * When a proposal is still stored on-chain as `Active` but its `voting_ends` deadline has elapsed,
 * the UI reflects whether it has reached `Succeeded` (strict `for_votes > against_votes`) or
 * `Defeated` (`for_votes <= against_votes`) so token holders know whether it can be executed.
 */
export function deriveEffectiveLifecycle(
  proposal: Pick<
    DaoProposalView,
    "state" | "voting_ends" | "for_votes" | "against_votes"
  >,
  nowSec: bigint
): ProposalLifecycleTag {
  const onChainTag = proposal.state.tag;
  if (onChainTag !== "Active") {
    return onChainTag;
  }
  if (nowSec >= proposal.voting_ends) {
    return proposal.for_votes > proposal.against_votes
      ? "Succeeded"
      : "Defeated";
  }
  return "Active";
}

/**
 * Return Tailwind CSS classes for a proposal lifecycle badge (`Active`, `Succeeded`, `Defeated`, `Executed`, etc.).
 */
export function lifecycleBadgeClass(tag: ProposalLifecycleTag): string {
  switch (tag) {
    case "Active":
      return "bg-blue-100 text-blue-800 border border-blue-200";
    case "Succeeded":
      return "bg-green-100 text-green-800 border border-green-200";
    case "Defeated":
      return "bg-red-100 text-red-800 border border-red-200";
    case "Executed":
      return "bg-purple-100 text-purple-800 border border-purple-200";
    case "Queued":
      return "bg-yellow-100 text-yellow-800 border border-yellow-200";
    case "Cancelled":
      return "bg-gray-200 text-gray-700 border border-gray-300";
  }
}

/**
 * Compute vote percentages and majority status without floating-point overflow on large i128 tallies.
 */
export function computeVoteBreakdown(
  forVotes: bigint,
  againstVotes: bigint
): VoteBreakdown {
  const safeFor = forVotes > BigInt(0) ? forVotes : BigInt(0);
  const safeAgainst = againstVotes > BigInt(0) ? againstVotes : BigInt(0);
  const totalVotes = safeFor + safeAgainst;
  if (totalVotes === BigInt(0)) {
    return {
      forVotes: safeFor,
      againstVotes: safeAgainst,
      totalVotes: BigInt(0),
      forPct: 0,
      againstPct: 0,
      hasQuorumMajority: false,
    };
  }
  const forBasisPoints = Number((safeFor * BigInt(10000)) / totalVotes);
  const forTenths = Math.round(forBasisPoints / 10);
  const forPct = forTenths / 10;
  const againstPct = Math.max(0, Math.min(100, (1000 - forTenths) / 10));
  return {
    forVotes: safeFor,
    againstVotes: safeAgainst,
    totalVotes,
    forPct,
    againstPct,
    hasQuorumMajority: safeFor > safeAgainst,
  };
}

/**
 * Check whether the connected account can cast a `for` or `against` vote on a proposal.
 */
export function canVoteOnProposal(
  proposal: Pick<
    DaoProposalView,
    "state" | "voting_ends" | "for_votes" | "against_votes"
  >,
  account: string | null,
  nowSec: bigint,
  alreadyVoted: boolean
): VoteEligibility {
  if (!account || !account.trim()) {
    return {
      allowed: false,
      reason: "Connect a wallet with governance tokens to cast a vote.",
      errorCategory: "unauthorized",
    };
  }
  if (alreadyVoted) {
    return {
      allowed: false,
      reason:
        "Duplicate vote blocked: this wallet has already voted on this proposal.",
      errorCategory: "duplicate_vote",
    };
  }
  if (proposal.state.tag !== "Active") {
    return {
      allowed: false,
      reason: `Voting is closed because proposal status is ${proposal.state.tag}.`,
      errorCategory: "expired_proposal",
    };
  }
  if (nowSec >= proposal.voting_ends) {
    return {
      allowed: false,
      reason:
        "Proposal voting period has expired. No additional votes can be submitted.",
      errorCategory: "expired_proposal",
    };
  }
  return { allowed: true, reason: null };
}

/**
 * Determine whether the "Execute" button should be enabled for a proposal.
 *
 * A proposal can be executed when its voting window has concluded (`nowSec >= voting_ends`)
 * and its effective status is `Succeeded` (either already transitioned to `Succeeded` on-chain
 * or currently `Active` past deadline with strict majority `for_votes > against_votes`).
 */
export function canExecuteProposal(
  proposal: Pick<
    DaoProposalView,
    "state" | "voting_ends" | "for_votes" | "against_votes"
  >,
  nowSec: bigint
): ExecuteEligibility {
  if (proposal.state.tag === "Executed") {
    return {
      allowed: false,
      reason: "Proposal has already been executed.",
    };
  }
  if (proposal.state.tag === "Cancelled") {
    return {
      allowed: false,
      reason: "Cancelled proposals cannot be executed.",
    };
  }
  if (proposal.state.tag === "Defeated") {
    return {
      allowed: false,
      reason: "Defeated proposals cannot be executed.",
    };
  }
  if (nowSec < proposal.voting_ends) {
    return {
      allowed: false,
      reason: "Voting window is still open; wait until voting ends to execute.",
    };
  }
  const effective = deriveEffectiveLifecycle(proposal, nowSec);
  if (effective !== "Succeeded") {
    return {
      allowed: false,
      reason:
        "Proposal did not reach a strict majority of For votes and cannot be executed.",
    };
  }
  return { allowed: true, reason: null };
}

/**
 * Parse a proposal action payload from either UTF-8 text or hex string into bytes.
 */
export function parseActionPayload(
  rawInput: string,
  encoding: "utf8" | "hex"
): Uint8Array {
  const trimmed = rawInput.trim();
  if (!trimmed) {
    throw new Error("Action payload cannot be empty.");
  }
  if (encoding === "hex") {
    const cleanHex = trimmed.startsWith("0x") || trimmed.startsWith("0X")
      ? trimmed.slice(2)
      : trimmed;
    if (cleanHex.length === 0 || cleanHex.length % 2 !== 0 || !/^[0-9a-fA-F]+$/.test(cleanHex)) {
      throw new Error(
        "Hex action payload must contain an even number of hexadecimal characters (0-9, a-f)."
      );
    }
    const bytes = new Uint8Array(cleanHex.length / 2);
    for (let i = 0; i < cleanHex.length; i += 2) {
      bytes[i / 2] = Number.parseInt(cleanHex.slice(i, i + 2), 16);
    }
    return bytes;
  }
  return new TextEncoder().encode(trimmed);
}

/**
 * Format an action byte buffer for compact display in proposal cards.
 */
export function formatActionPreview(action: Uint8Array): {
  hex: string;
  text: string | null;
} {
  const bytes = action instanceof Uint8Array ? action : new Uint8Array(action);
  const hex =
    "0x" +
    Array.from(bytes)
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
  if (bytes.length === 0) {
    return { hex: "0x", text: null };
  }
  const isPrintableAscii = bytes.every((b) => b >= 0x20 && b <= 0x7e);
  if (isPrintableAscii) {
    return { hex, text: new TextDecoder().decode(bytes) };
  }
  return { hex, text: null };
}

/**
 * Validate inputs for creating a new DAO governance proposal.
 */
export function validateProposalDraft(
  draft: ProposalDraftInput
): ValidatedProposalDraft {
  const proposer = draft.proposer.trim();
  if (!proposer) {
    return {
      ok: false,
      banner: {
        category: "unauthorized",
        title: "Unauthorized Account",
        message:
          "Connect your Stellar wallet or specify a valid proposer public key before creating a proposal.",
        code: 1,
      },
    };
  }
  if (!isValidStellarAddress(proposer)) {
    return {
      ok: false,
      banner: {
        category: "unauthorized",
        title: "Invalid Proposer Address",
        message:
          "Proposer address must be a valid 56-character Stellar public key (G…) or contract address (C…).",
        code: 3,
      },
    };
  }

  const target = draft.target.trim();
  if (!isValidStellarAddress(target)) {
    return {
      ok: false,
      banner: {
        category: "validation",
        title: "Invalid Target Contract Address",
        message:
          "Target address must be a valid 56-character Stellar contract (C…) or account (G…) address.",
        code: 3,
      },
    };
  }

  let actionBytes: Uint8Array;
  try {
    actionBytes = parseActionPayload(draft.actionInput, draft.actionEncoding);
  } catch (err) {
    return {
      ok: false,
      banner: {
        category: "validation",
        title: "Invalid Action Payload",
        message: (err as Error).message,
        code: 3,
      },
    };
  }

  const durationTrimmed = draft.durationSecs.trim();
  if (!/^\d+$/.test(durationTrimmed)) {
    return {
      ok: false,
      banner: {
        category: "validation",
        title: "Invalid Voting Duration",
        message: "Voting duration must be a positive integer number of seconds.",
        code: 3,
      },
    };
  }
  const duration = BigInt(durationTrimmed);
  if (duration <= BigInt(0)) {
    return {
      ok: false,
      banner: {
        category: "validation",
        title: "Invalid Voting Duration",
        message: "Voting duration must be greater than 0 seconds.",
        code: 3,
      },
    };
  }

  return {
    ok: true,
    proposer,
    target,
    actionBytes,
    duration,
  };
}

function extractContractErrorCode(rawMessage: string): number | null {
  const contractMatch = rawMessage.match(/Error\(Contract,\s*#(\d+)\)/i);
  if (contractMatch) {
    return Number.parseInt(contractMatch[1], 10);
  }
  const codeMatch = rawMessage.match(/\bcode\s*[:=#]?\s*(\d+)\b/i);
  if (codeMatch) {
    return Number.parseInt(codeMatch[1], 10);
  }
  return null;
}

/**
 * Classify an error thrown during DAO proposal creation, voting, or execution into
 * a structured, user-friendly error banner covering duplicate votes, expired proposals,
 * unauthorized accounts, and Soroban `ForgeError` codes.
 */
export function classifyDaoError(
  error: unknown,
  actionContext: "propose" | "vote" | "execute" | "cancel" | "load" = "vote"
): DaoErrorBanner {
  const rawMessage =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : "Unknown DAO transaction failure";
  const lower = rawMessage.toLowerCase();
  const code = extractContractErrorCode(rawMessage);

  // Duplicate vote detection
  if (
    lower.includes("already voted") ||
    lower.includes("duplicate vote") ||
    lower.includes("has_voted") ||
    (actionContext === "vote" && (code === 3 || lower.includes("invalidinput")))
  ) {
    return {
      category: "duplicate_vote",
      title: "Duplicate Vote Rejected",
      message:
        "Your account has already cast a vote on this proposal. Each governance account may vote only once per proposal.",
      code: code ?? 3,
    };
  }

  // Expired proposal / deadline reached detection
  if (
    code === 7 ||
    lower.includes("deadlinereached") ||
    lower.includes("deadline reached") ||
    lower.includes("expired") ||
    lower.includes("voting closed") ||
    lower.includes("voting has ended") ||
    lower.includes("voting period has expired")
  ) {
    return {
      category: "expired_proposal",
      title:
        actionContext === "execute"
          ? "Proposal Execution Timing Error"
          : "Expired Proposal — Voting Closed",
      message:
        actionContext === "execute"
          ? "This proposal cannot be executed yet because its voting window has not concluded or it is no longer executable."
          : "The voting deadline (voting_ends) for this proposal has already passed. Votes can only be submitted while the proposal is Active.",
      code: code ?? 7,
    };
  }

  // Unauthorized account / zero voting power / rejected wallet signature
  if (
    code === 1 ||
    code === 4 ||
    code === 8 ||
    lower.includes("unauthorized") ||
    lower.includes("not permitted") ||
    lower.includes("insufficientfunds") ||
    lower.includes("insufficientallowance") ||
    lower.includes("zero voting power") ||
    lower.includes("no voting weight") ||
    lower.includes("user declined") ||
    lower.includes("rejected by user")
  ) {
    return {
      category: "unauthorized",
      title: "Unauthorized Account or Insufficient Governance Weight",
      message:
        code === 4 || lower.includes("insufficientfunds")
          ? "Your connected wallet holds zero governance token balance or insufficient bond balance for this action."
          : "The connected wallet is not authorized for this governance action or declined transaction signing.",
      code: code ?? 1,
    };
  }

  // Bond not configured / NotInitialized
  if (
    code === 6 ||
    lower.includes("notinitialized") ||
    lower.includes("not initialized")
  ) {
    return {
      category: "not_initialized",
      title: "DAO Bond Configuration Missing",
      message:
        "The DAO contract has not had its proposal bond configured yet (ForgeError::NotInitialized). Configure the proposal bond before creating proposals.",
      code: 6,
    };
  }

  // Cross-contract target execution failure
  if (
    code === 12 ||
    lower.includes("contractinvocationfailed") ||
    lower.includes("invocation failed")
  ) {
    return {
      category: "execution_failed",
      title: "Target Contract Execution Failed",
      message:
        "The proposal's target contract reverted during execution (ForgeError::ContractInvocationFailed). The proposal remains Succeeded and its bond stays in custody.",
      code: 12,
    };
  }

  // Token transfer failure (bond pull / refund / forfeit)
  if (
    code === 11 ||
    lower.includes("tokentransferfailed") ||
    lower.includes("token transfer failed")
  ) {
    return {
      category: "token_transfer",
      title: "Governance Token Transfer Failed",
      message:
        "The SEP-41 bond or governance token transfer failed (ForgeError::TokenTransferFailed). Check your token balance and allowance.",
      code: 11,
    };
  }

  return {
    category: "network",
    title: "DAO Governance Error",
    message: rawMessage,
    code,
  };
}
