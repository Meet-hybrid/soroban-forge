/**
 * Pre-flight input validation for developer console write flows.
 * Ensures invalid arguments are caught before signing and submission.
 */

// Basic Stellar StrKey check: 56 characters, base32 (A-Z, 2-7), starting with G (account) or C (contract)
export function isValidStellarAddress(address: string): boolean {
  if (typeof address !== "string") return false;
  const trimmed = address.trim();
  return /^[GC][A-Z2-7]{55}$/.test(trimmed);
}

export type ValidationResult<T> =
  | { success: true; data: T }
  | { success: false; error: string };

// 1. Escrow: create_escrow
export interface CreateEscrowInputs {
  buyer: string;
  seller: string;
  arbiter: string;
  token: string;
  amount: string; // stroops
  timeoutSecs: string;
}

export interface ValidatedEscrowParams {
  buyer: string;
  seller: string;
  arbiter: string;
  token: string;
  amount: bigint;
  timeout: bigint;
}

export function validateCreateEscrow(inputs: CreateEscrowInputs): ValidationResult<ValidatedEscrowParams> {
  const buyer = inputs.buyer.trim();
  const seller = inputs.seller.trim();
  const arbiter = inputs.arbiter.trim();
  const token = inputs.token.trim();

  if (!buyer || !isValidStellarAddress(buyer)) {
    return { success: false, error: "Buyer must be a valid 56-character Stellar account or contract address (G... or C...)." };
  }
  if (!seller || !isValidStellarAddress(seller)) {
    return { success: false, error: "Seller must be a valid 56-character Stellar address (G... or C...)." };
  }
  if (!arbiter || !isValidStellarAddress(arbiter)) {
    return { success: false, error: "Arbiter must be a valid 56-character Stellar address (G... or C...)." };
  }
  if (!token || !isValidStellarAddress(token)) {
    return { success: false, error: "Token must be a valid 56-character Stellar contract address (C...)." };
  }

  let amountBig: bigint;
  try {
    amountBig = BigInt(inputs.amount.trim() || "0");
  } catch {
    return { success: false, error: "Amount must be a valid positive integer." };
  }
  if (amountBig <= BigInt(0)) {
    return { success: false, error: "Amount must be strictly greater than 0 stroops." };
  }

  let timeoutBig: bigint;
  try {
    timeoutBig = BigInt(inputs.timeoutSecs.trim() || "0");
  } catch {
    return { success: false, error: "Timeout must be a valid integer number of seconds." };
  }
  if (timeoutBig <= BigInt(0)) {
    return { success: false, error: "Timeout duration must be greater than 0 seconds." };
  }

  return {
    success: true,
    data: {
      buyer,
      seller,
      arbiter,
      token,
      amount: amountBig,
      timeout: timeoutBig,
    },
  };
}

// 2. Vesting: create_schedule
export interface CreateVestingInputs {
  beneficiary: string;
  token: string;
  totalAmount: string; // stroops
  cliffSecs: string;
  durationSecs: string;
}

export interface ValidatedVestingParams {
  beneficiary: string;
  token: string;
  total_amount: bigint;
  cliff: bigint;
  duration: bigint;
}

