//! DAO governance + Marketplace royalties composition settlement scenarios.
//!
//! Exercises cross-contract flows between [`DaoGovernance`] and
//! [`MarketplaceRoyalties`] backed by a real SEP-41 [`TokenFixture`] and
//! verified against an independent [`BalanceMirror`]:
//! - Bonded governance proposals (`propose`), balance-weighted voting (`vote`),
//!   two-step finalisation/execution (`execute`), and cross-contract dispatch of
//!   single-recipient, multi-recipient split, and batch marketplace royalty
//!   settlements (`settle_sale`, `distribute`, `settle_sales`).
//! - Proposal dependency chains (`requires`) gating royalty settlement behind
//!   on-chain royalty configuration.
//! - Defeated proposals forfeiting bonds to the DAO treasury without invoking
//!   the downstream royalty contract.
//! - Cross-contract authorization enforcement (`env.auths()` and negative
//!   `mock_auths` / `set_auths`).
//! - Downstream royalty errors (`NotFound`, `TokenTransferFailed`,
//!   `InvalidInput`) rolling back execution while preserving DAO `Succeeded`
//!   state, bond custody, and token balances for subsequent retry.
//! - Deterministic multi-seed conservation runs across [`DEFAULT_COMPOSITION_SEEDS`].

use super::*;
use soroban_forge_dao_governance::{
    BondState, DaoGovernance, ProposalState, SorobanForgeDaoGovernanceClient,
};
use soroban_forge_marketplace_royalties::{
    MarketplaceRoyalties, RoyaltyShare, SorobanForgeMarketplaceRoyaltiesClient,
};
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::{Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::xdr::{FromXdr, ToXdr};
use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Bytes, Env, IntoVal, InvokeError, Vec,
};

/// Typed governance payload decoded by [`RoyaltyGovernanceDispatcher`] when
/// [`DaoGovernance::execute`] dispatches `target.execute(action)`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RoyaltyGovernanceAction {
    /// Configure `(royalties_id, collection, recipient, bps)`.
    SetRoyalty(Address, Address, Address, u32),
    /// Settle a single sale `(royalties_id, collection, token, payer, seller, amount)`.
    SettleSale(Address, Address, Address, Address, Address, i128),
    /// Distribute standalone royalty share `(royalties_id, collection, token, payer, seller, amount)`.
    Distribute(Address, Address, Address, Address, Address, i128),
    /// Settle a batch of sales `(royalties_id, collection, token, payer, sales)`.
    SettleSalesBatch(Address, Address, Address, Address, Vec<(Address, i128)>),
}

/// Adapter target contract bridging [`DaoGovernance`]'s `execute(Bytes)` ABI
/// to typed [`MarketplaceRoyalties`] entrypoints.
#[contract]
pub struct RoyaltyGovernanceDispatcher;

#[contractimpl]
impl RoyaltyGovernanceDispatcher {
    /// Decode an XDR-encoded [`RoyaltyGovernanceAction`] and invoke the
    /// target [`MarketplaceRoyalties`] contract.
    pub fn execute(env: Env, action: Bytes) -> Result<(), ForgeError> {
        let decoded = RoyaltyGovernanceAction::from_xdr(&env, &action)
            .map_err(|_| ForgeError::InvalidInput)?;

        match decoded {
            RoyaltyGovernanceAction::SetRoyalty(royalties_id, collection, recipient, bps) => {
                let client = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &royalties_id);
                match client.try_set_royalty(&collection, &recipient, &bps) {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(err)) => Err(err),
                    Err(_) => Err(ForgeError::ContractInvocationFailed),
                }
            }
            RoyaltyGovernanceAction::SettleSale(
                royalties_id,
                collection,
                token,
                payer,
                seller,
                amount,
            ) => {
                let client = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &royalties_id);
                match client.try_settle_sale(&collection, &token, &payer, &seller, &amount) {
                    Ok(Ok(_settlement)) => Ok(()),
                    Ok(Err(err)) => Err(err),
                    Err(_) => Err(ForgeError::ContractInvocationFailed),
                }
            }
            RoyaltyGovernanceAction::Distribute(
                royalties_id,
                collection,
                token,
                payer,
                seller,
                amount,
            ) => {
                let client = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &royalties_id);
                match client.try_distribute(&collection, &token, &payer, &seller, &amount) {
                    Ok(Ok(_net)) => Ok(()),
                    Ok(Err(err)) => Err(err),
                    Err(_) => Err(ForgeError::ContractInvocationFailed),
                }
            }
            RoyaltyGovernanceAction::SettleSalesBatch(
                royalties_id,
                collection,
                token,
                payer,
                sales,
            ) => {
                let client = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &royalties_id);
                match client.try_settle_sales(&collection, &token, &payer, &sales) {
                    Ok(Ok(_settlements)) => Ok(()),
                    Ok(Err(err)) => Err(err),
                    Err(_) => Err(ForgeError::ContractInvocationFailed),
                }
            }
        }
    }
}

