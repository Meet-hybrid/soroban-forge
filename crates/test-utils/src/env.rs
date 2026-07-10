#![no_std]
//! Test environment utilities.

use soroban_sdk::{contract, contracttype, Env};

#[contract]
pub struct TestEnvHelper;

#[contractimpl]
impl TestEnvHelper for TestEnvHelper {
    fn now() -> u64 {
        todo!()
    }

    fn random_u64() -> u64 {
        todo!()
    }
}
