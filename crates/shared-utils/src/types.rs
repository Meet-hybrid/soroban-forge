use soroban_sdk::{contracttype, Address, Env, Timestamp, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimeBounds {
    pub start: Timestamp,
    pub end: Timestamp,
}

impl TimeBounds {
    pub fn is_active(&self, env: &Env, now: Timestamp) -> bool {
        now >= self.start && now <= self.end
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Party {
    pub address: Address,
    pub role: String,
    pub approved: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaginationCursor {
    pub offset: u32,
    pub limit: u32,
}

impl PaginationCursor {
    pub fn new(offset: u32, limit: u32) -> Self {
        Self { offset, limit }
    }
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PaginatedResult<T> {
    pub items: Vec<T>,
    pub total: u32,
    pub cursor: PaginationCursor,
}