/// Reusable setup fixture for DAO Governance + Marketplace Royalties tests.
pub struct DaoRoyaltiesFixture {
    pub env: Env,
    pub accounts: TestAccounts,
    pub token: TokenFixture,
    pub dao_id: Address,
    pub royalties_id: Address,
    pub dispatcher_id: Address,
    pub treasury: Address,
    pub mirror: BalanceMirror,
}

impl DaoRoyaltiesFixture {
    /// Deploy and initialize DAO governance, marketplace royalties, dispatcher,
    /// SEP-41 token, and independent [`BalanceMirror`].
    pub fn new(bond_amount: i128) -> Self {
        let (env, accounts, token) = setup_env();
        let dao_id = env.register(DaoGovernance, ());
        let royalties_id = env.register(MarketplaceRoyalties, ());
        let dispatcher_id = env.register(RoyaltyGovernanceDispatcher, ());
        let treasury = accounts.arbiter.clone();

        let dao = SorobanForgeDaoGovernanceClient::new(&env, &dao_id);
        dao.initialize(&token.address);
        dao.configure_bond(&token.address, &bond_amount, &treasury);

        let participants = [
            accounts.deployer.clone(),
            accounts.user1.clone(),
            accounts.user2.clone(),
            accounts.user3.clone(),
            accounts.validator.clone(),
            accounts.arbiter.clone(),
            dao_id.clone(),
            royalties_id.clone(),
            dispatcher_id.clone(),
        ];
        let mirror = BalanceMirror::new(&env, &token.address, &participants);

        Self {
            env,
            accounts,
            token,
            dao_id,
            royalties_id,
            dispatcher_id,
            treasury,
            mirror,
        }
    }

    pub fn dao(&self) -> SorobanForgeDaoGovernanceClient<'_> {
        SorobanForgeDaoGovernanceClient::new(&self.env, &self.dao_id)
    }

    pub fn royalties(&self) -> SorobanForgeMarketplaceRoyaltiesClient<'_> {
        SorobanForgeMarketplaceRoyaltiesClient::new(&self.env, &self.royalties_id)
    }

    pub fn encode_action(&self, action: &RoyaltyGovernanceAction) -> Bytes {
        action.clone().to_xdr(&self.env)
    }
}

