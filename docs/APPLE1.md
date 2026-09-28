# Apple-1

Modelo da placa Apple-1: RAM em `$0000..=$7FFF`, PIA 6821 do crate em
`$D010..=$D013` (teclado na porta A com strobe em CA1; vídeo na porta B, PB7 =
ocupado) e um **monitor original do projeto** em `$FE00..=$FFFF`, montado em tempo
de execução a partir de `firmware/apple1_monitor.asm`. O WozMon não é distribuído:
nenhuma imagem de ROM de terceiros acompanha o projeto.

| Endereço | Uso |
|---|---|
| `$0200..=$027F` | buffer de linha |
| `$D010` / `$D011` | KBD / KBDCR |
| `$D012` / `$D013` | DSP / DSPCR |
| `$FE00..=$FFFF` | monitor (512 bytes) |

Comandos (hexadecimal, separados por espaço; prompt `\`):

- `AAAA` mostra um byte; `AAAA.BBBB` mostra a faixa, 8 por linha;
- `AAAA: BB BB ...` grava bytes a partir de `AAAA`;
- `AAAA R` executa a partir de `AAAA`.

`mos6502 apple1` abre a sessão; `Apple1::enter` faz o mesmo em testes.
