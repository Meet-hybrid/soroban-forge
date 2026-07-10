use soroban_sdk::{contractclient, Address, Env, String, Vec};

#[contractclient(name = "SorobanForgeEscrowClient")]
pub trait SorobanForgeEscrow {
    fn create_escrow(
        env: Env,
        buyer: Address,
        seller: Address,
        arbiter: Address,
        amount: i128,
        timeout: u64,
    ) -> Result<u64, soroban_forge_shared_utils::ForgeError>;

    fn deposit(env: Env, escrow_id: u64) -> Result<(), soroban_forge_shared_utils::ForgeError>;
    fn release(env: Env, escrow_id: u64) -> Result<(), soroban_forge_shared_utils::ForgeError>;
    fn refund(env: Env, escrow_id: u64) -> Result<(), soroban_forge_shared_utils::ForgeError>;
    fn get_status(
        env: Env,
        escrow_id: u64,
    ) -> Result<EscrowStatus, soroban_forge_shared_utils::ForgeError>;
    fn cancel(env: Env, escrow_id: u64) -> Result<(), soroban_forge_shared_utils::ForgeError>;
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EscrowStatus {
    Pending,
    Funded,
    Completed,
    Refunded,
    Disputed,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EscrowData {
    pub escrow_id: u64,
    pub buyer: Address,
    pub seller: Address,
    pub arbiter: Address,
    pub amount: i128,
    pub timeout: u64,
    pub status: EscrowStatus,
    pub created_at: u64,
}