#[test]
fn dao_proposal_dispatches_single_and_split_royalty_settlements() {
    let bond_amount = 500_i128;
    let mut fx = DaoRoyaltiesFixture::new(bond_amount);
    let dao = fx.dao();
    let royalties = fx.royalties();

    let proposer = fx.accounts.user1.clone();
    let voter_2 = fx.accounts.user2.clone();
    let payer = fx.accounts.validator.clone();
    let seller = fx.accounts.user3.clone();
    let creator_1 = fx.accounts.deployer.clone();
    let creator_2 = fx.accounts.user2.clone();
    let collection = fx.accounts.user1.clone();

    // Mint governance/settlement tokens to proposer, voter_2, and payer.
    fx.token.mint(&fx.env, &proposer, 3_000);
    fx.mirror.record_mint(&proposer, 3_000);
    fx.token.mint(&fx.env, &voter_2, 2_000);
    fx.mirror.record_mint(&voter_2, 2_000);
    fx.token.mint(&fx.env, &payer, 10_000);
    fx.mirror.record_mint(&payer, 10_000);
    fx.mirror.assert_conserved(&fx.env);

    // Configure a 2-recipient royalty split on the collection:
    // 500 bps (5%) to creator_1 and 250 bps (2.5%) to creator_2 = 750 bps (7.5%).
    let mut splits = Vec::new(&fx.env);
    splits.push_back(RoyaltyShare {
        recipient: creator_1.clone(),
        bps: 500,
    });
    splits.push_back(RoyaltyShare {
        recipient: creator_2.clone(),
        bps: 250,
    });
    royalties.set_royalty_splits(&collection, &splits);
    assert_contract_event(&fx.env, &fx.royalties_id, "royalty_configured");
    assert_auth_invocation(&fx.env, &collection, &fx.royalties_id, "set_royalty_splits");

    // Proposer submits a governance proposal to settle a 4,000-token sale.
    let sale_amount = 4_000_i128;
    let action = fx.encode_action(&RoyaltyGovernanceAction::SettleSale(
        fx.royalties_id.clone(),
        collection.clone(),
        fx.token.address.clone(),
        payer.clone(),
        seller.clone(),
        sale_amount,
    ));
    let duration = 1_800_u64;
    let proposal_id = dao.propose(
        &proposer,
        &fx.dispatcher_id,
        &action,
        &duration,
        &Vec::new(&fx.env),
        &None,
    );
    assert_contract_event(&fx.env, &fx.dao_id, "proposed");
    assert_contract_event(&fx.env, &fx.dao_id, "bond_posted");
    assert_sac_transfer_event(
        &fx.env,
        &fx.token.address,
        &proposer,
        &fx.dao_id,
        bond_amount,
    );
    assert_auth_invocation(&fx.env, &proposer, &fx.dao_id, "propose");
    fx.mirror
        .record_transfer(&proposer, &fx.dao_id, bond_amount);
    fx.mirror.assert_conserved(&fx.env);

    // Cast weighted votes (proposer has 2,500 remaining, voter_2 has 2,000).
    dao.vote(&proposal_id, &proposer, &true);
    assert_contract_event(&fx.env, &fx.dao_id, "vote_cast");
    assert_auth_invocation(&fx.env, &proposer, &fx.dao_id, "vote");

    dao.vote(&proposal_id, &voter_2, &true);
    assert_contract_event(&fx.env, &fx.dao_id, "vote_cast");
    let prop = dao.get_proposal(&proposal_id);
    assert_eq!(prop.for_votes, 4_500);
    assert_eq!(prop.against_votes, 0);

    // Advance past voting window; first `execute` finalizes Active -> Succeeded.
    let now = fx.env.ledger().timestamp();
    fx.env.ledger().set_timestamp(now + duration + 1);
    dao.execute(&proposal_id);
    assert_contract_event(&fx.env, &fx.dao_id, "finalised");
    assert_eq!(
        dao.get_proposal(&proposal_id).state,
        ProposalState::Succeeded
    );
    fx.mirror.assert_conserved(&fx.env);

    // Second `execute` dispatches RoyaltyGovernanceDispatcher -> MarketplaceRoyalties::settle_sale
    // and refunds the proposal bond to `proposer`.
    dao.execute(&proposal_id);
    assert_contract_event(&fx.env, &fx.royalties_id, "sale_settled");
    assert_contract_event(&fx.env, &fx.dao_id, "finalised");
    assert_contract_event(&fx.env, &fx.dao_id, "bond_released");

    // 4,000 sale at 750 bps:
    // creator_1 gets 200 (5%), creator_2 gets 100 (2.5%), seller gets 3,700 (92.5%).
    fx.mirror.record_transfer(&payer, &seller, 3_700);
    fx.mirror.record_transfer(&payer, &creator_1, 200);
    fx.mirror.record_transfer(&payer, &creator_2, 100);
    // Proposal bond refunded from DAO to proposer.
    fx.mirror
        .record_transfer(&fx.dao_id, &proposer, bond_amount);
    fx.mirror.assert_conserved(&fx.env);

    let final_prop = dao.get_proposal(&proposal_id);
    assert_eq!(final_prop.state, ProposalState::Executed);
    assert_eq!(final_prop.bond_state, BondState::Refunded);

    let summary = royalties.get_settlement_summary(&collection);
    assert_eq!(summary.sales_count, 1);
    assert_eq!(summary.total_volume, 4_000);
    assert_eq!(summary.total_royalties, 300);
}

