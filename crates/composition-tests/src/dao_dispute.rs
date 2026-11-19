//! A passed DAO action resolves an escrow dispute in favor of its seller.

use super::*;
use soroban_forge_dao_governance::{
    BondState, DaoGovernance, ProposalState, SorobanForgeDaoGovernanceClient,
};
use soroban_forge_escrow::{Escrow, EscrowStatus, SorobanForgeEscrowClient};
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{contract, contractimpl, contracttype, Address, Bytes, Env};

const START: u64 = 3_000_000;
const ESCROW_AMOUNT: i128 = 500;
const PROPOSAL_BOND: i128 = 100;
const VOTING_DURATION: u64 = 100;
const REVIEW_PERIOD: u64 = 3_600;

#[test]
fn dao_proposal_resolves_disputed_escrow() {
    let (env, accounts) = setup_env();
    env.ledger().set_timestamp(START);

    let token_admin_id = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin_id.clone())
        .address();
    let token_admin = StellarAssetClient::new(&env, &token_id);
    let token = TokenClient::new(&env, &token_id);
    let escrow_id = env.register(Escrow, ());
    let escrow = SorobanForgeEscrowClient::new(&env, &escrow_id);
    let dao_id = env.register(DaoGovernance, ());
    let dao = SorobanForgeDaoGovernanceClient::new(&env, &dao_id);
    let resolver_id = env.register(EscrowDaoResolver, ());
    let resolver = EscrowDaoResolverClient::new(&env, &resolver_id);

    dao.initialize(&token_id);
    dao.configure_bond(&token_id, &PROPOSAL_BOND, &accounts.deployer);
    token_admin.mint(&accounts.user1, &(ESCROW_AMOUNT + PROPOSAL_BOND));
    token_admin.mint(&accounts.validator, &1);

    let holders = [
        accounts.deployer.clone(),
        accounts.user1.clone(),
        accounts.user2.clone(),
        accounts.user3.clone(),
        accounts.validator.clone(),
        accounts.arbiter.clone(),
        token_admin_id,
        token_id.clone(),
        escrow_id.clone(),
        dao_id.clone(),
        resolver_id.clone(),
    ];
    let minted = ESCROW_AMOUNT + PROPOSAL_BOND + 1;
    let initially_minted = [0, ESCROW_AMOUNT + PROPOSAL_BOND, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    assert_balances(&token, minted, &holders, &initially_minted);

    let escrow_number = escrow.create_escrow_with_review(
        &accounts.user1,
        &accounts.user2,
        &dao_id,
        &token_id,
        &ESCROW_AMOUNT,
        &86_400,
        &REVIEW_PERIOD,
    );
    assert_balances(&token, minted, &holders, &initially_minted);

    escrow.deposit(&escrow_number);
    let escrow_funded = [0, PROPOSAL_BOND, 0, 0, 1, 0, 0, 0, ESCROW_AMOUNT, 0, 0];
    assert_eq!(escrow.get_status(&escrow_number), EscrowStatus::Funded);
    assert_balances(&token, minted, &holders, &escrow_funded);

    escrow.request_release(&escrow_number, &ESCROW_AMOUNT);
    assert_balances(&token, minted, &holders, &escrow_funded);

    escrow.dispute(&escrow_number, &accounts.user1);
    assert_eq!(escrow.get_status(&escrow_number), EscrowStatus::Disputed);
    assert_eq!(
        escrow.try_execute_release(&escrow_number),
        Err(Ok(ForgeError::NotFound))
    );
    assert_balances(&token, minted, &holders, &escrow_funded);

    resolver.initialize(&escrow_id, &escrow_number);
    let action = Bytes::from_slice(&env, b"resolve-in-favor-of-seller");
    let proposal_id = dao.propose(
        &accounts.user1,
        &resolver_id,
        &action,
        &VOTING_DURATION,
        &soroban_sdk::Vec::new(&env),
        &None,
    );
    let bond_posted = [0, 0, 0, 0, 1, 0, 0, 0, ESCROW_AMOUNT, PROPOSAL_BOND, 0];
    assert_balances(&token, minted, &holders, &bond_posted);

    dao.vote(&proposal_id, &accounts.validator, &true);
    assert_balances(&token, minted, &holders, &bond_posted);
    env.ledger().set_timestamp(START + VOTING_DURATION - 1);
    assert_eq!(
        dao.try_execute(&proposal_id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_eq!(dao.get_proposal(&proposal_id).state, ProposalState::Active);
    assert_balances(&token, minted, &holders, &bond_posted);

    env.ledger().set_timestamp(START + VOTING_DURATION);
    dao.execute(&proposal_id);
    assert_eq!(
        dao.get_proposal(&proposal_id).state,
        ProposalState::Succeeded
    );
    assert_balances(&token, minted, &holders, &bond_posted);

    dao.execute(&proposal_id);
    let proposal = dao.get_proposal(&proposal_id);
    assert_eq!(proposal.state, ProposalState::Executed);
    assert_eq!(proposal.bond_state, BondState::Refunded);
    assert_eq!(escrow.get_status(&escrow_number), EscrowStatus::Completed);
    let resolved = [0, PROPOSAL_BOND, ESCROW_AMOUNT, 0, 1, 0, 0, 0, 0, 0, 0];
    assert_balances(&token, minted, &holders, &resolved);
}

#[contract]
struct EscrowDaoResolver;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
enum ResolverKey {
    Escrow,
    EscrowId,
}

#[contractimpl]
impl EscrowDaoResolver {
    pub fn initialize(env: Env, escrow: Address, escrow_id: u64) {
        let storage = env.storage().instance();
        storage.set(&ResolverKey::Escrow, &escrow);
        storage.set(&ResolverKey::EscrowId, &escrow_id);
    }

    pub fn execute(env: Env, action: Bytes) -> Result<(), ForgeError> {
        if action != Bytes::from_slice(&env, b"resolve-in-favor-of-seller") {
            return Err(ForgeError::InvalidInput);
        }

        let storage = env.storage().instance();
        let escrow_address: Address = storage.get(&ResolverKey::Escrow).unwrap();
        let escrow_id: u64 = storage.get(&ResolverKey::EscrowId).unwrap();
        SorobanForgeEscrowClient::new(&env, &escrow_address).resolve(&escrow_id, &true);
        Ok(())
    }
}
