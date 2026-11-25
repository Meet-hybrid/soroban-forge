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
    use super::*;
    use soroban_sdk::testutils::storage::Persistent as _;
    use soroban_sdk::testutils::Ledger as _;
    use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

    /// Reusable harness for TTLHelper tests.
    ///
    /// The harness creates a deterministic Soroban contract context, writes
    /// persistent probe keys inside that context, and reads each key's TTL back
    /// through the same contract frame. Reuse it for existing, missing, mixed,
    /// and repeated-key touch scenarios.
    struct TtlHarness {
        env: Env,
        contract: Address,
    }

    #[contract]
    struct TtlProbe;

    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    enum ProbeKey {
        Alpha,
        Beta,
        Gamma,
    }

    #[contractimpl]
    impl TtlProbe {
        pub fn noop() {}
    }

    impl TtlHarness {
        fn setup() -> Self {
            let env = Env::default();
            let contract = env.register(TtlProbe, ());
            Self { env, contract }
        }

        fn write(&self, key: &ProbeKey) {
            self.env.as_contract(&self.contract, || {
                self.env.storage().persistent().set(key, &1_u32);
            });
        }

        fn ttl(&self, key: &ProbeKey) -> u32 {
            self.env.as_contract(&self.contract, || {
                if !self.env.storage().persistent().has(key) {
                    0
                } else {
                    self.env.storage().persistent().get_ttl(key)
                }
            })
        }

        fn helper(&self) -> TTLHelper {
            TTLHelper::new(self.env.storage(), BUMP_AMOUNT)
        }

        fn touch(&self, keys: &[ProbeKey]) -> Result<(), crate::ForgeError> {
            self.env
                .as_contract(&self.contract, || self.helper().touch(&self.env, keys))
        }

        fn advance(&self, ledgers: u32) {
            let next = self
                .env
                .ledger()
                .sequence()
                .checked_add(ledgers)
                .expect("ledger sequence must not overflow");
            self.env.ledger().set_sequence_number(next);
        }
    }

    #[test]
    fn touch_missing_keys_succeeds_and_keeps_ttl_zero() {
        let harness = TtlHarness::setup();
        harness
            .touch(&[ProbeKey::Alpha])
            .expect("missing key touch");
        assert_eq!(harness.ttl(&ProbeKey::Alpha), 0);
    }

    #[test]
    fn touch_existing_key_refreshes_ttl() {
        let harness = TtlHarness::setup();
        harness.write(&ProbeKey::Alpha);
        harness.advance(DAY_IN_LEDGERS);

        let before = harness.ttl(&ProbeKey::Alpha);
        harness
            .touch(&[ProbeKey::Alpha])
            .expect("existing key touch");
        let after = harness.ttl(&ProbeKey::Alpha);

        assert!(before > 0);
        assert!(after > before);
    }

    #[test]
    fn touch_mixed_key_list_refreshes_only_existing_keys() {
        let harness = TtlHarness::setup();
        harness.write(&ProbeKey::Alpha);
        harness.write(&ProbeKey::Beta);
        harness.advance(DAY_IN_LEDGERS);

        let alpha_before = harness.ttl(&ProbeKey::Alpha);
        let beta_before = harness.ttl(&ProbeKey::Beta);
        let gamma_before = harness.ttl(&ProbeKey::Gamma);

        harness
            .touch(&[ProbeKey::Alpha, ProbeKey::Gamma, ProbeKey::Beta])
            .expect("mixed key touch");

        assert!(harness.ttl(&ProbeKey::Alpha) > alpha_before);
        assert!(harness.ttl(&ProbeKey::Beta) > beta_before);
        assert_eq!(harness.ttl(&ProbeKey::Gamma), gamma_before);
        assert_eq!(gamma_before, 0);
    }

    #[test]
    fn touch_multiple_existing_keys_in_one_operation() {
        let harness = TtlHarness::setup();
        harness.write(&ProbeKey::Alpha);
        harness.write(&ProbeKey::Beta);
        harness.write(&ProbeKey::Gamma);
        harness.advance(DAY_IN_LEDGERS);

        let before = [
            harness.ttl(&ProbeKey::Alpha),
            harness.ttl(&ProbeKey::Beta),
            harness.ttl(&ProbeKey::Gamma),
        ];
        harness
            .touch(&[ProbeKey::Alpha, ProbeKey::Beta, ProbeKey::Gamma])
            .expect("multiple key touch");

        let after = [
            harness.ttl(&ProbeKey::Alpha),
            harness.ttl(&ProbeKey::Beta),
            harness.ttl(&ProbeKey::Gamma),
        ];
        for index in 0..before.len() {
            assert!(after[index] > before[index]);
        }
    }

    #[test]
    fn repeated_touch_refreshes_existing_key_again() {
        let harness = TtlHarness::setup();
        harness.write(&ProbeKey::Alpha);
        harness.advance(DAY_IN_LEDGERS);

        harness.touch(&[ProbeKey::Alpha]).expect("first touch");
        let first = harness.ttl(&ProbeKey::Alpha);
        harness.advance(DAY_IN_LEDGERS);
        harness.touch(&[ProbeKey::Alpha]).expect("second touch");
        let second = harness.ttl(&ProbeKey::Alpha);

        assert!(first > 0);
        assert_eq!(second, BUMP_AMOUNT);
    }
}
