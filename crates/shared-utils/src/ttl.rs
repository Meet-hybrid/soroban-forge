use soroban_sdk::{Env, IntoVal, Val};

pub const DAY_IN_LEDGERS: u32 = 17_280;
pub const BUMP_AMOUNT: u32 = 30 * DAY_IN_LEDGERS;
pub const BUMP_THRESHOLD: u32 = BUMP_AMOUNT - DAY_IN_LEDGERS;
pub const DEFAULT_TTL: u32 = BUMP_AMOUNT;

pub fn bump_entry<K>(env: &Env, key: &K)
where
    K: IntoVal<Env, Val>,
{
    env.storage()
        .persistent()
        .extend_ttl(key, BUMP_THRESHOLD, BUMP_AMOUNT);
}

pub struct TTLHelper {
    storage: soroban_sdk::storage::Storage,
    min_ttl: u32,
}

impl TTLHelper {
    pub fn new(storage: soroban_sdk::storage::Storage, min_ttl: u32) -> Self {
        Self { storage, min_ttl }
    }

    pub fn bump<K>(&self, key: &K) -> Result<(), crate::ForgeError>
    where
        K: IntoVal<Env, Val>,
    {
        self.storage.persistent().extend_ttl(
            key,
            self.min_ttl.saturating_sub(DAY_IN_LEDGERS),
            self.min_ttl,
        );
        Ok(())
    }

    pub fn touch<K>(&self, _env: &Env, keys: &[K]) -> Result<(), crate::ForgeError>
    where
        K: IntoVal<Env, Val> + Clone,
    {
        for key in keys {
            if self.storage.persistent().has(key) {
                self.bump(key)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::vec::Vec;
    use soroban_sdk::testutils::storage::Persistent as _;
    use soroban_sdk::testutils::Ledger as _;
    use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

    use super::*;

    #[contract]
    struct TtlProbe;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    #[contracttype]
    enum ProbeKey {
        One,
        Two,
        Three,
        Missing,
    }

    #[contractimpl]
    impl TtlProbe {
        fn seed(env: Env, key: ProbeKey) {
            env.storage().persistent().set(&key, &42_u32);
            env.storage()
                .persistent()
                .extend_ttl(&key, BUMP_THRESHOLD, BUMP_AMOUNT);
        }
    }

    fn advance_ledger(env: &Env, amount: u32) {
        let next = env.ledger().sequence().saturating_add(amount);
        env.ledger()
            .with_mut(|ledger| ledger.sequence_number = next);
    }

    fn seed_entry(env: &Env, contract: &Address, key: &ProbeKey) {
        env.as_contract(contract, || TtlProbe::seed(env.clone(), *key));
    }

    fn assert_ttl_refreshed(env: &Env, contract: &Address, key: &ProbeKey, expected: u32) {
        env.as_contract(contract, || {
            assert_eq!(env.storage().persistent().get_ttl(key), expected);
            assert!(env.storage().persistent().has(key));
        });
    }

    fn assert_refresh_harness(env: &Env, contract: &Address, keys: &[ProbeKey]) {
        env.as_contract(contract, || {
            let helper = TTLHelper::new(env.storage(), BUMP_AMOUNT);
            let before = keys
                .iter()
                .map(|key| {
                    if env.storage().persistent().has(key) {
                        env.storage().persistent().get_ttl(key)
                    } else {
                        0
                    }
                })
                .collect::<Vec<_>>();

            helper.touch(env, keys).unwrap();

            for (key, ttl_before) in keys.iter().zip(before) {
                let ttl_after = if env.storage().persistent().has(key) {
                    env.storage().persistent().get_ttl(key)
                } else {
                    0
                };
                if env.storage().persistent().has(key) {
                    assert!(ttl_after >= ttl_before, "TTL should not shrink after touch");
                    assert_eq!(
                        ttl_after, BUMP_AMOUNT,
                        "existing key TTL should be refreshed"
                    );
                } else {
                    assert_eq!(ttl_after, 0, "missing key should remain absent");
                }
            }
        });
    }

    #[test]
    fn ttl_helper_touch_ignores_missing_keys() {
        let env = Env::default();
        let contract = env.register(TtlProbe, ());

        let existing = ProbeKey::One;
        let missing = ProbeKey::Missing;
        seed_entry(&env, &contract, &existing);
        advance_ledger(&env, BUMP_THRESHOLD);

        env.as_contract(&contract, || {
            let helper = TTLHelper::new(env.storage(), BUMP_AMOUNT);
            let ttl_before = env.storage().persistent().get_ttl(&existing);
            assert!(ttl_before <= BUMP_THRESHOLD);

            helper.touch(&env, &[missing, existing]).unwrap();
        });

        assert_ttl_refreshed(&env, &contract, &existing, BUMP_AMOUNT);
        env.as_contract(&contract, || {
            assert!(!env.storage().persistent().has(&missing));
        });
    }

    #[test]
    fn ttl_helper_touch_refreshes_multiple_existing_keys_in_one_call() {
        let env = Env::default();
        let contract = env.register(TtlProbe, ());

        let first = ProbeKey::One;
        let second = ProbeKey::Two;
        seed_entry(&env, &contract, &first);
        seed_entry(&env, &contract, &second);
        advance_ledger(&env, BUMP_THRESHOLD);

        env.as_contract(&contract, || {
            let helper = TTLHelper::new(env.storage(), BUMP_AMOUNT);
            helper.touch(&env, &[first, second]).unwrap();
        });

        assert_ttl_refreshed(&env, &contract, &first, BUMP_AMOUNT);
        assert_ttl_refreshed(&env, &contract, &second, BUMP_AMOUNT);
    }

    #[test]
    fn ttl_helper_touch_refreshes_mixed_key_lists() {
        let env = Env::default();
        let contract = env.register(TtlProbe, ());

        let existing = ProbeKey::One;
        let also_existing = ProbeKey::Two;
        let missing = ProbeKey::Missing;
        seed_entry(&env, &contract, &existing);
        seed_entry(&env, &contract, &also_existing);
        advance_ledger(&env, BUMP_THRESHOLD);

        assert_refresh_harness(
            &env,
            &contract,
            &[missing, existing, also_existing, ProbeKey::Three],
        );

        assert_ttl_refreshed(&env, &contract, &existing, BUMP_AMOUNT);
        assert_ttl_refreshed(&env, &contract, &also_existing, BUMP_AMOUNT);
        env.as_contract(&contract, || {
            assert!(!env.storage().persistent().has(&missing));
            assert!(!env.storage().persistent().has(&ProbeKey::Three));
        });
    }
}