#[test]
fn dao_dependency_chain_configures_royalty_then_settles_batch_sales() {
    let bond_amount = 300_i128;
    let mut fx = DaoRoyaltiesFixture::new(bond_amount);
    let dao = fx.dao();
    let royalties = fx.royalties();

    let proposer = fx.accounts.user1.clone();
    let payer = fx.accounts.validator.clone();
    let seller_a = fx.accounts.user2.clone();
    let seller_b = fx.accounts.user3.clone();
    let creator = fx.accounts.deployer.clone();
    // Use `dispatcher_id` as the collection so the dispatcher contract itself
    // is the collection authorizer when `SetRoyalty` executes.
    let collection = fx.dispatcher_id.clone();

    fx.token.mint(&fx.env, &proposer, 4_000);
    fx.mirror.record_mint(&proposer, 4_000);
    fx.token.mint(&fx.env, &payer, 8_000);
    fx.mirror.record_mint(&payer, 8_000);
    fx.mirror.assert_conserved(&fx.env);

    let duration = 1_200_u64;

    // Proposal 1: Configure 1,000 bps (10%) royalty on `collection` for `creator`.
    let config_action = fx.encode_action(&RoyaltyGovernanceAction::SetRoyalty(
        fx.royalties_id.clone(),
        collection.clone(),
        creator.clone(),
        1_000,
    ));
    let prop_1 = dao.propose(
        &proposer,
        &fx.dispatcher_id,
        &config_action,
        &duration,
        &Vec::new(&fx.env),
        &None,
    );
    fx.mirror
        .record_transfer(&proposer, &fx.dao_id, bond_amount);

    // Proposal 2: Batch settle two sales `(seller_a, 2_000)` and `(seller_b, 3_000)`,
    // declaring an explicit dependency on `prop_1`.
    let mut batch_sales = Vec::new(&fx.env);
    batch_sales.push_back((seller_a.clone(), 2_000_i128));
    batch_sales.push_back((seller_b.clone(), 3_000_i128));
    let batch_action = fx.encode_action(&RoyaltyGovernanceAction::SettleSalesBatch(
        fx.royalties_id.clone(),
        collection.clone(),
        fx.token.address.clone(),
        payer.clone(),
        batch_sales,
    ));
    let requires = soroban_sdk::vec![&fx.env, prop_1];
    let prop_2 = dao.propose(
        &proposer,
        &fx.dispatcher_id,
        &batch_action,
        &duration,
        &requires,
        &None,
    );
    fx.mirror
        .record_transfer(&proposer, &fx.dao_id, bond_amount);
    fx.mirror.assert_conserved(&fx.env);

    dao.vote(&prop_1, &proposer, &true);
    dao.vote(&prop_2, &proposer, &true);

    let now = fx.env.ledger().timestamp();
    fx.env.ledger().set_timestamp(now + duration + 1);

    // Finalize both proposals to `Succeeded`.
    dao.execute(&prop_1);
    dao.execute(&prop_2);
    assert_eq!(dao.get_proposal(&prop_1).state, ProposalState::Succeeded);
    assert_eq!(dao.get_proposal(&prop_2).state, ProposalState::Succeeded);

    // Attempting to execute `prop_2` before its prerequisite `prop_1` is rejected.
    let snap = fx.mirror.snapshot();
    let dep_err = dao.try_execute(&prop_2).unwrap_err().unwrap();
    assert_eq!(dep_err, ForgeError::InvalidInput);
    assert_eq!(dao.get_proposal(&prop_2).state, ProposalState::Succeeded);
    fx.mirror.assert_unchanged(&fx.env, &snap);

    // Execute `prop_1` first (configures royalty + refunds bond 1).
    dao.execute(&prop_1);
    assert_contract_event(&fx.env, &fx.royalties_id, "royalty_configured");
    assert_eq!(dao.get_proposal(&prop_1).state, ProposalState::Executed);
    fx.mirror
        .record_transfer(&fx.dao_id, &proposer, bond_amount);
    fx.mirror.assert_conserved(&fx.env);

    // Now execute `prop_2` (batch settles both sales + refunds bond 2).
    dao.execute(&prop_2);
    assert_eq!(
        contract_event_count(&fx.env, &fx.royalties_id, "sale_settled"),
        2
    );
    assert_eq!(dao.get_proposal(&prop_2).state, ProposalState::Executed);

    // Sale 1 (2,000 @ 10%): 1,800 to seller_a, 200 to creator.
    // Sale 2 (3,000 @ 10%): 2,700 to seller_b, 300 to creator.
    fx.mirror.record_transfer(&payer, &seller_a, 1_800);
    fx.mirror.record_transfer(&payer, &creator, 200);
    fx.mirror.record_transfer(&payer, &seller_b, 2_700);
    fx.mirror.record_transfer(&payer, &creator, 300);
    fx.mirror
        .record_transfer(&fx.dao_id, &proposer, bond_amount);
    fx.mirror.assert_conserved(&fx.env);

    let summary = royalties.get_settlement_summary(&collection);
    assert_eq!(summary.sales_count, 2);
    assert_eq!(summary.total_volume, 5_000);
    assert_eq!(summary.total_royalties, 500);
}

