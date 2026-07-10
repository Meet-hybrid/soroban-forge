use soroban_sdk::{contracterror, contracttype, Env, String, Vec};

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum ForgeError {
    Unauthorized = 1,
    NotFound = 2,
    InvalidInput = 3,
    InsufficientFunds = 4,
    AlreadyInitialized = 5,
    NotInitialized = 6,
    DeadlineReached = 7,
    InsufficientAllowance = 8,
    ArithmeticOverflow = 9,
    Custom = 10,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorDetails {
    pub code: u32,
    pub message: String,
}
