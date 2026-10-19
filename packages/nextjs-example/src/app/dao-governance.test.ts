import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  canExecuteProposal,
  canVoteOnProposal,
  classifyDaoError,
  computeVoteBreakdown,
  deriveEffectiveLifecycle,
  formatActionPreview,
  isValidStellarAddress,
  lifecycleBadgeClass,
  parseActionPayload,
  validateProposalDraft,
  type DaoProposalView,
} from "./dao-governance.ts";

const VALID_ACCOUNT =
  "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";
const VALID_CONTRACT =
  "CBJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O";

function makeProposal(overrides: Partial<DaoProposalView> = {}): DaoProposalView {
  return {
    proposal_id: BigInt(1),
    proposer: VALID_ACCOUNT,
    target: VALID_CONTRACT,
    action: new Uint8Array([0x72, 0x65, 0x6c, 0x65, 0x61, 0x73, 0x65]),
    for_votes: BigInt(150_000_000),
    against_votes: BigInt(50_000_000),
    voting_ends: BigInt(2_000),
    state: { tag: "Active" },
    bond_token: VALID_CONTRACT,
    bond_amount: BigInt(10_000_000),
    bond_state: { tag: "Posted" },
    ...overrides,
  };
}

describe("DAO governance helpers", () => {
  it("derives effective proposal lifecycle before and after voting_ends", () => {
    const activeProposal = makeProposal({
      for_votes: BigInt(200),
      against_votes: BigInt(100),
      voting_ends: BigInt(1_000),
      state: { tag: "Active" },
    });

    assert.equal(deriveEffectiveLifecycle(activeProposal, BigInt(999)), "Active");
    assert.equal(
      deriveEffectiveLifecycle(activeProposal, BigInt(1_000)),
      "Succeeded"
    );

    const tiedProposal = makeProposal({
      for_votes: BigInt(100),
      against_votes: BigInt(100),
      voting_ends: BigInt(1_000),
      state: { tag: "Active" },
    });
    assert.equal(
      deriveEffectiveLifecycle(tiedProposal, BigInt(1_000)),
      "Defeated"
    );

    const executedProposal = makeProposal({
      state: { tag: "Executed" },
      voting_ends: BigInt(500),
    });
    assert.equal(
      deriveEffectiveLifecycle(executedProposal, BigInt(1_000)),
      "Executed"
    );
  });

  it("maps lifecycle tags to Tailwind badge classes", () => {
    assert.match(lifecycleBadgeClass("Active"), /bg-blue-100/);
    assert.match(lifecycleBadgeClass("Succeeded"), /bg-green-100/);
    assert.match(lifecycleBadgeClass("Defeated"), /bg-red-100/);
    assert.match(lifecycleBadgeClass("Executed"), /bg-purple-100/);
    assert.match(lifecycleBadgeClass("Queued"), /bg-yellow-100/);
    assert.match(lifecycleBadgeClass("Cancelled"), /bg-gray-200/);
  });

  it("computes vote breakdown accurately across zero, majority, and tie cases", () => {
    const empty = computeVoteBreakdown(BigInt(0), BigInt(0));
    assert.equal(empty.totalVotes, BigInt(0));
    assert.equal(empty.forPct, 0);
    assert.equal(empty.againstPct, 0);
    assert.equal(empty.hasQuorumMajority, false);

    const majority = computeVoteBreakdown(BigInt(750), BigInt(250));
    assert.equal(majority.totalVotes, BigInt(1000));
    assert.equal(majority.forPct, 75);
    assert.equal(majority.againstPct, 25);
    assert.equal(majority.hasQuorumMajority, true);

    const tie = computeVoteBreakdown(BigInt(500), BigInt(500));
    assert.equal(tie.hasQuorumMajority, false);
  });

  it("enforces voting eligibility for disconnected wallets, duplicate votes, and expired proposals", () => {
    const proposal = makeProposal({ voting_ends: BigInt(2_000) });

    const disconnected = canVoteOnProposal(proposal, null, BigInt(1_500), false);
    assert.equal(disconnected.allowed, false);
    assert.equal(disconnected.errorCategory, "unauthorized");

    const duplicate = canVoteOnProposal(
      proposal,
      VALID_ACCOUNT,
      BigInt(1_500),
      true
    );
    assert.equal(duplicate.allowed, false);
    assert.equal(duplicate.errorCategory, "duplicate_vote");

    const expired = canVoteOnProposal(
      proposal,
      VALID_ACCOUNT,
      BigInt(2_000),
      false
    );
    assert.equal(expired.allowed, false);
    assert.equal(expired.errorCategory, "expired_proposal");

    const eligible = canVoteOnProposal(
      proposal,
      VALID_ACCOUNT,
      BigInt(1_500),
      false
    );
    assert.equal(eligible.allowed, true);
    assert.equal(eligible.reason, null);
  });

  it("enables Execute only when a proposal has Succeeded and voting time has concluded", () => {
    const activeBeforeEnd = makeProposal({
      for_votes: BigInt(300),
      against_votes: BigInt(100),
      voting_ends: BigInt(2_000),
      state: { tag: "Active" },
    });
    assert.equal(canExecuteProposal(activeBeforeEnd, BigInt(1_999)).allowed, false);
    assert.equal(canExecuteProposal(activeBeforeEnd, BigInt(2_000)).allowed, true);

    const explicitSucceeded = makeProposal({
      voting_ends: BigInt(2_000),
      state: { tag: "Succeeded" },
    });
    assert.equal(
      canExecuteProposal(explicitSucceeded, BigInt(2_000)).allowed,
      true
    );

    const defeatedAfterEnd = makeProposal({
      for_votes: BigInt(100),
      against_votes: BigInt(300),
      voting_ends: BigInt(2_000),
      state: { tag: "Active" },
    });
    assert.equal(
      canExecuteProposal(defeatedAfterEnd, BigInt(2_500)).allowed,
      false
    );

    const alreadyExecuted = makeProposal({
      voting_ends: BigInt(2_000),
      state: { tag: "Executed" },
    });
    assert.equal(
      canExecuteProposal(alreadyExecuted, BigInt(2_500)).allowed,
      false
    );
  });

  it("parses UTF-8 and hex action payloads and validates proposal drafts", () => {
    const utf8Bytes = parseActionPayload("release(1)", "utf8");
    assert.equal(formatActionPreview(utf8Bytes).text, "release(1)");

    const hexBytes = parseActionPayload("0xdeadbeef", "hex");
    assert.deepEqual(Array.from(hexBytes), [0xde, 0xad, 0xbe, 0xef]);
    assert.equal(formatActionPreview(hexBytes).hex, "0xdeadbeef");

    assert.throws(() => parseActionPayload("0x123", "hex"), /even number/);

    assert.equal(isValidStellarAddress(VALID_ACCOUNT), true);
    assert.equal(isValidStellarAddress(VALID_CONTRACT), true);
    assert.equal(isValidStellarAddress("invalid"), false);

    const validDraft = validateProposalDraft({
      proposer: VALID_ACCOUNT,
      target: VALID_CONTRACT,
      actionInput: "set_fee_bps:50",
      actionEncoding: "utf8",
      durationSecs: "3600",
    });
    assert.equal(validDraft.ok, true);

    const missingProposer = validateProposalDraft({
      proposer: "",
      target: VALID_CONTRACT,
      actionInput: "set_fee_bps:50",
      actionEncoding: "utf8",
      durationSecs: "3600",
    });
    assert.equal(missingProposer.ok, false);
    if (!missingProposer.ok) {
      assert.equal(missingProposer.banner.category, "unauthorized");
    }
  });

  it("classifies duplicate votes, expired proposals, unauthorized accounts, and ForgeError codes", () => {
    const dupVote = classifyDaoError(
      new Error("HostError: Error(Contract, #3) InvalidInput"),
      "vote"
    );
    assert.equal(dupVote.category, "duplicate_vote");
    assert.equal(dupVote.code, 3);

    const expiredVote = classifyDaoError(
      new Error("HostError: Error(Contract, #7) DeadlineReached"),
      "vote"
    );
    assert.equal(expiredVote.category, "expired_proposal");
    assert.equal(expiredVote.code, 7);

    const unauthorized = classifyDaoError(
      new Error("HostError: Error(Contract, #1) Unauthorized"),
      "propose"
    );
    assert.equal(unauthorized.category, "unauthorized");
    assert.equal(unauthorized.code, 1);

    const zeroWeight = classifyDaoError(
      new Error("HostError: Error(Contract, #4) InsufficientFunds"),
      "vote"
    );
    assert.equal(zeroWeight.category, "unauthorized");
    assert.equal(zeroWeight.code, 4);

    const notInit = classifyDaoError(
      new Error("HostError: Error(Contract, #6) NotInitialized"),
      "propose"
    );
    assert.equal(notInit.category, "not_initialized");
    assert.equal(notInit.code, 6);

    const execFailed = classifyDaoError(
      new Error("HostError: Error(Contract, #12) ContractInvocationFailed"),
      "execute"
    );
    assert.equal(execFailed.category, "execution_failed");
    assert.equal(execFailed.code, 12);
  });
});
