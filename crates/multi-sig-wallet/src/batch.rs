use super::*;
use proptest::{prelude::*, test_runner::RngSeed};
use soroban_forge_test_utils::MockTarget;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::xdr::{ContractEvent, ToXdr};
use std::vec::Vec as List;

struct Wallet {
    env: Env,
    contract: Address,
    owners: [Address; 3],
    target: Address,
}

impl Wallet {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        let contract = env.register(MultiSigWallet, ());
        let owners = core::array::from_fn(|_| Address::generate(&env));
        let target = env.register(MockTarget, ());
        let w = Self {
            env,
            contract,
            owners,
            target,
        };
        w.client()
            .initialize(&Vec::from_array(&w.env, w.owners.clone()), &2);
        w
    }

    fn client(&self) -> SorobanForgeMultiSigWalletClient<'_> {
        SorobanForgeMultiSigWalletClient::new(&self.env, &self.contract)
    }

    fn submit(&self, expiry: Option<u64>) -> u64 {
        self.client().submit(
            &self.owners[0],
            &self.target,
            &Bytes::from_slice(&self.env, &[1]),
            &expiry,
        )
    }

    fn approvals(&self, id: u64) -> List<u8> {
        self.client()
            .get_confirmations(&id)
            .to_xdr(&self.env)
            .iter()
            .collect()
    }

    fn events(&self) -> List<ContractEvent> {
        self.env.events().all().events().to_vec()
    }

    fn eligible(&self, id: u64, signer: &Address) -> bool {
        let c = self.client();
        let Ok(Ok(tx)) = c.try_get_tx(&id) else {
            return false;
        };
        c.is_owner(signer)
            && tx.status == TxStatus::Pending
            && tx.expiry.is_none_or(|t| t > self.env.ledger().timestamp())
            && !tx.confirmations.contains(signer)
            && !tx.rejections.contains(signer)
    }

    fn rotate(&self) {
        let c = self.client();
        if c.is_paused() || !c.is_owner(&self.owners[2]) {
            return;
        }
        let id = c.remove_owner(&self.owners[0], &self.owners[2]);
        c.confirm(&id, &self.owners[0]);
        c.confirm(&id, &self.owners[1]);
        c.execute(&id);
    }
}

#[test]
fn invalid_at_each_position_is_atomic_and_size_is_bounded() {
    let w = Wallet::new();
    let c = w.client();
    let ids: List<_> = (0..=MAX_CONFIRM_BATCH).map(|_| w.submit(None)).collect();
    let signer = &w.owners[1];
    for bad in [[999, 1, 2], [1, 999, 2], [1, 2, 999]] {
        assert_eq!(
            c.try_confirm_batch(&Vec::from_array(&w.env, bad), signer),
            Err(Ok(ForgeError::NotFound))
        );
        assert!(w.events().is_empty());
        for id in &ids {
            assert!(c.get_confirmations(id).is_empty());
        }
    }
    for bad in [&[][..], &[1, 1], ids.as_slice()] {
        assert_eq!(
            c.try_confirm_batch(&Vec::from_slice(&w.env, bad), signer),
            Err(Ok(ForgeError::InvalidInput))
        );
        assert!(w.events().is_empty());
    }
    c.confirm_batch(
        &Vec::from_slice(&w.env, &ids[..MAX_CONFIRM_BATCH as usize]),
        &w.owners[2],
    );
    assert_eq!(w.events().len(), MAX_CONFIRM_BATCH as usize);
    assert_eq!(w.env.auths().len(), 1);
    w.rotate();
    for id in ids {
        assert!(c.get_confirmations(&id).is_empty());
    }
}

#[test]
fn paused_batch_approves_withdrawal_but_execution_still_obeys_the_limit() {
    let w = Wallet::new();
    let c = w.client();
    let signers = Vec::from_slice(&w.env, &w.owners[..2]);
    let token = w
        .env
        .register_stellar_asset_contract_v2(w.owners[0].clone())
        .address();
    let withdrawal = c.submit_withdrawal(&w.owners[0], &token, &w.owners[1], &10);
    let limit = c.set_withdrawal_limit(&w.owners[0], &token, &5, &60);
    let ids = Vec::from_array(&w.env, [withdrawal, limit]);
    c.pause_wallet(&signers);
    for signer in signers.iter() {
        c.confirm_batch(&ids, &signer);
    }
    assert_eq!(
        c.try_execute(&withdrawal),
        Err(Ok(ForgeError::InvalidInput))
    );
    c.resume_wallet(&signers);
    c.execute(&limit);
    assert_eq!(
        c.try_execute(&withdrawal),
        Err(Ok(ForgeError::WithdrawalLimitExceeded))
    );
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, rng_seed: RngSeed::Fixed(705), ..ProptestConfig::default() })]
    #[test]
    fn batches_match_sequential_confirmations(steps in prop::collection::vec((0u8..10, 0u8..12, 0u8..12, 0usize..3), 8..28)) {
        check_sequence(&steps);
    }
}

fn check_sequence(steps: &[(u8, u8, u8, usize)]) {
    let batch = Wallet::new();
    let single = Wallet::new();
    for w in [&batch, &single] {
        w.submit(None);
        w.submit(Some(4));
    }
    for &(op, x, y, owner) in steps {
        let count = batch.client().get_tx_count();
        let id = 1 + u64::from(x) % count;
        let other = 1 + u64::from(y) % (count + 1);
        if op <= 2 {
            let ids = [id, other];
            let expected = id != other
                && ids
                    .iter()
                    .all(|&n| single.eligible(n, &single.owners[owner]));
            let result = batch
                .client()
                .try_confirm_batch(&Vec::from_array(&batch.env, ids), &batch.owners[owner]);
            assert_eq!(result.is_ok(), expected);
            let actual = batch.events();
            let mut events = List::new();
            if expected {
                for tx in ids {
                    single.client().confirm(&tx, &single.owners[owner]);
                    events.extend(single.events());
                }
            }
            assert_eq!(actual, events);
        } else {
            for w in [&batch, &single] {
                let c = w.client();
                let signer = &w.owners[owner];
                match op {
                    3 => {
                        w.submit(Some(w.env.ledger().timestamp() + 4));
                    }
                    4 => {
                        let _ = c.try_confirm(&id, signer);
                    }
                    5 => {
                        let _ = c.try_reject(&id, signer);
                    }
                    6 => {
                        let _ = c.try_cancel_tx(&id, signer);
                    }
                    7 => w.rotate(),
                    8 => {
                        let approvers = Vec::from_slice(&w.env, &w.owners[..2]);
                        if c.is_paused() {
                            c.resume_wallet(&approvers);
                        } else {
                            c.pause_wallet(&approvers);
                        }
                    }
                    _ => w.env.ledger().set_timestamp(w.env.ledger().timestamp() + 4),
                }
            }
        }
        for n in 1..=batch.client().get_tx_count() {
            assert_eq!(batch.approvals(n), single.approvals(n));
            assert_eq!(
                batch.client().get_tx_preview(&n),
                single.client().get_tx_preview(&n)
            );
        }
        if op <= 2 {
            assert_eq!(
                batch.client().try_execute(&id),
                single.client().try_execute(&id)
            );
        }
    }
}
