use super::*;

#[contractevent]
pub struct Charged {
    #[topic]
    pub subscription_id: u64,
    /// Amount actually transferred for the settled period: the derived
    /// `base + overage` amount, not the base alone.
    pub amount: i128,
    pub last_charged: u64,
    pub next_charge_at: u64,
}

/// Usage metered for one metric in the open period. Indexers can rebuild
/// every settled period's overage from these plus the plan's quotas.
#[contractevent]
pub struct UsageRecorded {
    #[topic]
    pub subscription_id: u64,
    #[topic]
    pub metric: Symbol,
    /// Units added by this call.
    pub units: u64,
    /// Running total for the open period, including this call.
    pub period_units: u64,
    /// The billing window these units belong to.
    pub period_start: u64,
}

/// The subscription's declared quota list was replaced.
#[contractevent]
pub struct QuotasSet {
    #[topic]
    pub subscription_id: u64,
    pub quotas: Vec<MetricQuota>,
}

#[contractevent]
pub struct Cancelled {
    #[topic]
    pub subscription_id: u64,
    pub subscriber: Address,
}

/// Emitted when funds are deposited into a subscription's prepaid balance.
#[contractevent]
pub struct Deposited {
    #[topic]
    pub subscription_id: u64,
    pub subscriber: Address,
    pub amount: i128,
    pub new_balance: i128,
}

/// Emitted when a subscription charge is debited from prepaid balance.
#[contractevent]
pub struct BalanceDebited {
    #[topic]
    pub subscription_id: u64,
    pub amount: i128,
    pub remaining_balance: i128,
}

/// Emitted when remaining prepaid balance is refunded to subscriber on cancel.
#[contractevent]
pub struct BalanceRefunded {
    #[topic]
    pub subscription_id: u64,
    pub subscriber: Address,
    pub amount: i128,
}

pub fn charged(env: &Env, subscription: &Subscription, amount: i128) {
    let next_charge_at = subscription
        .last_charged
        .saturating_add(subscription.period);
    Charged {
        subscription_id: subscription.subscription_id,
        amount,
        last_charged: subscription.last_charged,
        next_charge_at,
    }
    .publish(env);
}

pub fn usage_recorded(env: &Env, record: &UsageRecord, units: u64) {
    UsageRecorded {
        subscription_id: record.subscription_id,
        metric: record.metric.clone(),
        units,
        period_units: record.units,
        period_start: record.period_start,
    }
    .publish(env);
}

pub fn quotas_set(env: &Env, subscription: &Subscription) {
    QuotasSet {
        subscription_id: subscription.subscription_id,
        quotas: subscription.quotas.clone(),
    }
    .publish(env);
}

pub fn cancelled(env: &Env, subscription: &Subscription) {
    Cancelled {
        subscription_id: subscription.subscription_id,
        subscriber: subscription.subscriber.clone(),
    }
    .publish(env);
}

pub fn deposited(
    env: &Env,
    subscription_id: u64,
    subscriber: &Address,
    amount: i128,
    new_balance: i128,
) {
    Deposited {
        subscription_id,
        subscriber: subscriber.clone(),
        amount,
        new_balance,
    }
    .publish(env);
}

pub fn balance_debited(env: &Env, subscription_id: u64, amount: i128, remaining_balance: i128) {
    BalanceDebited {
        subscription_id,
        amount,
        remaining_balance,
    }
    .publish(env);
}

pub fn balance_refunded(env: &Env, subscription_id: u64, subscriber: &Address, amount: i128) {
    BalanceRefunded {
        subscription_id,
        subscriber: subscriber.clone(),
        amount,
    }
    .publish(env);
}
