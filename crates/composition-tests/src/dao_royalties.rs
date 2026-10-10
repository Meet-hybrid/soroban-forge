use super::*;
use soroban_forge_dao_governance::{
    BondState, DaoGovernance, ProposalState, SorobanForgeDaoGovernanceClient,
};
use soroban_forge_marketplace_royalties::{
    MarketplaceRoyalties, SorobanForgeMarketplaceRoyaltiesClient,
};
use soroban_forge_test_utils::MockTarget;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{vec, Address, Bytes, Env};

const START: u64 = 1_000_000;
const VOTING_PERIOD: u64 = 3_600;
const BOND_AMOUNT: i128 = 1_000;

fn setup_dao_royalties() -> (
    Env,
    Address,
    StellarAssetClient<'static>,
    Address,
    StellarAssetClient<'static>,
    SorobanForgeDaoGovernanceClient<'static>,
    SorobanForgeMarketplaceRoyaltiesClient<'static>,
    soroban_forge_test_utils::TestAccounts,
) {
    let (env, accounts) = setup_env();
    env.mock_all_auths_allowing_non_root_auth();
    env.ledger().set_timestamp(START);

    // Settlement token (used for marketplace sales and DAO bonds)
    let token_admin = Address::generate(&env);
    let sac_token = env.register_stellar_asset_contract_v2(token_admin);
    let token = sac_token.address();
    let token_client = StellarAssetClient::new(&env, &token);

    // Governance token (used for DAO voting weights)
    let gov_admin = Address::generate(&env);
    let sac_gov = env.register_stellar_asset_contract_v2(gov_admin);
    let gov_token = sac_gov.address();
    let gov_client = StellarAssetClient::new(&env, &gov_token);

    let dao_id = env.register(DaoGovernance, ());
    let dao_client = SorobanForgeDaoGovernanceClient::new(&env, &dao_id);

    let royalties_id = env.register(MarketplaceRoyalties, ());
    let royalties_client = SorobanForgeMarketplaceRoyaltiesClient::new(&env, &royalties_id);

    (
        env,
        token,
        token_client,
        gov_token,
        gov_client,
        dao_client,
        royalties_client,
        accounts,
    )
}

