use soroban_sdk::{contracttype, Address, Env, IntoVal, InvokeError, Symbol, Val, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DepositPreflightStatus {
    Sufficient,
    InsufficientBalance,
    InsufficientAllowance,
    Unavailable,
    InvalidInput,
    UnknownEscrow,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DepositPreflight {
    pub status: DepositPreflightStatus,
    pub escrow_id: u64,
    pub token: Option<Address>,
    pub buyer: Option<Address>,
    pub required_amount: i128,
    pub buyer_balance: Option<i128>,
    pub allowance: Option<i128>,
}

pub fn read_balance(env: &Env, token: &Address, buyer: &Address) -> Option<i128> {
    let args: Vec<Val> = soroban_sdk::vec![env, buyer.clone().into_val(env)];
    match env.try_invoke_contract::<i128, InvokeError>(token, &Symbol::new(env, "balance"), args) {
        Ok(Ok(balance)) => Some(balance),
        Ok(Err(_)) | Err(_) => None,
    }
}
