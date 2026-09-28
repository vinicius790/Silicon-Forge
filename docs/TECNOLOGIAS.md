# Tecnologias

| Peça | Implementação atual |
|---|---|
| Linguagem | Rust 2021 |
| Build e testes | Cargo; `Cargo.lock` incluído |
| CLI | Parser manual com `std::env::args` |
| Erros da CPU | `Result<_, String>` |
| Erros de montagem | `AsmError` com implementação de `Display` |
| Arquivos e processo | `std::fs`, `std::process` |
| Dependências externas | Nenhuma; seção `[dependencies]` vazia |
| Rede | `std::net` apenas em `debug_api` (`127.0.0.1`) |
| JSON de snapshot | Codificador próprio em `snapshot.rs` (sem serde) |
| Smoke script | Bash, Cargo e o binário compilado |
| Licença | MIT |

Não são usados `clap`, `anyhow`, `thiserror`, SDL ou `unsafe` no código atual.
Rust/Cargo precisam estar instalados antes do build; com a toolchain disponível,
o projeto pode ser compilado e testado offline.
