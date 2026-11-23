use crate::{events, Escrow, EscrowAsset, EscrowStatus, SorobanForgeEscrowClient};
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::events::Event as _;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env, IntoVal, InvokeError, Vec};

struct Fixture {
    env: Env,
    contract: Address,
    buyer: Address,
    seller: Address,
    old: Address,
    new: Address,
    token: Address,
    basket: bool,
}

impl Fixture {
    fn new(basket: bool) -> Self {
        let env = Env::default();
        env.mock_all_auths();
        let contract = env.register(Escrow, ());
        let buyer = Address::generate(&env);
        let token = env
            .register_stellar_asset_contract_v2(buyer.clone())
            .address();
        StellarAssetClient::new(&env, &token).mint(&buyer, &10_000);
        Self {
            contract,
            buyer,
            seller: Address::generate(&env),
            old: Address::generate(&env),
            new: Address::generate(&env),
            token,
            env,
            basket,
        }
    }

    fn client(&self) -> SorobanForgeEscrowClient<'_> {
        SorobanForgeEscrowClient::new(&self.env, &self.contract)
    }

    fn create(&self, arbiter: &Address) -> u64 {
        let timeout = 604_800;
        if self.basket {
            let assets = Vec::from_array(
                &self.env,
                [EscrowAsset {
                    token: self.token.clone(),
                    amount: 100,
                    released: 0,
                }],
            );
            self.client()
                .create_basket(&self.buyer, &self.seller, arbiter, &assets, &timeout)
        } else {
            self.client().create_escrow(
                &self.buyer,
                &self.seller,
                arbiter,
                &self.token,
                &100,
                &timeout,
            )
        }
    }

    fn deposit(&self, id: u64) {
        if self.basket {
            self.client().deposit_basket(&id);
        } else {
            self.client().deposit(&id);
        }
    }

    fn dispute(&self, id: u64) {
        if self.basket {
            self.client().dispute_basket(&id, &self.buyer);
        } else {
            self.client().dispute(&id, &self.buyer);
        }
    }

    fn replace(
        &self,
        id: u64,
        arbiter: &Address,
    ) -> Result<Result<(), soroban_sdk::ConversionError>, Result<ForgeError, InvokeError>> {
        if self.basket {
            self.client().try_replace_basket_arbiter(&id, arbiter)
        } else {
            self.client().try_replace_arbiter(&id, arbiter)
        }
    }

    fn resolve(
        &self,
        id: u64,
        seller: bool,
    ) -> Result<Result<(), soroban_sdk::ConversionError>, Result<ForgeError, InvokeError>> {
        if self.basket {
            self.client().try_resolve_basket(&id, &seller)
        } else {
            self.client().try_resolve(&id, &seller)
        }
    }

    fn ids(&self, participant: &Address) -> Vec<u64> {
        self.client()
            .escrows_for_participant(participant, &0, &50)
            .ids
    }
}

#[test]
fn replacement_requires_both_signatures_and_preserves_index_order() {
    for basket in [false, true] {
        let t = Fixture::new(basket);
        let id = t.create(&t.old);
        let later = t.create(&t.new);
        let invoke = MockAuthInvoke {
            contract: &t.contract,
            fn_name: if basket {
                "replace_basket_arbiter"
            } else {
                "replace_arbiter"
            },
            args: (id, t.new.clone()).into_val(&t.env),
            sub_invokes: &[],
        };
        let buyer = MockAuth {
            address: &t.buyer,
            invoke: &invoke,
        };
        let seller = MockAuth {
            address: &t.seller,
            invoke: &invoke,
        };
        t.env.mock_auths(core::slice::from_ref(&buyer));
        assert_eq!(t.replace(id, &t.new), Err(Err(InvokeError::Abort)));
        t.env.mock_auths(core::slice::from_ref(&seller));
        assert_eq!(t.replace(id, &t.new), Err(Err(InvokeError::Abort)));
        for invalid in [&t.old, &t.buyer, &t.seller] {
            assert_eq!(t.replace(id, invalid), Err(Ok(ForgeError::InvalidInput)));
        }
        t.env.mock_auths(&[buyer, seller]);
        assert_eq!(t.replace(id, &t.new), Ok(Ok(())));
        assert_eq!(
            t.env
                .events()
                .all()
                .filter_by_contract(&t.contract)
                .events(),
            &[events::ArbiterReplaced {
                escrow_id: id,
                old_arbiter: t.old.clone(),
                new_arbiter: t.new.clone(),
            }
            .to_xdr(&t.env, &t.contract)]
        );
        assert!(t.ids(&t.old).is_empty());
        assert_eq!(t.ids(&t.new), Vec::from_array(&t.env, [id, later]));
        t.env.mock_all_auths();
        t.deposit(id);
        assert_eq!(t.replace(id, &t.old), Ok(Ok(())));
        assert_eq!(t.ids(&t.old), Vec::from_array(&t.env, [id]));
        assert_eq!(t.ids(&t.new), Vec::from_array(&t.env, [later]));
    }
}

#[test]
fn replacement_preserves_pending_release_and_rejects_completed_and_cancelled() {
    for basket in [false, true] {
        let t = Fixture::new(basket);
        let cancelled = t.create(&t.old);
        if basket {
            t.client().cancel_basket(&cancelled);
        } else {
            t.client().cancel(&cancelled);
        }
        assert_eq!(
            t.replace(cancelled, &t.new),
            Err(Ok(ForgeError::InvalidState))
        );

        let id = t.create(&t.old);
        t.deposit(id);
        let end = if basket {
            t.client().request_release_basket(&id)
        } else {
            t.client().request_release(&id, &100)
        };
        let request = t.client().get_release_request(&id);
        assert_eq!(t.replace(id, &t.new), Ok(Ok(())));
        assert_eq!(t.client().get_release_request(&id), request);
        t.env.ledger().set_timestamp(end - 1);
        assert_eq!(
            t.client().try_execute_release(&id),
            Err(Ok(ForgeError::DeadlineReached))
        );
        t.env.ledger().set_timestamp(end);
        t.client().execute_release(&id);
        let status = if basket {
            t.client().get_basket(&id).status
        } else {
            t.client().get_escrow(&id).status
        };
        assert_eq!(status, EscrowStatus::Completed);
        assert_eq!(t.replace(id, &t.old), Err(Ok(ForgeError::InvalidState)));
    }
}

#[test]
fn only_new_arbiter_resolves_later_dispute_and_refunded_cannot_replace() {
    for basket in [false, true] {
        let t = Fixture::new(basket);
        let id = t.create(&t.old);
        t.deposit(id);
        assert_eq!(t.replace(id, &t.new), Ok(Ok(())));
        t.dispute(id);
        assert_eq!(t.replace(id, &t.old), Err(Ok(ForgeError::InvalidState)));
        let invoke = MockAuthInvoke {
            contract: &t.contract,
            fn_name: if basket { "resolve_basket" } else { "resolve" },
            args: (id, false).into_val(&t.env),
            sub_invokes: &[],
        };
        t.env.mock_auths(&[MockAuth {
            address: &t.old,
            invoke: &invoke,
        }]);
        assert_eq!(t.resolve(id, false), Err(Err(InvokeError::Abort)));
        t.env.mock_auths(&[MockAuth {
            address: &t.new,
            invoke: &invoke,
        }]);
        assert_eq!(t.resolve(id, false), Ok(Ok(())));
        assert_eq!(t.replace(id, &t.old), Err(Ok(ForgeError::InvalidState)));
    }
}