#[test]
fn defeated_proposal_forfeits_bond_to_treasury_without_invoking_royalties() {
    let bond_amount = 400_i128;
    let mut fx = DaoRoyaltiesFixture::new(bond_amount);
    let dao = fx.dao();
    let royalties = fx.royalties();

    let proposer = fx.accounts.user1.clone();
    let opponent = fx.accounts.user2.clone();
    let payer = fx.accounts.validator.clone();
    let seller = fx.accounts.user3.clone();
    let collection = fx.accounts.deployer.clone();

    fx.token.mint(&fx.env, &proposer, 1_500);
    fx.mirror.record_mint(&proposer, 1_500);
    fx.token.mint(&fx.env, &opponent, 3_000);
    fx.mirror.record_mint(&opponent, 3_000);
    fx.token.mint(&fx.env, &payer, 5_000);
    fx.mirror.record_mint(&payer, 5_000);

    royalties.set_royalty(&collection, &fx.accounts.deployer, &500);

    let action = fx.encode_action(&RoyaltyGovernanceAction::SettleSale(
        fx.royalties_id.clone(),
        collection.clone(),
        fx.token.address.clone(),
        payer,
        seller,
        2_000,
    ));
    let duration = 900_u64;
    let proposal_id = dao.propose(
        &proposer,
        &fx.dispatcher_id,
        &action,
        &duration,
        &Vec::new(&fx.env),
        &None,
    );
    fx.mirror
        .record_transfer(&proposer, &fx.dao_id, bond_amount);
    fx.mirror.assert_conserved(&fx.env);

    dao.vote(&proposal_id, &proposer, &true);
    dao.vote(&proposal_id, &opponent, &false);

    let now = fx.env.ledger().timestamp();
    fx.env.ledger().set_timestamp(now + duration + 1);

    dao.execute(&proposal_id);
    assert_contract_event(&fx.env, &fx.dao_id, "finalised");
    assert_contract_event(&fx.env, &fx.dao_id, "bond_released");
    assert_no_contract_events(&fx.env, &fx.royalties_id);

    let defeated = dao.get_proposal(&proposal_id);
    assert_eq!(defeated.state, ProposalState::Defeated);
    assert_eq!(defeated.bond_state, BondState::Forfeited);
    assert!(royalties.try_get_settlement_summary(&collection).is_err());

    fx.mirror
        .record_transfer(&fx.dao_id, &fx.treasury, bond_amount);
    fx.mirror.assert_conserved(&fx.env);
}

