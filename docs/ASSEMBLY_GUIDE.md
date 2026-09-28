# Guia Prático de Programação em Assembly MOS 6502 (Assembler 3.0)

Este guia apresenta os fundamentos da linguagem assembly do MOS 6502 e as diretivas suportadas pelo assembler integrado.

Números sem prefixo são interpretados em hexadecimal. Use `$2A` ou `0x2A` para hexadecimal, `%101010` para binário e `0d42` para decimal. Literais entre aspas simples, como `'A'`, representam um byte ASCII.

---

## 1. Sintaxe Geral

Um programa em assembly do 6502 consiste em linhas compostas por:

- **Labels** (rótulos de endereço): identificam posições de código ou variáveis.
- **Instruções** (mnemônicos): `LDA`, `STA`, `JMP`, `BNE`, etc.
- **Operandos**: definem os valores imediatos ou endereços de memória.
- **Comentários**: iniciados com ponto e vírgula `;`.

```assembly
; Exemplo de laço de contagem regressiva
.org $8000          ; Define a origem na memória

start:
    LDX #$0A        ; Carrega 10 no registrador X (Imediato #)
loop:
    DEX             ; Decrementa X
    BNE loop        ; Pula para 'loop' enquanto X != 0
    BRK             ; Interrompe a execução
```

---

## 2. Modos de Endereçamento Suportados

| Modo | Sintaxe Exemplo | Descrição |
| --- | --- | --- |
| **Imediato** | `LDA #$42` ou `LDA #10` | O valor numérico em si é fornecido |
| **Página Zero** | `LDA $10` | Endereço nos primeiros 256 bytes (`$00..$FF`) - rápido! |
| **Página Zero, X** | `LDA $10,X` | Endereço `$10 + X` (com wrap dentro da página zero) |
| **Página Zero, Y** | `LDX $10,Y` | Endereço `$10 + Y` (usado por LDX/STX) |
| **Absoluto** | `LDA $8000` | Endereço completo de 16 bits |
| **Absoluto, X** | `LDA $8000,X` | Endereço `$8000 + X` |
| **Absoluto, Y** | `LDA $8000,Y` | Endereço `$8000 + Y` |
| **Indireto** | `JMP ($1234)` | Salta para o endereço apontado pelo ponteiro em `$1234` |
| **Indexado Indireto** | `LDA ($20,X)` | Zero Page indexada antes: pega ponteiro em `$20 + X` |
| **Indireto Indexado** | `LDA ($20),Y` | Pega ponteiro na Zero Page e soma Y ao destino |
| **Relativo** | `BNE label` | Salto de branch condicional (-128 a +127 bytes) |

---

## 3. Diretivas do Assembler 3.0

O Assembler 2.0 suporta diretivas modernas para facilitar o desenvolvimento de programas complexos:

### `.org <endereço>` ou `* = <endereço>`

Define onde os bytes subsequentes serão montados. A imagem `.bin` é contígua: `.org` crescente preenche a lacuna com `$00`; um `.org` que volte sobre bytes já emitidos é rejeitado. A primeira origem determina o endereço base da imagem.

```assembly
.org $8000
```

### `.byte` ou `db <b1>, <b2>...`

Emite bytes brutos diretamente no binário:

```assembly
.byte $01, $02, $FF, %10101010, 0d42
```

### `.word` ou `dw <w1>, <w2>...`

Emite palavras de 16 bits no formato little-endian (byte baixo primeiro):

```assembly
.word $1234, $ABCD   ; Gera os bytes: 34 12 CD AB
```

### `.text` ou `.ascii "<texto>"`

Emite caracteres ASCII correspondentes à string:

```assembly
.text "MOS 6502"
```

### `.asciiz "<texto>"`

Emite string ASCII com terminador nulo (`$00`) ao final:

```assembly
.asciiz "FIM DA MENSAGEM"
```

### `.fill <quantidade>, [valor]`

Preenche um determinado número de bytes na memória com um valor específico:

```assembly
.fill 0d64, $EA      ; Emite 64 instruções NOP ($EA)
```

### `<nome> = <valor>` ou `<nome> .equ <valor>`

Define constantes simbólicas reutilizáveis no código:

```assembly
BUFFER_SIZE = $20
SCREEN_ADDR = $D012
```

---

## 4. Compilando e Exportando com a CLI

Para montar um programa em assembly diretamente pela linha de comando:

```bash
# Gera binário simples (.bin)
mos6502 asm program.asm -o program.bin

# Imprime Intel HEX no stdout (somente -o grava arquivo, sempre como binário cru)
mos6502 asm program.asm -o program.bin --hex

# Imprime a tabela de símbolos no stdout
mos6502 asm program.asm --sym
```
