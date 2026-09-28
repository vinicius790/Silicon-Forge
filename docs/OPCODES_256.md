# Referência Completa da Matriz de 256 Opcodes (MOS 6502 / 65C02)

O microprocessador MOS Technology 6502 possui uma matriz de instruções de 16x16 (256 posições possíveis de código de operação de 1 byte).

O emulador cobre **100% dos 256 opcodes**:
- **151 Opcodes Oficiais (Documentados)**
- **105 Opcodes Não-Documentados (Ilegais / Unofficial NMOS)**
- **Extensões de Instrução CMOS (65C02)**

---

## Tabela de Opcodes Não-Documentados (Ilegais)

Muitos jogos comerciais do NES e Commodore 64 utilizavam opcodes não-documentados intencionalmente para economizar ciclos e bytes. O emulador suporta todos eles com fidelidade de comportamento:

| Mnemônico | Opcode(s) | Modos | Operação |
|---|---|---|---|
| **`LAX`** | `$A7, $B7, $AF, $BF, $A3, $B3, $AB` | zp, zpY, abs, absY, (zp,X), (zp),Y, #imm | `LDA oper` + `LDX oper`. Carrega o operando simultaneamente em A e X. |
| **`SAX`** | `$87, $97, $8F, $83` | zp, zpY, abs, (zp,X) | Grava `A & X` na memória sem alterar registradores. |
| **`DCP`** | `$C7, $D7, $CF, $DF, $DB, $C3, $D3` | zp, zpX, abs, absX, absY, (zp,X), (zp),Y | `DEC oper` seguido imediatamente por `CMP oper`. |
| **`ISC`** (`ISB`) | `$E7, $F7, $EF, $FF, $FB, $E3, $F3` | zp, zpX, abs, absX, absY, (zp,X), (zp),Y | `INC oper` seguido imediatamente por `SBC oper`. |
| **`SLO`** (`ASO`) | `$07, $17, $0F, $1F, $1B, $03, $13` | zp, zpX, abs, absX, absY, (zp,X), (zp),Y | `ASL oper` seguido imediatamente por `ORA oper`. |
| **`RLA`** | `$27, $37, $2F, $3F, $3B, $23, $33` | zp, zpX, abs, absX, absY, (zp,X), (zp),Y | `ROL oper` seguido imediatamente por `AND oper`. |
| **`SRE`** (`LSE`) | `$47, $57, $4F, $5F, $5B, $43, $53` | zp, zpX, abs, absX, absY, (zp,X), (zp),Y | `LSR oper` seguido imediatamente por `EOR oper`. |
| **`RRA`** | `$67, $77, $6F, $7F, $7B, $63, $73` | zp, zpX, abs, absX, absY, (zp,X), (zp),Y | `ROR oper` seguido imediatamente por `ADC oper`. |
| **`ALR`** (`ASR`) | `$4B` | #imm | `AND #imm` seguido de `LSR A`. |
| **`ANC`** | `$0B, $2B` | #imm | `AND #imm`, copiando o bit mais significativo (N) para a flag Carry (C). |
| **`ARR`** | `$6B` | #imm | `AND #imm` seguido de `ROR A` com ajustes de Carry e Overflow. |
| **`SBX`** (`AXS`) | `$CB` | #imm | `(A & X) - imm -> X`. Define flags C, Z e N. |
| **`NOPs`** | `$04, $0C, $14, $1A, $1C, $34, $3A, $3C...` | Vários | NOPs que consomem 1, 2 ou 3 bytes e executam leituras falsas (dummy reads). |
| **`JAM`** (`KIL`) | `$02, $12, $22, $32, $42, $52, $62, $72, $92...` | Implied | Congela o pipeline da CPU até que ocorra um sinal de Reset de hardware. |

---

## Extensões do CMOS 65C02

Quando a CPU é inicializada com `CpuVariant::CMOS` (via `--variant cmos` ou `Cpu::with_variant(bus, CpuVariant::CMOS)`), as seguintes novas instruções são disponibilizadas:

| Instrução | Opcode(s) | Modo | Descrição |
|---|---|---|---|
| **`BRA`** | `$80` | Relative | Salto incondicional relativo de 8 bits (-128..+127). |
| **`STZ`** | `$64, $74, $9C, $9E` | zp, zpX, abs, absX | Grava zero (`$00`) na memória sem modificar o registrador A. |
| **`PHX`** | `$DA` | Implied | Empilha o registrador X na pilha (`$0100`). |
| **`PLX`** | `$FA` | Implied | Desempilha o topo da pilha diretamente para o registrador X. |
| **`PHY`** | `$5A` | Implied | Empilha o registrador Y na pilha (`$0100`). |
| **`PLY`** | `$7A` | Implied | Desempilha o topo da pilha diretamente para o registrador Y. |
| **`INC A`** | `$1A` | Accumulator | Incrementa diretamente o acumulador. |
| **`DEC A`** | `$3A` | Accumulator | Decrementa diretamente o acumulador. |
| **`TRB`** | `$14, $1C` | zp, abs | Test and Reset Bits (testa bits com Z e limpa-os na memória). |
| **`TSB`** | `$04, $0C` | zp, abs | Test and Set Bits (testa bits com Z e ativa-os na memória). |
| **`JMP (abs,X)`** | `$7C` | AbsIndX | Salto indireto indexado para tabelas de ponteiros de funções. |
| **`LDA ($zp)`** | `$B2` | ZpInd | Endereçamento indireto na Zero Page sem necessidade de indexar com Y. |
| **`STA ($zp)`** | `$92` | ZpInd | Armazenamento indireto na Zero Page. |