#[test]
fn cross_contract_authorization_enforced_across_dao_and_royalties() {
    let bond_amount = 250_i128;
    let mut fx = DaoRoyaltiesFixture::new(bond_amount);
    let dao = fx.dao();
    let royalties = fx.royalties();

    let proposer = fx.accounts.user1.clone();
    let collection = fx.accounts.deployer.clone();

    fx.token.mint(&fx.env, &proposer, 2_000);
    fx.mirror.record_mint(&proposer, 2_000);
    fx.mirror.assert_conserved(&fx.env);

    // 1. Enforce-mode check: non-collection signer cannot configure royalties.
    let snap = fx.mirror.snapshot();
    fx.env.mock_auths(&[MockAuth {
        address: &proposer,
        invoke: &MockAuthInvoke {
            contract: &fx.royalties_id,
            fn_name: "set_royalty",
            args: (&collection, &proposer, 500_u32).into_val(&fx.env),
            sub_invokes: &[],
        },
    }]);
    let res = royalties.try_set_royalty(&collection, &proposer, &500);
    assert!(matches!(res, Err(Err(InvokeError::Abort))));
    fx.env.mock_all_auths();
    assert!(royalties.try_get_royalty(&collection).is_err());
    fx.mirror.assert_unchanged(&fx.env, &snap);

    // 2. Blank auth envelope rejects `dao.propose` without moving bond tokens.
    let action = fx.encode_action(&RoyaltyGovernanceAction::SetRoyalty(
        fx.royalties_id.clone(),
        collection.clone(),
        proposer.clone(),
        500,
    ));
    fx.env.set_auths(&[]);
    let res = dao.try_propose(
        &proposer,
        &fx.dispatcher_id,
        &action,
        &600,
        &Vec::new(&fx.env),
        &None,
    );
    assert!(matches!(res, Err(Err(InvokeError::Abort))));
    fx.env.mock_all_auths();
    fx.mirror.assert_unchanged(&fx.env, &snap);

    // 3. Create proposal under valid auth; verify zero-balance voter and
    //    duplicate voter are rejected.
    let proposal_id = dao.propose(
        &proposer,
        &fx.dispatcher_id,
        &action,
        &600,
        &Vec::new(&fx.env),
        &None,
    );
    fx.mirror
        .record_transfer(&proposer, &fx.dao_id, bond_amount);
    fx.mirror.assert_conserved(&fx.env);

    let zero_voter_err = dao
        .try_vote(&proposal_id, &fx.accounts.user3, &true)
        .unwrap_err()
        .unwrap();
    assert_eq!(zero_voter_err, ForgeError::InvalidInput);

    dao.vote(&proposal_id, &proposer, &true);
    let dup_vote_err = dao
        .try_vote(&proposal_id, &proposer, &true)
        .unwrap_err()
        .unwrap();
    assert_eq!(dup_vote_err, ForgeError::InvalidInput);
    fx.mirror.assert_conserved(&fx.env);
}

