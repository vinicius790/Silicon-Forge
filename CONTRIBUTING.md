# Contribuindo

Antes de abrir um patch, rode:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --offline
```

Inclua um teste que falha sem a sua mudança.

Se o comportamento visível mudar, uma linha em [Unreleased] no `CHANGELOG.md`.
