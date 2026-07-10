# Architecture — Contract Interfaces

Every contract exposes a stable interface using `#[contract]` and `#[contractimpl]`.

Public methods follow the pattern: `verb_noun` (e.g., `create_escrow`, `release_funds`, `cancel_subscription`).

All inputs are validated at the boundary. Errors use the shared `ForgeError` enum.
