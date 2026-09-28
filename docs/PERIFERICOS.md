# Periféricos: VIA 6522 e ACIA 6551

Este documento descreve o que `src/via.rs` e `src/acia.rs` modelam, onde o
modelo segue a folha de dados e onde ele diverge de propósito. Os testes que
fixam cada comportamento estão em `tests/pe_via.rs`, `tests/pe_acia.rs` e
`tests/pe_examples.rs`.

## VIA 6522

### Variantes (`ViaVariant`)

| Peça | Tecnologia | Saída /IRQ |
|------|------------|------------|
| `Mos6522` (padrão) | NMOS | dreno aberto, precisa de pull-up |
| `RockwellR6522` | NMOS | dreno aberto |
| `WdcW65c22N` | CMOS | dreno aberto (substituto direto do NMOS) |
| `WdcW65c22S` | CMOS | totem-pole; não pode ser ligada em OU com outras fontes |

`Via6522::irq_pin_drive()` informa o que o pino faz numa linha compartilhada:
`Some(false)` puxa para baixo, `Some(true)` força alto (só a W65C22S) e `None`
solta a linha para o pull-up. Os registradores, timers, o shift register e os
handshakes se comportam igual nas quatro peças; um teste roda a mesma sequência
de 200 acessos em todas e compara os resultados.

### PB7 controlado pelo timer 1

Com o bit 7 do ACR ligado, o timer 1 dirige PB7 qualquer que seja o DDRB, e a
leitura de ORB mostra esse nível (`Via6522::pb7_output`):

* **One-shot** (ACR bit 6 = 0): a escrita em T1C-H põe PB7 em nível baixo; o
  estouro o devolve ao nível alto. Resultado: um único pulso baixo de N + 1,5
  ciclos (N ticks inteiros depois do ciclo da escrita). Uma nova escrita em
  T1C-H antes do estouro estica o pulso.
* **Free-run** (ACR bit 6 = 1): a escrita em T1C-H põe PB7 em nível baixo e cada
  estouro o inverte, uma onda quadrada com meio período de N + 2 ciclos.
* Com o bit 7 desligado, PB7 volta a ser um bit comum da porta. O RES devolve o
  pino à porta.

`Via6522::pb7()` continua sendo o flip-flop que só alterna no modo free-run com
PB7 ativo, como antes.

### Latch de entrada (ACR bits 0 e 1)

A borda ativa de CA1 (PCR bit 0) captura os pinos da porta A, inclusive os bits
de saída; a borda ativa de CB1 (PCR bit 4) captura as entradas da porta B, e os
bits de saída de B continuam lendo ORB. A borda inativa não captura. Desligar o
latch devolve a leitura dos pinos ao vivo; religá-lo mostra o último valor
capturado.

### Shift register (ACR bits 4–2)

| Modo | Relógio | Comportamento |
|------|---------|---------------|
| `000` | — | desligado; nem SR nem IFR2 mudam |
| `001` | T2 | entrada; CB1 alterna a cada N + 2 ciclos, bit amostrado na borda de subida |
| `010` | PHI2 | entrada, um bit por ciclo |
| `011` | CB1 externo | entrada nas bordas de subida de CB1 |
| `100` | T2 | saída em recirculação contínua, nunca liga IFR2 |
| `101` | T2 | saída, para após oito bits |
| `110` | PHI2 | saída, um bit por ciclo |
| `111` | CB1 externo | saída a cada borda de CB1 (ver divergência abaixo) |

Nos modos de saída, `Via6522::cb2_shift_data()` dá o nível de CB2: o último bit
deslocado, MSB primeiro. Nos modos de T2, cada bit leva 2(N + 2) ciclos; CB1 começa
alto, os bits saem nas bordas de descida e o oitavo (com IFR2) sai 15(N + 2)
ciclos depois da escrita no SR.

Detalhes conferidos na folha de dados (MOS 6522, WDC W65C22):

* `111`: um deslocamento por pulso de CB1, na borda de descida (o dado em CB2
  fica válido depois da transição negativa de CB1). A borda de subida não
  desloca.
* `011` e `111`: o contador "não interrompe o deslocamento". Depois de oito
  pulsos (IFR2 ligado) os pulsos seguintes continuam deslocando, sem contar, até
  um acesso ao SR rearmar outros oito.
* `101`: o oitavo bit sai numa borda de descida em 15(N + 2) ciclos; o IFR2 liga
  quando CB1 volta ao nível alto, em 16(N + 2) ciclos, e CB1 fica alto. O `001`
  também liga o IFR2 em 16(N + 2), na oitava borda de subida.

