use soroban_sdk::{contracttype, Address, String, Vec};

/// Inclusive time window expressed as Unix timestamps (seconds).
///
/// Stored as plain `u64` because `soroban_sdk` models time as `u64`; a
/// dedicated newtype would add conversions without benefit.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimeBounds {
    /// Earliest moment (inclusive) at which the window is active.
    pub start: u64,
    /// Latest moment (inclusive) at which the window is active.
    pub end: u64,
}

impl TimeBounds {
    /// Returns `true` when `now` falls within `[start, end]`.
    pub fn is_active(&self, now: u64) -> bool {
        now >= self.start && now <= self.end
    }
}

/// A participant in a multi-party flow (escrow, governance, ...).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Party {
    /// On-chain address of the participant.
    pub address: Address,
    /// Human-readable role label, e.g. `"buyer"` or `"arbiter"`.
    pub role: String,
    /// Whether this party has granted approval for the current action.
    pub approved: bool,
}

/// Offset/limit pair for paginated reads.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaginationCursor {
    /// Number of items to skip from the start of the full result set.
    pub offset: u32,
    /// Maximum number of items to return.
    pub limit: u32,
}

impl PaginationCursor {
    /// Creates a new cursor from `offset` and `limit`.
    pub fn new(offset: u32, limit: u32) -> Self {
        Self { offset, limit }
    }
}

/// A page of results plus the cursor needed to fetch the next page.
///
/// Items are stored as a `soroban_sdk::Vec<Val>` so the helper is agnostic to
/// the concrete value type a contract paginates; callers re-interpret each
/// `Val` into their domain type. `Debug` is omitted because `Val` and
/// `Vec<Val>` do not implement it.
#[contracttype]
#[derive(Clone)]
pub struct PaginatedResult {
    /// Items on this page.
    pub items: Vec<soroban_sdk::Val>,
    /// Total number of items across all pages.
    pub total: u32,
    /// Cursor describing the next page (offset advanced by `limit`).
    pub cursor: PaginationCursor,
}