#[test]
fn dao_treasury_accumulates_marketplace_royalties_and_governance_bonds() {
    let (env, token, token_client, gov_token, gov_client, dao_client, royalties_client, accounts) =
        setup_dao_royalties();

    let dao_treasury = accounts.arbiter.clone();
    let collection = accounts.user1.clone();
    let payer = accounts.user2.clone();
    let seller = accounts.user3.clone();
    let proposer = accounts.deployer.clone();
    let voter = accounts.validator.clone();

    // 1. Initialize DAO with governance token and configure bond with treasury.
    dao_client.initialize(&gov_token);
    dao_client.configure_bond(&token, &BOND_AMOUNT, &dao_treasury);

    // 2. Configure Marketplace Royalties: collection pays 10% (1_000 bps) to the DAO treasury.
    royalties_client.set_royalty(&collection, &dao_treasury, &1_000);
    assert_eq!(token_client.balance(&dao_treasury), 0);

    // 3. Payer executes a sale on Marketplace for 10_000 tokens.
    token_client.mint(&payer, &20_000);
    let sale_amount = 10_000_i128;
    let settlement =
        royalties_client.settle_sale(&collection, &token, &payer, &seller, &sale_amount);
    assert_eq!(settlement.royalty_share, 1_000);
    assert_eq!(settlement.seller_net, 9_000);
    assert_eq!(token_client.balance(&seller), 9_000);
    assert_eq!(token_client.balance(&dao_treasury), 1_000);

    // 4. Payer settles a batch of 2 sales of 5_000 tokens each.
    let batch_sales = vec![
        &env,
        (seller.clone(), 5_000_i128),
        (seller.clone(), 5_000_i128),
    ];
    let batch_settlements =
        royalties_client.settle_sales(&collection, &token, &payer, &batch_sales);
    assert_eq!(batch_settlements.len(), 2);
    // Each 5_000 sale pays 500 royalty to treasury.
    assert_eq!(token_client.balance(&dao_treasury), 2_000);
    assert_eq!(token_client.balance(&seller), 18_000);

    let summary = royalties_client.get_settlement_summary(&collection);
    assert_eq!(summary.sales, 3);
    assert_eq!(summary.gross_volume, 20_000);
    assert_eq!(summary.royalties_paid, 2_000);

    // 5. In DAO governance: proposer posts a 1_000 token bond to submit a proposal.
    token_client.mint(&proposer, &BOND_AMOUNT);
    let target = env.register(MockTarget, ());
    let action = Bytes::from_slice(&env, &[1, 2, 3]);
    let proposal_id = dao_client.propose(
        &proposer,
        &target,
        &action,
        &VOTING_PERIOD,
        &vec![&env],
        &None,
    );
    assert_eq!(token_client.balance(&proposer), 0);
    assert_eq!(token_client.balance(&dao_client.address), BOND_AMOUNT);

    // 6. Voter with governance tokens votes AGAINST the proposal.
    gov_client.mint(&voter, &500_000);
    dao_client.vote(&proposal_id, &voter, &false);

    // 7. Advance time past voting period and execute finalisation: proposal is defeated.
    env.ledger().set_timestamp(START + VOTING_PERIOD + 1);
    dao_client.execute(&proposal_id);

    let proposal = dao_client.get_proposal(&proposal_id);
    assert_eq!(proposal.state, ProposalState::Defeated);
    assert_eq!(proposal.bond_state, BondState::Forfeited);

    // 8. Forfeited bond is transferred from DAO contract to DAO treasury!
    // Treasury balance = 2_000 (marketplace royalties) + 1_000 (forfeited bond) = 3_000.
    assert_eq!(token_client.balance(&dao_treasury), 3_000);
    assert_eq!(token_client.balance(&dao_client.address), 0);

    // 9. Verify full token conservation:
    // 20_000 (payer) + 1_000 (proposer) = 21_000 total minted.
    // Payer remaining: 0. Seller: 18_000. Treasury: 3_000. Sum = 21_000.
    assert_eq!(
        token_client.balance(&seller) + token_client.balance(&dao_treasury),
        21_000
    );
}

#[test]
fn dao_proposal_execution_interacts_with_marketplace_settlement() {
    let (env, token, token_client, gov_token, gov_client, dao_client, royalties_client, accounts) =
        setup_dao_royalties();

    let dao_treasury = accounts.arbiter.clone();
    let collection = accounts.user1.clone();
    let proposer = accounts.deployer.clone();
    let voter = accounts.validator.clone();

    dao_client.initialize(&gov_token);
    dao_client.configure_bond(&token, &BOND_AMOUNT, &dao_treasury);
    royalties_client.set_royalty(&collection, &dao_treasury, &500);

    // Proposer submits a proposal that targets MockTarget.
    token_client.mint(&proposer, &BOND_AMOUNT);
    let target = env.register(MockTarget, ());
    let action = Bytes::from_slice(&env, &[42]);
    let proposal_id = dao_client.propose(
        &proposer,
        &target,
        &action,
        &VOTING_PERIOD,
        &vec![&env],
        &None,
    );

    // Voter casts vote FOR the proposal.
    gov_client.mint(&voter, &100_000);
    dao_client.vote(&proposal_id, &voter, &true);

    // Advance time and finalize: transitions Active -> Succeeded.
    env.ledger().set_timestamp(START + VOTING_PERIOD + 1);
    dao_client.execute(&proposal_id);
    let proposal = dao_client.get_proposal(&proposal_id);
    assert_eq!(proposal.state, ProposalState::Succeeded);

    // Second execute invocation dispatches the target call and refunds the bond.
    dao_client.execute(&proposal_id);
    let executed_proposal = dao_client.get_proposal(&proposal_id);
    assert_eq!(executed_proposal.state, ProposalState::Executed);
    assert_eq!(executed_proposal.bond_state, BondState::Refunded);

    // Proposer receives bond refund in full.
    assert_eq!(token_client.balance(&proposer), BOND_AMOUNT);
    assert_eq!(token_client.balance(&dao_client.address), 0);

    // Verify keeper touch_ttl on marketplace royalties works cleanly alongside DAO state.
    assert_eq!(royalties_client.touch_ttl(&collection), ());
}