#[test]
fn downstream_royalty_failure_rolls_back_dao_execution_and_preserves_bond() {
    let bond_amount = 500_i128;
    let mut fx = DaoRoyaltiesFixture::new(bond_amount);
    let dao = fx.dao();
    let royalties = fx.royalties();

    let proposer = fx.accounts.user1.clone();
    let payer = fx.accounts.validator.clone();
    let seller = fx.accounts.user3.clone();
    let creator = fx.accounts.deployer.clone();
    let collection = fx.accounts.user2.clone();

    // Fund proposer with 2,000 tokens, but give `payer` only 1,000 initially
    // while proposing a 3,000-token sale.
    fx.token.mint(&fx.env, &proposer, 2_000);
    fx.mirror.record_mint(&proposer, 2_000);
    fx.token.mint(&fx.env, &payer, 1_000);
    fx.mirror.record_mint(&payer, 1_000);
    fx.mirror.assert_conserved(&fx.env);

    let sale_amount = 3_000_i128;
    let action = fx.encode_action(&RoyaltyGovernanceAction::SettleSale(
        fx.royalties_id.clone(),
        collection.clone(),
        fx.token.address.clone(),
        payer.clone(),
        seller.clone(),
        sale_amount,
    ));
    let duration = 600_u64;
    let proposal_id = dao.propose(
        &proposer,
        &fx.dispatcher_id,
        &action,
        &duration,
        &Vec::new(&fx.env),
        &None,
    );
    fx.mirror
        .record_transfer(&proposer, &fx.dao_id, bond_amount);
    dao.vote(&proposal_id, &proposer, &true);

    let now = fx.env.ledger().timestamp();
    fx.env.ledger().set_timestamp(now + duration + 1);
    dao.execute(&proposal_id);
    assert_eq!(
        dao.get_proposal(&proposal_id).state,
        ProposalState::Succeeded
    );

    // Failure 1: Collection has no royalty config yet (`ForgeError::NotFound`).
    let snap = fx.mirror.snapshot();
    let err_unconfigured = dao.try_execute(&proposal_id).unwrap_err().unwrap();
    assert_eq!(err_unconfigured, ForgeError::ContractInvocationFailed);
    let prop_after_fail_1 = dao.get_proposal(&proposal_id);
    assert_eq!(prop_after_fail_1.state, ProposalState::Succeeded);
    assert_eq!(prop_after_fail_1.bond_state, BondState::Posted);
    fx.mirror.assert_unchanged(&fx.env, &snap);

    // Configure 1,000 bps (10%) royalty on `collection`, then retry while `payer`
    // still has only 1,000 tokens (< 3,000 required).
    royalties.set_royalty(&collection, &creator, &1_000);
    let snap = fx.mirror.snapshot();
    let err_underfunded = dao.try_execute(&proposal_id).unwrap_err().unwrap();
    assert_eq!(err_underfunded, ForgeError::ContractInvocationFailed);
    let prop_after_fail_2 = dao.get_proposal(&proposal_id);
    assert_eq!(prop_after_fail_2.state, ProposalState::Succeeded);
    assert_eq!(prop_after_fail_2.bond_state, BondState::Posted);
    assert!(royalties.try_get_settlement_summary(&collection).is_err());
    fx.mirror.assert_unchanged(&fx.env, &snap);

    // Top up `payer` with 2,500 more tokens (now 3,500 >= 3,000) and retry `dao.execute`.
    fx.token.mint(&fx.env, &payer, 2_500);
    fx.mirror.record_mint(&payer, 2_500);
    dao.execute(&proposal_id);

    assert_contract_event(&fx.env, &fx.royalties_id, "sale_settled");
    assert_contract_event(&fx.env, &fx.dao_id, "bond_released");
    let prop_final = dao.get_proposal(&proposal_id);
    assert_eq!(prop_final.state, ProposalState::Executed);
    assert_eq!(prop_final.bond_state, BondState::Refunded);

    // 3,000 sale @ 10%: 2,700 to seller, 300 to creator; 500 bond refunded to proposer.
    fx.mirror.record_transfer(&payer, &seller, 2_700);
    fx.mirror.record_transfer(&payer, &creator, 300);
    fx.mirror
        .record_transfer(&fx.dao_id, &proposer, bond_amount);
    fx.mirror.assert_conserved(&fx.env);
}

