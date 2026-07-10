use soroban_sdk::{contracttype, Env, Vec};

#[contracttype]
#[derive(Clone, Debug)]
pub struct StorageEntry<T> {
    pub value: T,
    pub updated_at: u64,
    pub updated_by: Vec<u8>,
}