export function validateCreateVesting(inputs: CreateVestingInputs): ValidationResult<ValidatedVestingParams> {
  const beneficiary = inputs.beneficiary.trim();
  const token = inputs.token.trim();

  if (!beneficiary || !isValidStellarAddress(beneficiary)) {
    return { success: false, error: "Beneficiary must be a valid 56-character Stellar address (G... or C...)." };
  }
  if (!token || !isValidStellarAddress(token)) {
    return { success: false, error: "Token must be a valid 56-character Stellar contract address (C...)." };
  }

  let amountBig: bigint;
  try {
    amountBig = BigInt(inputs.totalAmount.trim() || "0");
  } catch {
    return { success: false, error: "Total amount must be a valid integer." };
  }
  if (amountBig <= BigInt(0)) {
    return { success: false, error: "Total amount must be strictly greater than 0." };
  }

  let cliffBig: bigint;
  try {
    cliffBig = BigInt(inputs.cliffSecs.trim() || "0");
  } catch {
    return { success: false, error: "Cliff must be a non-negative integer of seconds." };
  }
  if (cliffBig < BigInt(0)) {
    return { success: false, error: "Cliff cannot be negative." };
  }

  let durationBig: bigint;
  try {
    durationBig = BigInt(inputs.durationSecs.trim() || "0");
  } catch {
    return { success: false, error: "Duration must be a valid integer of seconds." };
  }
  if (durationBig <= BigInt(0)) {
    return { success: false, error: "Duration must be strictly greater than 0." };
  }

  if (cliffBig > durationBig) {
    return { success: false, error: "Cliff duration cannot exceed total schedule duration." };
  }

  return {
    success: true,
    data: {
      beneficiary,
      token,
      total_amount: amountBig,
      cliff: cliffBig,
      duration: durationBig,
    },
  };
}

// 3. Multi-Sig Wallet: submit_withdrawal or confirm
export interface MultiSigSubmitInputs {
  submitter: string;
  token: string;
  destination: string;
  amount: string;
}

export interface ValidatedMultiSigSubmitParams {
  submitter: string;
  token: string;
  destination: string;
  amount: bigint;
}

export function validateMultiSigSubmit(inputs: MultiSigSubmitInputs): ValidationResult<ValidatedMultiSigSubmitParams> {
  const submitter = inputs.submitter.trim();
  const token = inputs.token.trim();
  const destination = inputs.destination.trim();

  if (!submitter || !isValidStellarAddress(submitter)) {
    return { success: false, error: "Submitter must be an authorized owner address (G...)." };
  }
  if (!token || !isValidStellarAddress(token)) {
    return { success: false, error: "Token must be a valid Stellar contract address (C...)." };
  }
  if (!destination || !isValidStellarAddress(destination)) {
    return { success: false, error: "Destination must be a valid recipient address (G... or C...)." };
  }

  let amountBig: bigint;
  try {
    amountBig = BigInt(inputs.amount.trim() || "0");
  } catch {
    return { success: false, error: "Amount must be a valid integer." };
  }
  if (amountBig <= BigInt(0)) {
    return { success: false, error: "Withdrawal amount must be strictly greater than 0." };
  }

  return {
    success: true,
    data: {
      submitter,
      token,
      destination,
      amount: amountBig,
    },
  };
}

export interface MultiSigConfirmInputs {
  signer: string;
  txId: string;
}

export interface ValidatedMultiSigConfirmParams {
  signer: string;
  tx_id: bigint;
}

export function validateMultiSigConfirm(inputs: MultiSigConfirmInputs): ValidationResult<ValidatedMultiSigConfirmParams> {
  const signer = inputs.signer.trim();
  if (!signer || !isValidStellarAddress(signer)) {
    return { success: false, error: "Signer must be a valid Stellar owner address (G...)." };
  }

  let txIdBig: bigint;
  try {
    txIdBig = BigInt(inputs.txId.trim() || "0");
  } catch {
    return { success: false, error: "Transaction ID must be a valid positive integer." };
  }
  if (txIdBig <= BigInt(0)) {
    return { success: false, error: "Transaction ID must be greater than 0." };
  }

  return {
    success: true,
    data: {
      signer,
      tx_id: txIdBig,
    },
  };
}

// 4. DAO Governance: vote
export interface DaoVoteInputs {
  proposalId: string;
  voter: string;
  support: boolean;
}

export interface ValidatedDaoVoteParams {
  proposal_id: bigint;
  voter: string;
  support: boolean;
}

