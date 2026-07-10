# WASM Optimization

Target < 100KB per contract.

Techniques:
- Use `opt-level = "z"` in release profile.
- Enable LTO and single codegen unit.
- Strip symbols.
- Prefer `BTreeMap` over `HashMap` when order matters and sizes are small.
- Avoid large `Vec` allocations; pre-allocate exact sizes.
- Use `Cow<'_, str>` or `SmallVec<[u8; 32]>` for small strings.
- Profile with `wasm-opt` before release.
