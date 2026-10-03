# Subscription Payments Contract Reference

- **Crate**: `crates/subscription-payments`
- **Package**: `soroban-forge-subscription-payments`
- **Client**: `SorobanForgeSubscriptionPaymentsClient`
- **Contract Type**: `SubscriptionPayments`
- **Trait**: `SorobanForgeSubscriptionPayments`

---

## Overview

The Subscription Payments contract provides recurring, on-chain billing protocols on Stellar. Subscribers authorize providers to bill fixed amounts per period, or opt into prepaid custody balances. The contract supports metered usage quotas with exact bucketed overage pricing, multi-period catch-up billing, pause and resume capabilities, provider opt-in consent, and automated arrears retries.

### Key Capabilities
- **Dual Billing Modes**:
  - **Pull Mode**: Direct recurring pulls from the subscriber's account via SEP-41 token transfer authorization.
  - **Prepaid Mode**: Funds deposited into contract custody (`deposit`), debited per billing cycle, and refunded upon cancellation or retry exhaustion.
- **Provider Opt-In Protocol**: Explicit subscriber authorization (`authorize_provider`) required before a provider can create subscriptions on the subscriber's behalf (`subscribe_on_behalf_of`).
- **Metered Usage Quotas & Overages**: Allows providers to meter consumption units. The contract derives overage charges based on included units, bucketed rounding, and rate caps.
- **Arrears & Retry Engine**: Failed payments transition subscriptions to `PastDue` and allow up to 3 retry attempts (`MAX_RETRIES`) before automatic cancellation.
- **Multi-Period Catch-Up**: Providers can settle up to 32 accumulated periods in a single atomic transaction (`charge_catchup`).
- **Fair Time Pause/Resume**: Pausing halts charges and usage metering; resuming shifts billing schedules forward by the exact paused duration.

---

## State Machine & Lifecycle

