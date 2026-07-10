use soroban_sdk::{contracttype, Address, Env, Vec};

#[contracttype]
pub struct TestAccounts {
    pub deployer: Address,
    pub user1: Address,
    pub user2: Address,
    pub user3: Address,
    pub validator: Address,
    pub arbiter: Address,
}
