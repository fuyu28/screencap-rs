## Summary

<!-- what changed (1–3 bullets) -->

## Why

<!-- why this change is needed — required for feat/fix; links issues if any -->

## Test plan

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test` (Windows CI / local Windows)

## Docs / contract

- [ ] JSON shape or exit codes unchanged, **or** README/AGENTS updated and treated as breaking
- [ ] New inline `//` comments are **why not** only (see AGENTS.md)