### IFR e IER

* O bit 7 do IFR é o OU dos flags habilitados; escrever nele não faz nada.
* Escrever 1 num bit do IFR limpa esse flag.
* No IER, o bit 7 escolhe entre ligar e desligar os bits marcados; a leitura
  sempre traz o bit 7 em 1.
* Desligar um bit do IER solta o IRQ mas mantém o flag; religá-lo com o flag
  ainda ativo volta a afirmar o IRQ.
* Caminhos de limpeza: T1 por leitura de T1C-L ou escrita em T1C-H; T2 por
  leitura de T2C-L ou escrita em T2C-H; CA1/CB1 por acesso a ORA/ORB (não pelo
  espelho `$0F`); CA2/CB2 também por esse acesso, exceto no modo de entrada
  independente; SR por acesso ao SR.

## ACIA 6551

### Variantes (`AciaVariant`)

* `Classic` (padrão): MOS, Rockwell R6551, Synertek. O TDRE (status bit 4)
  acompanha o registrador de transmissão.
* `WdcW65c51n`: a WDC documenta que na W65C51N o bit TDRE sempre lê 1 e que
  escrever no registrador de transmissão durante um envio corrompe o byte. Por
  isso a interrupção de transmissão nunca dispara; o software precisa esperar o
  tempo de um quadro por conta própria. A recepção e sua interrupção funcionam
  normalmente.

### Receptor bit a bit

O receptor com sobreamostragem de 16x (`Acia6551::rxd_16x`) considera a linha em
repouso (nível alto, marca) ao ligar, como um receptor RS-232 a vê; a primeira
amostra baixa já é um start bit. Snapshots antigos sem o campo `rx_line` carregam
com o último nível baixo. Uma descida inicia o start bit, que é confirmado
na amostra 8; se a linha já voltou ao alto, é ruído: o quadro é descartado e
`Acia6551::false_starts()` conta o caso. Pulsos de 1 a 7 amostras nunca geram
byte.

### Break

* **Envio:** bits 3–2 do comando em `11` seguram TxD em baixo e abandonam o
  quadro em andamento; ao sair do break a linha volta ao repouso.
* **Detecção:** um quadro com dados, paridade (se houver) e stop bit todos em
  baixo chega como `$00` com erro de enquadramento, e
  `Acia6551::rx_break_detected()` fica ligado até o próximo quadro. Um `$00`
  com stop bit correto não é break.

## Exemplos

| Arquivo | O que faz | Verificado por |
|---------|-----------|----------------|
| `examples/wozmon_lite.asm` | monitor no estilo do Wozmon (código original): examinar, bloco, gravar, `R` | `scripts/examples.sh` e `tests/pe_programs.rs` |
| `examples/forth_lite.asm` | Forth interpretado: pilha de 16 bits, `:`/`;` recursivo, `IF/ELSE/THEN`, `BEGIN/UNTIL` | idem |
| `examples/basic_tokenizer.asm` | tokenizador e LIST de um BASIC inteiro (14 palavras-chave) | idem |
| `examples/float_lite.asm` | soma e multiplicação em ponto flutuante de 4 bytes (layout do MS BASIC), truncando | idem |
| `examples/lz_decompress.asm` | descompressor LZSS (distância até 4096, tamanho 3–18) | idem |
| `examples/via_clock.asm` | relógio por IRQ do T1 (1000 ciclos), segundos em ORB, onda em PB7 | `tests/pe_examples.rs` |
| `examples/acia_irq_echo.asm` | eco por interrupção de recepção, com buffer em `$0200` | `tests/pe_examples.rs` |
| `examples/sieve_256.asm` | crivo de Eratóstenes de 0 a 255; 54 primos | `scripts/examples.sh` e teste |
| `examples/crc16.asm` | CRC-16/CCITT-FALSE de "123456789" = `$29B1` | `scripts/examples.sh` e teste |
| `examples/muldiv.asm` | multiplicação 8x8 e divisão 16/8 | `scripts/examples.sh` e teste |

Os exemplos de VIA e ACIA rodam no comando `board` da CLI (RAM + 6522 em `$C000`
+ 6551 em `$D000`) e numa placa equivalente montada no teste. `scripts/examples.sh` executa esse teste quando
`DEVICE_EXAMPLES=1`. Atenção: no assembler do projeto, número sem prefixo é
hexadecimal (`#10` é 16); os exemplos usam sempre `$`.
