//! Multi-sig wallet execution creates and funds a real escrow.

use super::*;
use soroban_forge_escrow::{Escrow, EscrowStatus, SorobanForgeEscrowClient};
use soroban_forge_multi_sig_wallet::{MultiSigWallet, SorobanForgeMultiSigWalletClient};
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{contract, contractimpl, contracttype, Address, Bytes, Env, IntoVal, Symbol};

const START: u64 = 1_000_000;
const AMOUNT: i128 = 500;
const TIMEOUT: u64 = 86_400;
const REVIEW_PERIOD: u64 = 3_600;

#[test]
fn threshold_approved_wallet_action_funds_and_releases_escrow() {
    let (env, accounts) = setup_env();
    env.ledger().set_timestamp(START);

    let token_admin_id = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin_id.clone())
        .address();
    let token_admin = StellarAssetClient::new(&env, &token_id);
    let token = TokenClient::new(&env, &token_id);
    let wallet_id = env.register(MultiSigWallet, ());
    let wallet = SorobanForgeMultiSigWalletClient::new(&env, &wallet_id);
    let escrow_id = env.register(Escrow, ());
    let escrow = SorobanForgeEscrowClient::new(&env, &escrow_id);
    let adapter_id = env.register(EscrowFundingAdapter, ());
    let adapter = EscrowFundingAdapterClient::new(&env, &adapter_id);

    let owners = soroban_sdk::vec![
        &env,
        accounts.user1.clone(),
        accounts.user2.clone(),
        accounts.user3.clone()
    ];
    wallet.initialize(&owners, &2);
    adapter.initialize(
        &wallet_id,
        &escrow_id,
        &accounts.validator,
        &accounts.arbiter,
        &token_id,
        &AMOUNT,
        &TIMEOUT,
        &REVIEW_PERIOD,
    );
    token_admin.mint(&wallet_id, &AMOUNT);

    let holders = [
        accounts.deployer.clone(),
        accounts.user1.clone(),
        accounts.user2.clone(),
        accounts.user3.clone(),
        accounts.validator.clone(),
        accounts.arbiter.clone(),
        token_admin_id,
        token_id,
        wallet_id,
        escrow_id,
        adapter_id.clone(),
    ];
    let wallet_funded = [0, 0, 0, 0, 0, 0, 0, 0, AMOUNT, 0, 0];
    assert_balances(&token, AMOUNT, &holders, &wallet_funded);

    let payload = Bytes::from_slice(&env, b"fund-escrow");
    let args = soroban_sdk::vec![&env, payload.into_val(&env)];
    let tx_id = wallet.submit_call(
        &accounts.user1,
        &adapter_id,
        &Symbol::new(&env, "execute"),
        &args,
        &None,
    );
    assert_balances(&token, AMOUNT, &holders, &wallet_funded);

    wallet.confirm(&tx_id, &accounts.user1);
    assert_balances(&token, AMOUNT, &holders, &wallet_funded);
    assert_eq!(
        wallet.try_execute(&tx_id).unwrap_err().unwrap(),
        ForgeError::InvalidInput
    );
    assert_balances(&token, AMOUNT, &holders, &wallet_funded);

    wallet.confirm(&tx_id, &accounts.user2);
    assert_balances(&token, AMOUNT, &holders, &wallet_funded);
    wallet.execute(&tx_id);
    let created_escrow_id = adapter.escrow_id().unwrap();
    assert_eq!(created_escrow_id, 1);
    assert_eq!(escrow.get_status(&created_escrow_id), EscrowStatus::Funded);
    let escrow_funded = [0, 0, 0, 0, 0, 0, 0, 0, 0, AMOUNT, 0];
    assert_balances(&token, AMOUNT, &holders, &escrow_funded);

    let release_at = escrow.request_release(&created_escrow_id, &AMOUNT);
    assert_eq!(release_at, START + REVIEW_PERIOD);
    assert_balances(&token, AMOUNT, &holders, &escrow_funded);
    env.ledger().set_timestamp(release_at - 1);
    assert_eq!(
        escrow.try_execute_release(&created_escrow_id),
        Err(Ok(ForgeError::DeadlineReached))
    );
    assert_balances(&token, AMOUNT, &holders, &escrow_funded);

    env.ledger().set_timestamp(release_at);
    escrow.execute_release(&created_escrow_id);
    assert_eq!(
        escrow.get_status(&created_escrow_id),
        EscrowStatus::Completed
    );
    let seller_paid = [0, 0, 0, 0, AMOUNT, 0, 0, 0, 0, 0, 0];
    assert_balances(&token, AMOUNT, &holders, &seller_paid);
}

#[contract]
struct EscrowFundingAdapter;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
enum FundingKey {
    Buyer,
    Escrow,
    Seller,
    Arbiter,
    Token,
    Amount,
    Timeout,
    ReviewPeriod,
    EscrowId,
}

#[contractimpl]
impl EscrowFundingAdapter {
    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
        env: Env,
        buyer: Address,
        escrow: Address,
        seller: Address,
        arbiter: Address,
        token: Address,
        amount: i128,
        timeout: u64,
        review_period: u64,
    ) {
        let storage = env.storage().instance();
        storage.set(&FundingKey::Buyer, &buyer);
        storage.set(&FundingKey::Escrow, &escrow);
        storage.set(&FundingKey::Seller, &seller);
        storage.set(&FundingKey::Arbiter, &arbiter);
        storage.set(&FundingKey::Token, &token);
        storage.set(&FundingKey::Amount, &amount);
        storage.set(&FundingKey::Timeout, &timeout);
        storage.set(&FundingKey::ReviewPeriod, &review_period);
    }

    pub fn execute(env: Env, payload: Bytes) -> Result<(), ForgeError> {
        if payload != Bytes::from_slice(&env, b"fund-escrow") {
            return Err(ForgeError::InvalidInput);
        }

        let storage = env.storage().instance();
        let escrow_address: Address = storage.get(&FundingKey::Escrow).unwrap();
        let buyer: Address = storage.get(&FundingKey::Buyer).unwrap();
        let seller: Address = storage.get(&FundingKey::Seller).unwrap();
        let arbiter: Address = storage.get(&FundingKey::Arbiter).unwrap();
        let token: Address = storage.get(&FundingKey::Token).unwrap();
        let amount: i128 = storage.get(&FundingKey::Amount).unwrap();
        let timeout: u64 = storage.get(&FundingKey::Timeout).unwrap();
        let review_period: u64 = storage.get(&FundingKey::ReviewPeriod).unwrap();

        let escrow = SorobanForgeEscrowClient::new(&env, &escrow_address);
        let escrow_id = escrow.create_escrow_with_review(
            &buyer,
            &seller,
            &arbiter,
            &token,
            &amount,
            &timeout,
            &review_period,
        );
        escrow.deposit(&escrow_id);
        storage.set(&FundingKey::EscrowId, &escrow_id);
        Ok(())
    }

    pub fn escrow_id(env: Env) -> Option<u64> {
        env.storage().instance().get(&FundingKey::EscrowId)
    }
}