#[test]
fn dao_royalties_conservation_holds_across_deterministic_seeds() {
    for seed in DEFAULT_COMPOSITION_SEEDS {
        let mut rng = SeededRng::new(seed);
        let bond_amount = rng.range_i128(100, 1_000);
        let mut fx = DaoRoyaltiesFixture::new(bond_amount);
        let dao = fx.dao();
        let royalties = fx.royalties();

        let proposer = fx.accounts.user1.clone();
        let voter_2 = fx.accounts.user2.clone();
        let payer = fx.accounts.validator.clone();
        let seller = fx.accounts.user3.clone();
        let creator = fx.accounts.deployer.clone();
        let collection = fx.accounts.user1.clone();

        let proposer_mint = rng.range_i128(bond_amount + 500, 10_000);
        let voter_2_mint = rng.range_i128(500, 10_000);
        let sale_amount = rng.range_i128(1_000, 20_000);
        let bps = rng.range_u32(100, 2_500);
        let duration = rng.range_u64(300, 3_600);
        let voter_2_supports = rng.choose_bool();

        fx.token.mint(&fx.env, &proposer, proposer_mint);
        fx.mirror.record_mint(&proposer, proposer_mint);
        fx.token.mint(&fx.env, &voter_2, voter_2_mint);
        fx.mirror.record_mint(&voter_2, voter_2_mint);
        fx.token.mint(&fx.env, &payer, sale_amount + 1_000);
        fx.mirror.record_mint(&payer, sale_amount + 1_000);
        fx.mirror.assert_conserved(&fx.env);

        royalties.set_royalty(&collection, &creator, &bps);

        let use_distribute_only = rng.choose_bool();
        let action = if use_distribute_only {
            fx.encode_action(&RoyaltyGovernanceAction::Distribute(
                fx.royalties_id.clone(),
                collection.clone(),
                fx.token.address.clone(),
                payer.clone(),
                seller.clone(),
                sale_amount,
            ))
        } else {
            fx.encode_action(&RoyaltyGovernanceAction::SettleSale(
                fx.royalties_id.clone(),
                collection.clone(),
                fx.token.address.clone(),
                payer.clone(),
                seller.clone(),
                sale_amount,
            ))
        };

        let proposal_id = dao.propose(
            &proposer,
            &fx.dispatcher_id,
            &action,
            &duration,
            &Vec::new(&fx.env),
            &None,
        );
        fx.mirror
            .record_transfer(&proposer, &fx.dao_id, bond_amount);
        fx.mirror.assert_conserved(&fx.env);

        dao.vote(&proposal_id, &proposer, &true);
        dao.vote(&proposal_id, &voter_2, &voter_2_supports);

        let prop = dao.get_proposal(&proposal_id);
        let passes = prop.for_votes > prop.against_votes;

        let now = fx.env.ledger().timestamp();
        fx.env.ledger().set_timestamp(now + duration + 1);

        dao.execute(&proposal_id);
        if passes {
            assert_eq!(
                dao.get_proposal(&proposal_id).state,
                ProposalState::Succeeded
            );
            dao.execute(&proposal_id);
            assert_eq!(
                dao.get_proposal(&proposal_id).state,
                ProposalState::Executed
            );

            let royalty_share = (sale_amount * i128::from(bps)) / 10_000;
            let seller_net = sale_amount - royalty_share;
            if !use_distribute_only {
                fx.mirror.record_transfer(&payer, &seller, seller_net);
            }
            fx.mirror.record_transfer(&payer, &creator, royalty_share);
            fx.mirror
                .record_transfer(&fx.dao_id, &proposer, bond_amount);
        } else {
            assert_eq!(
                dao.get_proposal(&proposal_id).state,
                ProposalState::Defeated
            );
            fx.mirror
                .record_transfer(&fx.dao_id, &fx.treasury, bond_amount);
        }

        fx.mirror.assert_conserved(&fx.env);
    }
}
