import test from "node:test";
import assert from "node:assert/strict";
import {
  isValidStellarAddress,
  validateCreateEscrow,
  validateCreateVesting,
  validateMultiSigSubmit,
  validateMultiSigConfirm,
  validateDaoVote,
  validateSubscribe,
  validateSetRoyalty,
} from "../src/lib/validation.ts";

const VALID_ACCOUNT = "GBJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O";
const VALID_CONTRACT = "CBJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O";

test("isValidStellarAddress validates G... and C... addresses properly", () => {
  assert.equal(isValidStellarAddress(VALID_ACCOUNT), true);
  assert.equal(isValidStellarAddress(VALID_CONTRACT), true);
  assert.equal(isValidStellarAddress("invalid"), false);
  assert.equal(isValidStellarAddress(""), false);
  assert.equal(isValidStellarAddress("ABJQ53EOHB5MWSS7CETN523WILLNVS7NQAQQAQIS5QAYTPRCZSGKL23O"), false);
  assert.equal(isValidStellarAddress(VALID_ACCOUNT.slice(0, 50)), false);
});

test("Escrow: validateCreateEscrow accepts valid inputs", () => {
  const result = validateCreateEscrow({
    buyer: VALID_ACCOUNT,
    seller: VALID_ACCOUNT,
    arbiter: VALID_ACCOUNT,
    token: VALID_CONTRACT,
    amount: "10000000",
    timeoutSecs: "3600",
  });
  assert.equal(result.success, true);
  if (result.success) {
    assert.equal(result.data.amount, BigInt("10000000"));
    assert.equal(result.data.timeout, BigInt("3600"));
  }
});

test("Escrow: validateCreateEscrow rejects zero or negative amounts", () => {
  const result = validateCreateEscrow({
    buyer: VALID_ACCOUNT,
    seller: VALID_ACCOUNT,
    arbiter: VALID_ACCOUNT,
    token: VALID_CONTRACT,
    amount: "0",
    timeoutSecs: "3600",
  });
  assert.equal(result.success, false);
});

test("Vesting: validateCreateVesting rejects cliff greater than duration", () => {
  const result = validateCreateVesting({
    beneficiary: VALID_ACCOUNT,
    token: VALID_CONTRACT,
    totalAmount: "50000000",
    cliffSecs: "700000",
    durationSecs: "600000",
  });
  assert.equal(result.success, false);
  if (!result.success) {
    assert.match(result.error, /cliff.*cannot exceed.*duration/i);
  }
});

test("Vesting: validateCreateVesting accepts valid schedule inputs", () => {
  const result = validateCreateVesting({
    beneficiary: VALID_ACCOUNT,
    token: VALID_CONTRACT,
    totalAmount: "50000000",
    cliffSecs: "100000",
    durationSecs: "600000",
  });
  assert.equal(result.success, true);
  if (result.success) {
    assert.equal(result.data.total_amount, BigInt("50000000"));
    assert.equal(result.data.cliff, BigInt("100000"));
    assert.equal(result.data.duration, BigInt("600000"));
  }
});

test("Multi-Sig: validateMultiSigSubmit accepts valid withdrawal inputs", () => {
  const result = validateMultiSigSubmit({
    submitter: VALID_ACCOUNT,
    token: VALID_CONTRACT,
    destination: VALID_ACCOUNT,
    amount: "25000000",
  });
  assert.equal(result.success, true);
  if (result.success) {
    assert.equal(result.data.amount, BigInt("25000000"));
  }
});

test("Multi-Sig: validateMultiSigConfirm rejects invalid tx_id", () => {
  const result = validateMultiSigConfirm({
    signer: VALID_ACCOUNT,
    txId: "0",
  });
  assert.equal(result.success, false);
});

test("DAO: validateDaoVote constructs valid boolean support params", () => {
  const result = validateDaoVote({
    proposalId: "42",
    voter: VALID_ACCOUNT,
    support: true,
  });
  assert.equal(result.success, true);
  if (result.success) {
    assert.equal(result.data.proposal_id, BigInt(42));
    assert.equal(result.data.support, true);
  }
});

test("Subscription: validateSubscribe rejects identical subscriber and provider", () => {
  const result = validateSubscribe({
    subscriber: VALID_ACCOUNT,
    provider: VALID_ACCOUNT,
    token: VALID_CONTRACT,
    amount: "10000000",
    periodSecs: "2592000",
  });
  assert.equal(result.success, false);
  if (!result.success) {
    assert.match(result.error, /distinct/i);
  }
});

test("Marketplace: validateSetRoyalty rejects bps outside 0-10,000", () => {
  const result = validateSetRoyalty({
    collection: VALID_CONTRACT,
    recipient: VALID_ACCOUNT,
    bps: "12000",
  });
  assert.equal(result.success, false);
});

test("Marketplace: validateSetRoyalty accepts valid 500 bps (5%)", () => {
  const result = validateSetRoyalty({
    collection: VALID_CONTRACT,
    recipient: VALID_ACCOUNT,
    bps: "500",
  });
  assert.equal(result.success, true);
  if (result.success) {
    assert.equal(result.data.bps, 500);
  }
});
