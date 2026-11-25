
use soroban_forge_shared_utils::ForgeError;
use soroban_sdk::{contracttype, Address, Env};

use crate::Settlement;

/// A durable record describing one successfully completed marketplace sale.
///
/// The settlement ID is supplied by the caller and uniquely identifies
/// the receipt within this contract.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettlementReceipt {
    /// Collection whose royalty configuration was applied.
    pub collection: Address,
    /// Token used to settle the sale.
    pub token: Address,
    /// Account that funded the settlement.
    pub payer: Address,
    /// Seller who received the net proceeds.
    pub seller: Address,
    /// Original sale amount before royalty deductions.
    pub gross_amount: i128,
    /// Amounts paid to the seller and royalty recipients.
    pub settlement: Settlement,
    /// Ledger sequence at which the receipt was recorded.
    pub created_at_ledger: u32,
}

/// Persistent storage keys for settlement receipts.
#[contracttype]
#[derive(Clone)]
enum ReceiptKey {
    Settlement(u64),
}

/// Store a receipt for a completed settlement.
///
/// Existing IDs are rejected to prevent accidentally overwriting
/// a previously recorded receipt.
pub(crate) fn record_receipt(
    env: &Env,
    settlement_id: u64,
    mut receipt: SettlementReceipt,
) -> Result<(), ForgeError> {
    let storage = env.storage().persistent();
    let key = ReceiptKey::Settlement(settlement_id);

    if storage.has(&key) {
        return Err(ForgeError::InvalidInput);
    }

    receipt.created_at_ledger = env.ledger().sequence();
    storage.set(&key, &receipt);

    Ok(())
}

/// Retrieve a previously recorded settlement receipt.
///
/// Returns `None` when the supplied settlement ID does not exist.
pub(crate) fn get_receipt(
    env: &Env,
    settlement_id: u64,
) -> Option<SettlementReceipt> {
    env.storage()
        .persistent()
        .get(&ReceiptKey::Settlement(settlement_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Settlement;

    #[test]
    fn records_and_retrieves_settlement_receipt() {
        let env = Env::default();

        let receipt = SettlementReceipt {
            collection: Address::generate(&env),
            token: Address::generate(&env),
            payer: Address::generate(&env),
            seller: Address::generate(&env),
            gross_amount: 1_000,
            settlement: Settlement {
                royalty_share: 50,
                seller_net: 950,
            },
            created_at_ledger: 0,
        };

        record_receipt(&env, 1, receipt.clone())
            .expect("receipt should be recorded");

        let stored = get_receipt(&env, 1)
            .expect("receipt should be retrievable");

        assert_eq!(stored.collection, receipt.collection);
        assert_eq!(stored.token, receipt.token);
        assert_eq!(stored.payer, receipt.payer);
        assert_eq!(stored.seller, receipt.seller);
        assert_eq!(stored.gross_amount, 1_000);
        assert_eq!(stored.settlement.royalty_share, 50);
        assert_eq!(stored.settlement.seller_net, 950);
        assert_eq!(stored.created_at_ledger, env.ledger().sequence());
    }

    #[test]
    fn rejects_duplicate_settlement_ids() {
        let env = Env::default();

        let receipt = SettlementReceipt {
            collection: Address::generate(&env),
            token: Address::generate(&env),
            payer: Address::generate(&env),
            seller: Address::generate(&env),
            gross_amount: 500,
            settlement: Settlement {
                royalty_share: 25,
                seller_net: 475,
            },
            created_at_ledger: 0,
        };

        record_receipt(&env, 10, receipt.clone())
            .expect("first receipt should be recorded");

        let duplicate = record_receipt(&env, 10, receipt);

        assert_eq!(duplicate, Err(ForgeError::InvalidInput));
    }

    #[test]
    fn returns_none_for_unknown_settlement_id() {
        let env = Env::default();

        assert!(get_receipt(&env, 999).is_none());
    }
}
