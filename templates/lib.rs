#![no_std]

use soroban_sdk::{contract, contractimpl, Env};

mod contract;
mod types;
mod errors;

pub use contract::Contract;
pub use types::*;
pub use errors::*;

#[contract]
pub struct Contract;

#[contractimpl]
impl Contract for Contract {
    fn example_method(env: Env, input: Option<i128>) -> Result<i128, ForgeError> {
        todo!()
    }
}
