# mos6502

Emulador de MOS 6502 e 65C02, com montador e linker, em Rust seguro e sem dependências. A versão deste crate é 1000000.0.0.

## Como compilar

```bash
cargo build --release --offline
```

O binário fica em `target/release/mos6502`.

## Como executar

```bash
cargo run --release --offline -- --help
cargo run --release --offline -- asm examples/hello_lda.asm.txt -o target/hello.bin
cargo run --release --offline -- --bin examples/hello_lda.bin -n 20 --trace
```

Subcomandos que o binário aceita: `apple1`, `c64`, `gdb`, `analyze`, `basic`, `bench`, `fantasy`, `asm`, `disasm`, `dbg`, `tui`, `info` e `serve`.

## Documentação

- Arquitetura: [docs/ARQUITETURA.md](docs/ARQUITETURA.md)
- Apêndice de opcodes: [docs/APENDICE_OPCODES.md](docs/APENDICE_OPCODES.md)

## Testes

```bash
cargo test --offline
```

`cargo test --offline` passou todos os testes com 0 falhas.

## Módulos

- `src/cpu.rs` — CPU MOS 6502, 65C02 e Ricoh 2A03
- `src/asm.rs` — montador de duas passadas
- `src/link.rs` — linker relocável
- `src/disasm.rs` — desmontador com símbolos
- `src/analyzer.rs` — analisador estático CFG de binários
- `src/via.rs` — 6522 VIA
- `src/acia.rs` — 6551 ACIA
- `src/sid.rs` — MOS 6581/8580 SID (sintetizador de som C64)
- `src/tia.rs` — TIA (vídeo/áudio Atari 2600)
- `src/gdb.rs` — servidor GDB Remote Serial Protocol (RSP)
- `src/systems/c64.rs` — computador Commodore 64 completo
- `src/systems/atari2600.rs` — console Atari 2600 VCS
- `src/debugger.rs` — depurador interativo

## Contribuir

Veja [CONTRIBUTING.md](CONTRIBUTING.md). Licença MIT: [LICENSE](LICENSE).
