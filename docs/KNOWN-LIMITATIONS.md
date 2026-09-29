
# Known Limitations

## Escrow Contract

### Expiry and Refunds

1. **Storage Liveness**: While the contract supports permissionless expiry refunds, it does not implement automatic storage cleanup for expired escrows. This is intentionally separate from fund settlement (see issue #93).

2. **Dispute Freeze**: Escrows in dispute cannot be automatically refunded, even after expiry. This prevents potential griefing vectors while disputes are pending resolution.

3. **Boundary Precision**: The exact-at-deadline boundary is enforced to prevent front-running, but keepers should account for potential ledger timestamp variability in their monitoring systems.

4. **No Keeper Rewards**: The current implementation does not include any mechanism for rewarding keepers. This is intentionally out of scope but may be addressed in future work (see issue #244 discussion).

5. **Full-Amount Only**: The permissionless refund moves the entire remaining escrow amount. Partial refunds are not supported in this implementation (see issue #232).
