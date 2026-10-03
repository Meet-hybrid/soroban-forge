# Subscription Payments Contract Reference

- **Crate**: `soroban-forge-subscription-payments`
- **Wasm Target**: `wasm32v1-none`
- **Rust Client**: `SorobanForgeSubscriptionPaymentsClient`
- **TypeScript Client**: `@soroban-forge/subscription-payments-client` (`Client`)

---

## 1. Overview & Architectural Role

The Subscription Payments contract provides recurring token billing with support for:
1. **Pull & Prepaid Billing Modes**: Direct pull transfers from subscriber balances or automated deductions from contract-custodied prepaid deposits.
2. **Provider Opt-In Authorization**: Cryptographic consent mechanism enabling authorized providers to establish subscriptions on behalf of subscribers.
3. **Metered Usage & Quotas**: Tiered overage pricing supporting inclusions, integer bucket units, and optional maximum unit caps.
4. **Arrears & Retries**: Automated status degradation to `PastDue` upon billing failure, followed by cancellation after 3 consecutive failures.
5. **Multi-Period Catch-Up**: Atomic processing of missed billing intervals up to 32 periods.

---

## 2. API Reference

### Subscription Management

#### `subscribe`
Subscribes an account directly. Requires subscriber signature.
