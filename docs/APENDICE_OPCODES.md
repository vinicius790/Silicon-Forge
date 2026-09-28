# Apêndice: o que `step_cycle` cobre

Os sete instáveis NMOS entram no traço, `WAI` e `STP` existem no CMOS, e `Cpu::step_half_cycle` divide cada ciclo em PHI1 e PHI2. Isto não é visual6502.

Este apêndice lista encodings, não tempos de transistor. A fonte da verdade no código é `cycles::classify` / `is_bus_cycle_stepped` / `is_bus_cycle_stepped_variant`.

A IRQ é amostrada no penúltimo ciclo (desvio: antes do operando e antes da correção de página) e entra na fronteira seguinte.

## Coberto no NMOS e no Ricoh 2A03

Todo byte em que `is_bus_cycle_stepped` é verdadeiro. São os 256.

- **151 oficiais**, em todos os modos que a tabela documenta: loads, ALU, stores, RMW (com a escrita do byte original), implícitos, pilha, branches, `JMP`/`JSR`/`RTS`/`RTI`/`BRK`, `BIT`, `NOP`
- **Estáveis não documentados**, com o mesmo efeito final que `Cpu::step`:
  - `SLO` `$07 $17 $0F $1F $1B $03 $13`
  - `RLA` `$27 $37 $2F $3F $3B $23 $33`
  - `SRE` `$47 $57 $4F $5F $5B $43 $53`
  - `RRA` `$67 $77 $6F $7F $7B $63 $73`
  - `DCP` `$C7 $D7 $CF $DF $DB $C3 $D3`
  - `ISC` `$E7 $F7 $EF $FF $FB $E3 $F3`
  - `LAX` `$A7 $B7 $AF $BF $A3 $B3 $AB`
  - `SAX` `$87 $97 $8F $83`
  - `ANC` `$0B $2B`, `ALR` `$4B`, `ARR` `$6B`, `AXS`/`SBX` `$CB`
  - `SBC` imediato não documentado `$EB`
  - `NOP` de um byte `$1A $3A $5A $7A $DA $FA`, imediato `$80 $82 $89 $C2 $E2`, página zero `$04 $44 $64`, página zero,X `$14 $34 $54 $74 $D4 $F4`, absoluto `$0C`, absoluto,X `$1C $3C $5C $7C $DC $FC`
- **`JAM`/`KIL` como halt de um ciclo**, sem avançar o PC: `$02 $12 $22 $32 $42 $52 $62 $72 $92 $B2 $D2 $F2`. Uma leitura do opcode e a CPU para. A chamada seguinte de `step_cycle` devolve erro (`cpu halted`). Não é um loop de leitura infinita no barramento

IRQ (`$FFFE`) e NMI (`$FFFA`) não são opcodes. São sete ciclos quando `step_cycle` começa uma instrução com a linha pendente. O reset são sete leituras dentro de `Cpu::reset`, fora deste conjunto.

## Instáveis NMOS

Mesmo efeito em `Cpu::step` e `step_cycle`. `H` é o byte alto da base, antes do índice. Com cruzamento de página o valor gravado também substitui o byte alto do endereço.

| Byte | Nome | Modo | Efeito |
|---|---|---|---|
| `$8B` | `ANE` | imediato | `A = (A \| ANE_MAGIC) & X & imm`, `ANE_MAGIC = $EE`. Chips reais variam |
| `$93` | `SHA` | `(zp),Y`, 6 ciclos | grava `A & X & (H+1)` |
| `$9F` | `SHA` | abs,Y, 5 ciclos | grava `A & X & (H+1)` |
| `$9B` | `TAS`/`SHS` | abs,Y, 5 ciclos | `SP = A & X`, grava `SP & (H+1)` |
| `$9C` | `SHY` | abs,X, 5 ciclos | grava `Y & (H+1)`. **No CMOS é `STZ` abs** |
| `$9E` | `SHX` | abs,Y, 5 ciclos | grava `X & (H+1)`. **No CMOS é `STZ` abs,X** |
| `$BB` | `LAS` | abs,Y, 4+1 ciclos | `SP & M` vai para A, X e SP |

## Só no CMOS (`CpuVariant::CMOS`)

Por cima da matriz NMOS, estes bytes deixam de ser `JAM` ou `NOP` ilegal e passam a ser a instrução 65C02 que o núcleo executa. `is_bus_cycle_stepped_variant(op, CMOS)` é verdadeiro, e o efeito final bate com `Cpu::step`:

| Bytes | Instrução |
|---|---|
| `$80` | `BRA` |
| `$1A` / `$3A` | `INC A` / `DEC A` |
| `$5A` / `$DA` | `PHY` / `PHX` |
| `$7A` / `$FA` | `PLY` / `PLX` |
| `$64` `$74` `$9C` `$9E` | `STZ` zp, zp,X, abs, abs,X |
| `$12` `$32` `$52` `$72` `$B2` `$D2` `$F2` | ALU/load `(zp)` |
| `$92` | `STA (zp)` |
| `$04` `$14` | `TSB`/`TRB` zp |
| `$0C` `$1C` | `TSB`/`TRB` abs |
| `$89` | `BIT` imediato (só Z) |
| `$34` `$3C` | `BIT` zp,X e abs,X |
| `$7C` | `JMP (abs,X)` |
| `$CB` | `WAI`: 3 ciclos, dorme até IRQ ou NMI; com `I` setado a IRQ acorda sem vetor |
| `$DB` | `STP`: 3 ciclos, para até `Cpu::reset` |

No CMOS, `$02 $22 $42 $62` continuam `JAM` e usam o mesmo halt de um ciclo.

## Decimal

`ADC`/`SBC` com D=1 seguem o tutorial de Bruce Clark. NMOS: N e V do `ADC` pela soma intermediária, Z pela soma binária; `SBC` só muda o acumulador. 65C02: N e Z do resultado decimal e um ciclo a mais (leitura do PC no traço).