export function validateDaoVote(inputs: DaoVoteInputs): ValidationResult<ValidatedDaoVoteParams> {
  const voter = inputs.voter.trim();
  if (!voter || !isValidStellarAddress(voter)) {
    return { success: false, error: "Voter must be a valid Stellar address (G...)." };
  }

  let proposalIdBig: bigint;
  try {
    proposalIdBig = BigInt(inputs.proposalId.trim() || "0");
  } catch {
    return { success: false, error: "Proposal ID must be a valid non-negative integer." };
  }
  if (proposalIdBig < BigInt(0)) {
    return { success: false, error: "Proposal ID cannot be negative." };
  }

  return {
    success: true,
    data: {
      proposal_id: proposalIdBig,
      voter,
      support: Boolean(inputs.support),
    },
  };
}

// 5. Subscription Payments: subscribe
export interface SubscribeInputs {
  subscriber: string;
  provider: string;
  token: string;
  amount: string; // stroops
  periodSecs: string;
}

export interface ValidatedSubscribeParams {
  subscriber: string;
  provider: string;
  token: string;
  amount: bigint;
  period: bigint;
}

export function validateSubscribe(inputs: SubscribeInputs): ValidationResult<ValidatedSubscribeParams> {
  const subscriber = inputs.subscriber.trim();
  const provider = inputs.provider.trim();
  const token = inputs.token.trim();

  if (!subscriber || !isValidStellarAddress(subscriber)) {
    return { success: false, error: "Subscriber must be a valid Stellar address (G... or C...)." };
  }
  if (!provider || !isValidStellarAddress(provider)) {
    return { success: false, error: "Provider must be a valid Stellar address (G... or C...)." };
  }
  if (subscriber === provider) {
    return { success: false, error: "Subscriber and provider must be distinct addresses." };
  }
  if (!token || !isValidStellarAddress(token)) {
    return { success: false, error: "Token must be a valid Stellar contract address (C...)." };
  }

  let amountBig: bigint;
  try {
    amountBig = BigInt(inputs.amount.trim() || "0");
  } catch {
    return { success: false, error: "Amount per period must be a valid integer." };
  }
  if (amountBig <= BigInt(0)) {
    return { success: false, error: "Amount per period must be strictly positive." };
  }

  let periodBig: bigint;
  try {
    periodBig = BigInt(inputs.periodSecs.trim() || "0");
  } catch {
    return { success: false, error: "Billing period must be a valid integer of seconds." };
  }
  if (periodBig <= BigInt(0)) {
    return { success: false, error: "Billing period must be strictly greater than 0 seconds." };
  }

  return {
    success: true,
    data: {
      subscriber,
      provider,
      token,
      amount: amountBig,
      period: periodBig,
    },
  };
}

// 6. Marketplace Royalties: set_royalty
export interface SetRoyaltyInputs {
  collection: string;
  recipient: string;
  bps: string; // basis points: 0 - 10000
}

export interface ValidatedSetRoyaltyParams {
  collection: string;
  recipient: string;
  bps: number;
}

export function validateSetRoyalty(inputs: SetRoyaltyInputs): ValidationResult<ValidatedSetRoyaltyParams> {
  const collection = inputs.collection.trim();
  const recipient = inputs.recipient.trim();

  if (!collection || !isValidStellarAddress(collection)) {
    return { success: false, error: "Collection must be a valid 56-character contract address (C...)." };
  }
  if (!recipient || !isValidStellarAddress(recipient)) {
    return { success: false, error: "Royalty recipient must be a valid Stellar address (G... or C...)." };
  }

  const bpsNum = parseInt(inputs.bps.trim() || "-1", 10);
  if (isNaN(bpsNum) || bpsNum < 0 || bpsNum > 10000) {
    return { success: false, error: "Royalty rate must be between 0 and 10,000 basis points (0% to 100%)." };
  }

  return {
    success: true,
    data: {
      collection,
      recipient,
      bps: bpsNum,
    },
  };
}
