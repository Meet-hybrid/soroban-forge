# Architecture — Error Handling

All contracts use `soroban-forge-shared-utils::ForgeError`.

Error categories:
1. **Authorization** — caller is not permitted
2. **Validation** — invalid input parameters
3. **State** — object does not exist or is in wrong state
4. **Arithmetic** — overflow / underflow / invalid math
5. **Custom** — domain-specific failures with messages

Avoid generic `Custom` errors for predictable outcomes.
