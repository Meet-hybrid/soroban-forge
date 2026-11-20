use soroban_sdk::{contracttype, Address, Bytes, Env, IntoVal, String, TryFromVal, Val, Vec};

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

/// Extract one page from `items`.
///
/// Returns `(page, next_cursor, total)`:
///
/// * `page` — up to `limit` cloned items starting at `offset`.
/// * `next_cursor` — the offset of the following page, or `None` once the
///   final page has been reached (also `None` when `offset >= total` or
///   `limit == 0`, so a caller replaying cursors always terminates).
/// * `total` — the number of items in `items`, independent of `offset` and
///   `limit`.
///
/// Out-of-range input is clamped rather than rejected: an `offset` past the
/// end yields an empty page, and `offset + limit` saturates at the end of the
/// list instead of overflowing. Only the items actually copied are allocated.
///
/// The SDK's [`Vec`] only exposes its element operations for values that can
/// convert to and from the host's [`Val`], so those conversions are part of
/// the bound in addition to `Clone`.
pub fn paginate_slice<T>(
    env: &Env,
    items: &Vec<T>,
    offset: u32,
    limit: u32,
) -> (Vec<T>, Option<u32>, u32)
where
    T: Clone + IntoVal<Env, Val> + TryFromVal<Env, Val>,
{
    let total = items.len();
    let mut page = Vec::new(env);
    // A zero limit (or an offset past the end) returns an empty page and no
    // next cursor: reporting `Some(offset)` here would make a caller that
    // replays cursors spin on the same page forever.
    if limit == 0 || offset >= total {
        return (page, None, total);
    }
    let end = offset.saturating_add(limit).min(total);
    let mut at = offset;
    while at < end {
        page.push_back(items.get_unchecked(at).clone());
        at += 1;
    }
    let next_cursor = if end < total { Some(end) } else { None };
    (page, next_cursor, total)
}

/// A page of results plus the cursor needed to fetch the next page.
///
/// Items are stored as serialized `Bytes` so the helper is agnostic to the
/// concrete value type a contract paginates. Callers decode each item into
/// their domain type. `Debug` is omitted because the SDK collection does not
/// implement it for this contract type.
#[contracttype]
#[derive(Clone)]
pub struct PaginatedResult {
    /// Items on this page.
    pub items: Vec<Bytes>,
    /// Total number of items across all pages.
    pub total: u32,
    /// Cursor describing the next page (offset advanced by `limit`).
    pub cursor: PaginationCursor,
}

#[cfg(test)]
mod tests {
    use super::paginate_slice;
    use soroban_sdk::{Env, Vec};

    fn items(env: &Env, len: u32) -> Vec<u32> {
        let mut values = Vec::new(env);
        let mut at = 0;
        while at < len {
            values.push_back(at);
            at += 1;
        }
        values
    }

    #[test]
    fn empty_vector_yields_empty_page_without_a_next_cursor() {
        let env = Env::default();
        let values: Vec<u32> = Vec::new(&env);

        let (page, next, total) = paginate_slice(&env, &values, 0, 10);

        assert!(page.is_empty());
        assert_eq!(next, None);
        assert_eq!(total, 0);
    }

    #[test]
    fn single_item_page_ends_the_list() {
        let env = Env::default();
        let values = items(&env, 1);

        let (page, next, total) = paginate_slice(&env, &values, 0, 10);

        assert_eq!(page.len(), 1);
        assert_eq!(page.get_unchecked(0), 0);
        assert_eq!(next, None);
        assert_eq!(total, 1);
    }

    #[test]
    fn exact_boundary_split_reports_the_following_offset() {
        let env = Env::default();
        let values = items(&env, 4);

        let (first, next, total) = paginate_slice(&env, &values, 0, 2);
        assert_eq!(first.len(), 2);
        assert_eq!(first.get_unchecked(0), 0);
        assert_eq!(first.get_unchecked(1), 1);
        assert_eq!(next, Some(2));
        assert_eq!(total, 4);

        let (second, next, total) = paginate_slice(&env, &values, 2, 2);
        assert_eq!(second.len(), 2);
        assert_eq!(second.get_unchecked(0), 2);
        assert_eq!(second.get_unchecked(1), 3);
        assert_eq!(next, None);
        assert_eq!(total, 4);
    }

    #[test]
    fn last_partial_page_reports_no_next_cursor() {
        let env = Env::default();
        let values = items(&env, 5);

        let (page, next, total) = paginate_slice(&env, &values, 4, 3);

        assert_eq!(page.len(), 1);
        assert_eq!(page.get_unchecked(0), 4);
        assert_eq!(next, None);
        assert_eq!(total, 5);
    }

    #[test]
    fn zero_limit_returns_an_empty_page_and_never_advances() {
        let env = Env::default();
        let values = items(&env, 3);

        for offset in [0, 1, 3, u32::MAX] {
            let (page, next, total) = paginate_slice(&env, &values, offset, 0);
            assert!(page.is_empty());
            assert_eq!(next, None, "a zero limit must not report a next cursor");
            assert_eq!(total, 3);
        }
    }

    #[test]
    fn offset_at_or_past_the_end_clamps_to_an_empty_page() {
        let env = Env::default();
        let values = items(&env, 3);

        for offset in [3, 4, u32::MAX] {
            let (page, next, total) = paginate_slice(&env, &values, offset, 10);
            assert!(page.is_empty());
            assert_eq!(next, None);
            assert_eq!(total, 3);
        }
    }

    #[test]
    fn offset_plus_limit_saturates_instead_of_overflowing() {
        let env = Env::default();
        let values = items(&env, 5);

        let (page, next, total) = paginate_slice(&env, &values, 2, u32::MAX);

        assert_eq!(page.len(), 3);
        assert_eq!(page.get_unchecked(0), 2);
        assert_eq!(page.get_unchecked(2), 4);
        assert_eq!(next, None);
        assert_eq!(total, 5);
    }

    #[test]
    fn replaying_cursors_visits_every_item_exactly_once() {
        let env = Env::default();
        let values = items(&env, 7);
        let mut seen = Vec::new(&env);
        let mut cursor = 0;

        loop {
            let (page, next, total) = paginate_slice(&env, &values, cursor, 3);
            assert_eq!(total, 7);
            for value in page.iter() {
                seen.push_back(value);
            }
            match next {
                Some(at) => {
                    assert!(at > cursor, "a cursor replay must terminate");
                    cursor = at;
                }
                None => break,
            }
        }

        assert_eq!(seen.len(), 7);
        for expected in 0..7 {
            assert_eq!(seen.get_unchecked(expected), expected);
        }
    }
}
