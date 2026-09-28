# Arquitetura do mos6502 1000000.0.0

Esta versão acrescenta comportamento testado em cima da 50.0. Não é uma PPU, não mede MIPS e não traz ROM de terceiros.

## O que a 1000000.0.0 acrescenta

- **CA2 e CB2.** O PCR escolhe o modo de saída. Bits 3–1 são CA2 e bits 7–5 são CB2. `111` segura alto, `110` segura baixo, `101` pulsa quando a CPU escreve o registrador de saída (`$01` pulsa CA2, `$00` pulsa CB2; `$0F` não pulsa). O pino fica baixo até o próximo `tick`. `100` baixa o pino na escrita e solta na borda de descida de CA1 ou CB1. Os modos de entrada não dirigem o pino.
- **Macros um nível.** Uma macro pode chamar outra. A macro chamada não pode chamar uma terceira: isso continua `macro aninhada rejeitada`. Definir `.macro` dentro de `.macro` também continua erro.
- **Name table.** 256 bytes, 16×16 índices de tile. `render_name_tile(coluna, linha)` pinta um tile 8×8 do canto 4×4 visível no framebuffer 32×32. Cor de padrão 0 não escreve.
- **Paleta.** 16 registradores. O blit e o tile da name table gravam `palette[nibble]` quando o nibble do padrão não é zero. O padrão de fábrica é a identidade.
- **Watch de PC.** `watch_pc_range` para `Debugger::run` com `StopReason::PcRange` se o PC está dentro do intervalo inclusive. Fora, a execução segue até o limite.
- **Baud da 6551.** O nibble baixo do controle escolhe o período da contagem que já existia. A tabela está em `acia` (0 = imediato, 1 = 20000 ciclos, …, `0xE` = 104, `0xF` = 52). `Acia6551::set_crystal(Some(hz_da_cpu))` troca a tabela pelo cristal (seção 6551). `Cpu::step` não dá `tick`.
- **Monitor de 1 KiB.** `examples/monitor_1k.asm` é um laço original: espera RDRF em `$D001` e devolve o byte em `$D000`. A imagem montada tem 1024 bytes em `$FC00`. Não é o WozMon.
- **Snapshot versão 4.** `at_console` grava paleta e name table. A ida e volta é JSON à mão. Versão 0 é rejeitada. `capture` continua na versão 2.
- **Ligador.** `link` coloca seções com `.org` no endereço fixo e seções sem origem em `reloc_base`. A segunda passagem aplica relocação absoluta (16 bits, little-endian) e relativa (desvio com sinal). Símbolo indefinido, global duplicado ou seções sobrepostas são erro. `write_object`/`read_object` guardam o módulo com soma dos bytes. `Linked::line_at` devolve a linha de origem daquele endereço.

## Fidelidade de CPU: instáveis, WAI/STP, PHI1/PHI2, IRQ e decimal

- **Instáveis NMOS.** `SHA` (`$93` `(zp),Y`, `$9F` abs,Y), `SHX` (`$9E` abs,Y), `SHY` (`$9C` abs,X) e `TAS`/`SHS` (`$9B` abs,Y) gravam `registrador & (byte alto da base + 1)`. Se o índice cruzou a página, esse valor também vira o byte alto do endereço: `SHX $12F0,Y` com Y=`$20` e X=`$05` grava `$01` em `$0110`, não em `$1310`. `TAS` antes faz `SP = A & X`. `LAS` (`$BB`) faz `SP & M` em A, X e SP. `ANE` (`$8B`) faz `A = (A | ANE_MAGIC) & X & imm`, com `ANE_MAGIC = $EE`. Chips reais variam (`$00`, `$EE`, `$EF`, `$FF`, e com a temperatura); a constante é a documentada mais comum. Todos entram em `step` e em `step_cycle`, com o mesmo resultado. No CMOS `$9C`/`$9E` continuam `STZ`.
- **`WAI` (`$CB`, só CMOS).** Três ciclos e a CPU dorme. Cada `step` ou `step_cycle` sem IRQ e sem NMI é um ciclo ocioso (`step_cycle` lê o PC com `peek`, sem efeito colateral). NMI acorda e entra no vetor. IRQ com `I` limpo entra no vetor; com `I` setado acorda e segue na instrução seguinte, sem vetor. Reset também acorda. O estado de espera vai para o snapshot versão 6 (`with_wait_state`, campo `waiting`); as versões 2 a 5 carregam com `waiting` falso e a 99 é recusada.
- **`STP` (`$DB`, só CMOS).** Três ciclos e a CPU para. IRQ e NMI não acordam; só `Cpu::reset`. No NMOS `$CB` continua `AXS`/`SBX` e `$DB` continua `DCP` abs,Y.
- **PHI1/PHI2.** `Cpu::step_half_cycle` divide cada ciclo de `step_cycle`: em PHI1 vêm o endereço e R/W (`data` é `None`), em PHI2 o dado. `LDA $1234` dá `8000/AD 8001/34 8002/12 1234/dado`, cada um como PHI1 e depois PHI2. Nessa vista o acesso acontece no PHI1. `Cpu::step_phase` é o relógio de duas fases: no PHI1 só saem endereço e R/W, achados rodando o ciclo numa cópia dos registradores sobre `MemoryBus::peek` com as escritas descartadas (o barramento não é copiado nem tocado), e o acesso real, com os efeitos no dispositivo, acontece no PHI2. Antes de cada metade (no PHI2, antes do acesso) a CPU chama `MemoryBus::phase`; `RamAcia`, `ViaBoard`, `LoggingBus` e `SystemBus` (para cada dispositivo mapeado, via `IoDevice::phase`) repassam para `tick_phase`; `Via6522` agora também é um `IoDevice`. O flag de timer segurado vai no instantâneo (`t1_flag_due`, `t2_flag_due`), e `MachineImage::with_phase_state` (v7) guarda o PHI1 aberto: a restauração continua no PHI2 sem dar clock de novo nos dispositivos. Ler o status da 6551 só apaga o IRQ no PHI2; um `STA` só chega ao registrador no PHI2. `step`, `step_cycle` e `step_half_cycle` recusam rodar com o PHI1 aberto. O resultado é o de `step`.
- **Ponto de amostragem da IRQ.** O nível da linha durante o penúltimo ciclo decide se a IRQ entra depois da instrução. IRQ que sobe antes do último ciclo entra logo depois; IRQ que sobe só no último ciclo espera mais uma instrução. Desvio amostra antes da busca do operando e, se tomado com cruzamento de página, de novo antes do ciclo de correção. Por isso o desvio tomado sem cruzamento ignora a linha nos dois últimos ciclos e adia a IRQ uma instrução. O latch de NMI usa o mesmo ponto: uma borda que chega só no último ciclo (ou, num desvio tomado sem cruzamento, depois da busca do opcode) espera mais uma instrução. `I` é conferido na fronteira; `CLI` continua adiando uma instrução. Uma linha mudada entre duas chamadas pertence ao ciclo seguinte: numa CPU nova, depois de restaurar uma imagem ou depois do `reset` (que apaga o latch de NMI), a borda cai dentro do primeiro opcode, que roda antes. `step` conta como se a linha estivesse no nível em que ele rodou. No `WAI` cada ciclo ocioso amostra; o ciclo que vê a linha ainda é ocioso e a interrupção começa no seguinte.
- **Sequestro por NMI.** Um NMI travado nos quatro primeiros ciclos de um `BRK` ou de uma entrada de IRQ troca a busca do vetor para `$FFFA`. O status empilhado mantém o B do `BRK` e o NMI é consumido. No ciclo 5 ou 6 não sequestra e entra depois do `BRK`; no último ciclo espera um opcode do handler.
- **`ARR` (`$6B`).** Binário: `(A & imm)` girado à direita com C, C = bit 6, V = bit 6 xor bit 5. Decimal NMOS: N = C antigo, V de `(t ^ resultado) & $40`, depois a correção BCD de cada nibble, e C vem da correção do nibble alto. O 2A03 ignora D.
- **Decimal NMOS.** `ADC`: acumulador e C pela sequência 1 de Bruce Clark; N e V pela soma intermediária antes do ajuste do nibble alto; Z pela soma binária (`$99 + $01` dá A=`$00`, C=1, Z=0, N=1). `SBC`: só o acumulador é decimal; N, V, Z e C são os da subtração binária. Vale para BCD inválido também.
- **Decimal 65C02.** N e Z vêm do resultado decimal, C e V como no NMOS (`SBC` pela sequência 4). `ADC`/`SBC` com D=1 gastam um ciclo a mais; em `step_cycle` é uma leitura do PC depois do último ciclo. O 2A03 não tem decimal.

## 6522 e 6551 conforme o datasheet

- **6522.** Ler ou escrever ORA (`$01`) limpa CA1; ORB (`$00`) limpa CB1. `$0F` não limpa nada. A borda ativa de CA1/CB1 é o bit 0/4 do PCR (0 descida, 1 subida). CA1, CB1, CA2 e CB2 ligam o flag no IFR mesmo com o IER desligado; o IER só controla o bit 7 e o pino de IRQ. Os contadores decrementam a cada PHI2 mesmo sem estarem armados (T2 só no modo temporizado); um disparo único que estoura segue contando a partir de `$FFFF` sem ligar o flag de novo.
- **Tempo do T1.** O flag liga no tick que leva o contador armado de 0 para `$FFFF`: N+1 ticks depois de carregar N, o que com o ciclo da escrita dá os N+1,5 do datasheet. No free-run o tick seguinte copia o latch, e cada período tem N+2. O pedido de recarga vai no instantâneo (`t1_reload`). Com `Via6522::tick_phase` o contador passa no PHI2 e o flag segurado aparece no PHI1 seguinte: escrita no PHI2 do ciclo 0, flag no PHI1 do ciclo N+2, os N+1,5 de fato; reescrever T1C-H/T2C-H descarta o flag segurado. O T2 temporizado também liga o flag na passagem para `$FFFF` (N+1 ticks, N+1,5 com a escrita); no modo de contagem de pulsos em PB6 o flag liga ao chegar a zero.
- **Shift pelo T2.** Nos modos `001` (entrada), `101` (saída) e `100` (saída livre) o CB1 inverte a cada N+2 PHI2, com N o byte baixo do latch do T2. A saída troca de bit na descida e a entrada amostra na subida: um bit a cada 2(N+2). `001`/`101` param depois de oito bits e ligam o bit 2 do IFR; `100` gira o byte para sempre sem flag. Ler o SR apaga o bit 2 e, nos modos de entrada, começa mais oito bits.
- **6551.** O reset de hardware deixa o comando em `$02` e o status em `$10`. O bit 7 do status liga quando RDRF sobe com DTR ligado e bit 1 do comando limpo, ou quando TDRE sobe com DTR ligado e bits 3–2 em `01`; ligar a interrupção com a condição já presente também liga. Ler o status apaga o bit 7. `irq_asserted` é o pino. O reset programado (escrever em `$01`) limpa os bits 0–4 do comando e mantém a paridade (bits 5–7) e o controle. O bit 7 vai para o instantâneo (`acia.irq`); imagens antigas sem o campo carregam falso.
- **DTR, modem e eco.** Com DTR desligado (bit 0 do comando) o receptor descarta os bytes. `/DCD` alto liga o bit 5 do status e também desliga o receptor; `/DSR` alto liga o bit 6. Uma mudança em DCD ou DSR gera IRQ quando a interrupção do receptor está habilitada. `/CTS` alto segura o transmissor: o byte fica no registrador, a contagem para e TDRE não volta. No modo eco (bit 4 com bits 3–2 em `00`) cada byte recebido sai também por `pop_echo`. Os níveis de DCD/DSR/CTS e a fila de eco vão para o instantâneo; imagens antigas carregam pinos baixos e fila vazia.
- **Cristal, break, RTS/DTR.** `set_crystal(Some(hz))` faz a contagem andar em períodos do cristal de 1,8432 MHz (`CRYSTAL_HZ / hz` por `tick`, com o resto guardado). O divisor interno (`CRYSTAL_DIVIDER_BY_NIBBLE`) gera o clock 16x; um quadro inteiro são 16 clocks por bit de start, dados, paridade e stop (1,5 stop com 5 bits sem paridade, 1 com 8 bits e paridade). O período é recalculado a cada escrita de controle ou comando, e o modo cristal vai no instantâneo. `rts_asserted` (bits 3–2 diferentes de `00`) e `dtr_asserted` (bit 0) são as saídas. `tx_break` é o par `11`, que manda break e desliga o IRQ do transmissor. Entrar em break corta o byte em transmissão (não chega ao host) e TxD fica baixo (`txd_high` falso) enquanto o break durar: a contagem para e um byte escrito nesse tempo espera o fim. `receive_break` entrega `$00` com erro de quadro.
- **Clock 16x externo e fases.** Com `set_external_clock(true)` o nibble 0 conta pulsos do host em `clock_16x` (um quadro 8N1 = 160 pulsos) e `tick` não conta. `tick_phase` conta a tabela só no PHI2 e o cristal nas duas fases (`CRYSTAL_HZ / 2·hz` por metade). O clock externo vai no instantâneo.

## Instantâneo v8, pinos da 6522 e janela de fase

- **v8.** `MachineImage::with_cycle_state` grava `cycle_state`, os 36 bytes de `Cpu::cycle_state` (tabela no doc da função): sequência em andamento, linha de IRQ, trinco de NMI, históricos de polling, `CLI` adiado, ciclo decimal extra do 65C02, PHI1 aberto e meia-volta pendente de `step_half_cycle`. Um corte em qualquer ciclo, inclusive numa sequência de IRQ ou entre as metades, continua no mesmo ciclo seguinte. Bytes inválidos (tamanho, bits reservados, tipo de sequência desconhecido) dão erro antes de mexer na CPU. A versão 9 é rejeitada; v2–v7 carregam como antes (v7 só guarda o PHI1 de fronteira). `jsr_depth` e `last_insn` são do depurador e não entram.
- **Carga dos pinos.** Modelo simples: um pino de entrada que o host não dirige (`drive_port_a/b(máscara, níveis)`) sobe pelo pull-up e lê 1; `set_port_a_in`/`set_port_b_in` dirigem todos. `set_port_a_load_low`/`set_port_b_load_low` são cargas open-drain (AND por fio). Na porta A uma carga baixa vence uma saída alta, porque a saída NMOS sobe passivamente e ORA lê o pino. Na porta B a saída é push-pull e a leitura de um bit de saída devolve ORB, então a carga só abaixa entradas. `port_a_pins`/`port_b_pins` dão o nível das linhas. Instantâneos antigos carregam tudo dirigido e sem carga, o comportamento anterior. Correntes, tensões e tempos de subida não são modelados.
- **Janela de fase.** Setup e hold são nanossegundos dentro de uma fase, e o núcleo não tem tempo abaixo da fase, então não há como medi-los. O que faz sentido é checar a janela: `PhaseCheckBus` recusa leitura ou escrita depois de um aviso de PHI1 (a leitura devolve `peek`, sem efeito; a escrita é descartada; as duas vão para `rejected`). `step_phase` nunca dispara a checagem. Sem aviso de fase (`step`, `step_cycle`) tudo passa.
- **`tick` da 6522** é exatamente `tick_phase(Phi2)` seguido de `tick_phase(Phi1)`; o resultado de antes não muda.

## O que continua de fora

- Tempos de setup/hold em nanossegundos e a parte elétrica dos pinos (corrente, tensão, subida), pelos motivos acima.

- Não há PPU, scroll, DMA de sprite nem temporização de vídeo de NES ou VIC.


`Cpu<B: MemoryBus>` executa sobre 64 KiB. O passo de instrução (`Cpu::step`) decodifica um opcode, aplica o efeito arquitetural e soma um total de ciclos. Isso não é um traço de barramento.

`Cpu::step_cycle` avança **um** ciclo de barramento. A sequência é de livro: busca do opcode, bytes do operando, endereço efetivo, a leitura extra opcional de cruzamento de página, stores, e o par read-modify-write (escreve o byte original de volta, depois escreve o resultado).

## O que ainda não é cycle-accurate

O núcleo **não** é totalmente cycle-accurate e **não** é o netlist do visual6502.

- `step_phase` separa endereço e acesso, mas não é o netlist: dentro de cada fase não há tempos de setup/hold.
- IRQ e NMI são amostrados no penúltimo ciclo (seção acima) e começam na fronteira. `Cpu::step` só olha as linhas durante `WAI`. `Cpu::irq` e `Cpu::nmi` continuam sendo a entrada imediata no nível de instrução.
- `JAM`/`KIL` é um halt de um ciclo (uma leitura do opcode, PC parado). Não é o trinco de barramento que fica relendo o byte para sempre, e não é um traço de transistor.
- O reset não passa por `step_cycle`. `Cpu::reset` faz sete leituras e nenhuma escrita, e `cycles` fica em 7.

## O que o traço cobre

No NMOS e no Ricoh 2A03 (a mesma matriz; o Ricoh só ignora o decimal dentro da ALU):

- os 151 opcodes oficiais
- grupos estáveis não documentados, com o mesmo efeito final que `Cpu::step`: `SLO`, `RLA`, `SRE`, `RRA`, `DCP`, `ISC`, `LAX` (incluindo o imediato `$AB`), `SAX`, `ANC`, `ALR`, `ARR`, `AXS`/`SBX`, o `SBC` imediato `$EB`, e os `NOP` de 1, 2 e 3 bytes
- `JAM`/`KIL`: uma leitura no PC, `halted = true`, PC não avança. No NMOS isso é `$02 $12 $22 $32 $42 $52 $62 $72 $92 $B2 $D2 $F2`. No CMOS, `$12 $32 $52 $72 $92 $B2 $D2 $F2` são os indiretos `(zp)` e entram no traço como instrução de verdade; `$02 $22 $42 $62` continuam `JAM`
- os instáveis `ANE`, `SHA`, `SHX`, `SHY`, `TAS`/`SHS` e `LAS`: os 256 bytes têm traço
- entrada de IRQ (`$FFFE`) e NMI (`$FFFA`), 7 ciclos, começando na fronteira

No CMOS, por cima dessa matriz, os bytes que o núcleo realmente executa como 65C02: `BRA`, `STZ`, `TSB`/`TRB`, `BIT` imediato e indexado, `INC A`/`DEC A`, `PHX`/`PHY`/`PLX`/`PLY`, `(zp)`, e `JMP (abs,X)`. No CMOS, `$9C` e `$9E` são `STZ` e são traçados; no NMOS esses mesmos bytes são `SHY`/`SHX` e ficam de fora.

`JMP ($nnnn)`: 5 ciclos no NMOS com o bug em `$xxFF`; 6 no CMOS, sem o bug. O ciclo extra do CMOS relê o byte alto do ponteiro. Isso não é o microcódigo da WDC.

`is_bus_cycle_stepped(op)` responde pela matriz NMOS/Ricoh, já com o `JAM`. `is_bus_cycle_stepped_variant` inclui o CMOS. Todo byte NMOS tem traço.

`Cpu::trace_bus_instruction` corre uma instrução inteira com `step_cycle`. `format_bus_trace` escreve `ADDR R|W DATA` por ciclo.

## 6551

`acia::Acia6551` é um holding register de um byte (TDRE, RDRF, overrun), com loopback opcional. `set_baud_period(n)` mais `tick` contam ciclos até TDRE voltar e até um byte de RX levantar RDRF. Com `n = 0` o comportamento é imediato. O nibble de baud do registrador de controle **é** copiado para o período (`BAUD_PERIOD_BY_NIBBLE`). Não há gerador de cristal: os números são contagens de `tick`, e `set_baud_period` ainda pode sobrescrever o valor.

`Cpu::step` não chama `tick`. `acia::run_cycles_with_acia` faz um `tick` depois de cada `step_cycle`.

## 6522

Há dois modelos.

`via::ViaTimer1` continua sendo só o timer 1 em one-shot, como na 1.1. Os testes antigos dependem disso.

`via::Via6522` é o chip usado pela placa nova:

- porta A e porta B, com DDR. Bit 1 no DDR é saída (lê o latch); bit 0 é entrada (lê o pino que o host colocou com `set_port_a_in` / `set_port_b_in`)
- timer 1 one-shot (ACR bit 6 limpo) e free-run (ACR bit 6 setado). O estouro liga o bit 6 do IFR. No free-run o contador recarrega o latch e continua. No one-shot para
- timer 2 one-shot no `tick`. Com o bit 5 do ACR, uma borda de descida em PB6 (`set_pb6`) também decrementa o mesmo contador e o estouro liga o bit 5 do IFR. O `tick` **não** para no modo pulso: os dois caminhos contam. Isso existe para não quebrar o one-shot da 5.0, que arma o ACR em `$20`
- IFR/IER no formato da 6522: escrever 1 no IFR limpa o flag; no IER o bit 7 escolhe set ou clear. O bit 7 do IFR sobe quando algum flag habilitado está ativo. É esse bit que a CPU lê
- CA1 e CB1 ficam em nível alto em repouso. A borda de descida liga o bit 1 (CA1) ou o bit 4 (CB1) do IFR **somente** se o bit igual do IER já estiver setado. IER desligado: a borda não liga o IFR. Não é o silício da 6522, em que o IFR sobe mesmo com o IER mascarado. Ler a porta não limpa esses flags. CA2/CB2 têm os modos de saída descritos no topo deste arquivo
- PB7 inverte no estouro do timer 1 só com ACR `$C0` (free-run e PB7). `pb7()` lê esse estado. Ele não é misturado na leitura do pino da porta B
- registrador de deslocamento em dois modos, os dois MSB primeiro, oito `tick`, bit 2 do IFR no oitavo: ACR `110` shift-out, e ACR `010` shift-in (`set_sr_in_bit` entra como LSB novo). Os outros modos não andam

A contagem é a mesma do `ViaTimer1`: um contador carregado com N estoura no N-ésimo `tick` (o tick que zera o contador). Latch 0 não estoura no tick seguinte, porque o contador vai para `$FFFF`. `Cpu::step` não chama `tick`.

## Placa com VIA e IRQ

`systems::via_board::ViaComputer` é uma máquina original. Não carrega ROM de Apple, KIM, NES, Commodore ou Microsoft. A ROM é o texto em `VIA_ROM_SOURCE`, montado pelo assembler do crate.

- RAM `$0000..=$7FFF`
- VIA `$C000..=$C00F`
- ROM `$E000..=$FFFF`

O firmware põe o bit 0 da porta A como saída, arma o timer 1, habilita a IRQ no IER e faz `CLI` / `JMP`. `ViaComputer::run_cycles` dá um `tick` na VIA e depois um `step_cycle`, e copia `irq_asserted()` para o pino de IRQ da CPU. O handler inverte o bit 0 do latch da porta A e lê o contador baixo do timer 1, o que baixa o flag. O teste observa esse bit. A IRQ só é aceita na fronteira de instrução.

## Terminal de eco

`systems::term::TermComputer` continua a máquina da 1.1: 32 KiB de RAM, 6551 em `$D000`, ROM montada de `examples/term_echo.asm`. Não é ROM histórica.

## Console 32×32 e a scanline

`graphics::FantasyConsoleBus` mantém o framebuffer em `$0200..=$05FF`, o RNG em `$00FE` e o teclado em `$00FF`. O contador de scanline fica em `$00FD`, valores `0..32`. `set_scanline` mascara com 31. `tick_scanline` soma um e volta a 0 depois de 31. O contador não anda sozinho dentro de `Cpu::step`: o host (ou o teste) chama `tick_scanline`. Um programa pode esperar o valor 0 com `LDA $00FD` / `BNE`.

A 50.0 acrescenta um blit de sprite, não uma PPU. Cada tile tem 64 bytes, 8×8, ordem de linha, na tabela que o host entrega com `set_patterns`. A cor 0 é transparente e não escreve o framebuffer. A CPU escreve o índice, X e Y:

- sprite 0: `$00F8` índice, `$00F9` X, `$00FA` Y. Escrever Y dispara o blit
- sprite 1: `$00F5` índice, `$00F6` X, `$00F7` Y. Escrever Y dispara o blit e recalcula a colisão

`$00F4` lê 1 só quando um pixel opaco (nibble diferente de zero) do sprite 1 cai no mesmo ponto da tela que um pixel opaco do sprite 0. Retângulos que se cruzam com pixels transparentes não ligam o flag. Pixel fora dos 32×32 é cortado. Não há scroll nem DMA.

## Montador

O pré-processador (`asm_pre.rs`) faz, nesta ordem: `.include` no mapa em memória, expansão de `.macro` (uma chamada dentro da outra funciona; dois níveis e `.macro` dentro de `.macro` são erro), `.rept`/`.endr`, `.if`/`.else`/`.endif`, `.incbin`, `.error`, `.warn`, e só então os labels locais e `.scope`/`.endscope`.

- `.rept N` … `.endr` repete o corpo N vezes. N é expressão constante (sem label). Acima de 4096 é erro. Zero repetições emite nada
- `.if` testa a expressão constante. Zero é falso; qualquer outro valor é verdadeiro. Número sem prefixo continua hexadecimal (`10` é dezesseis; `0d10` é dez)
- `.incbin "nome"` copia bytes. Procura primeiro o mapa binário de `assemble_with_blobs` (`HashMap<String, Vec<u8>>`) e, se não achar, os bytes UTF-8 da entrada no mapa de texto de `assemble_with_files`. Não há leitura de disco
- `.align N` na passagem do montador preenche com zeros até o PC ser múltiplo de N. N zero é erro
- `.error "texto"` aborta a montagem. O `AsmError` contém esse texto. Dentro de um `.if` falso não dispara
- `.warn "texto"` acrescenta a string em `AsmOutput::warnings` e a montagem segue. Dentro de um `.if` falso não grava
- `.scope` / `.endscope` abrem um espaço só para labels `@`. O mesmo nome lá dentro não é o `@` de fora, e um `jmp` de fora para um `@` que só existe lá dentro é erro. `.endscope` sem `.scope` (e o contrário) é erro

Expressões em operandos usam `*`, depois `+`/`-`, depois `<<`/`>>`, depois `&`, `^` e `|`, com parênteses, `hi()`/`lo()` e os prefixos `<` e `>`. `1+2<<2` é 12 e `1<<2+1` é 8. Número sem prefixo continua hexadecimal. `AsmOutput::listing` emite `ADDR: BYTES  linha-fonte`.

## Depurador

- breakpoint de PC antes da busca
- `break_when_a(valor)` para antes da próxima busca quando `A` já é esse valor (`StopReason::Accumulator`)
- `watch_pc_range(início, fim)` em `run`: para com `StopReason::PcRange` quando o PC já está no intervalo inclusive, antes da busca. Fora do intervalo não para
- `break_when_written(endereço, valor)` em `step_watched`: para com `StopReason::WatchValue` só se a instrução escreveu esse byte nesse endereço. Outro valor segue (`InstructionLimit` se não houver outro watch)
- watchpoints de leitura e escrita (`LoggingBus`, `step_watched`)
- anel de 32 snapshots **anteriores** à instrução. Cada frame guarda os registradores e um checksum FNV-1a de 64 bits sobre `peek($0000)` … `peek($FFFF)`. Não guarda uma segunda cópia de 64 KiB
- `step_back` restaura só os registradores. A memória escrita não volta. O checksum guardado deixa o teste ver que a RAM mudou

## Snapshot versões 2 e 3

`snapshot::MachineImage` continua JSON escrito à mão, sem serde. A versão 4 (`at_console`) acrescenta `palette` (32 dígitos hex) e `name_table` (512 dígitos hex). Versão 0 é rejeitada. `capture` grava a versão 2: registradores, `halted`, os 65536 bytes de RAM em hex, o estado de `Via6522` e o estado de `Acia6551` (holding registers, comando, controle, período e contagens; o histórico `taken` do host não entra). `at_scanline(linha)` promove a imagem para a versão 3 e grava `scanline` junto com o `halted` que já estava na versão 2. Falta o campo `version` e o parser rejeita. Versão 99 também. A versão 2 ainda carrega (`scanline` fica 0). A versão 3 exige `scanline`. Restaurar abandona uma sequência de `step_cycle` pela metade e limpa o latch de NMI. O pino de IRQ não faz parte da imagem. O `scanline` não é aplicado de volta num `Cpu<FlatMemory>`, porque essa memória não tem o registrador: ele volta no objeto da imagem.

## Outros blocos

`debug_api::DebugServer` escuta só em `127.0.0.1` com `std::net`. Os testes aceitam um número finito de conexões e derrubam o listener. Não deixe o `serve` rodando.

`audio::square_wave` devolve PCM `i16`. Não abre dispositivo de áudio.

Sistemas anteriores: Apple-1 com WozMon; Mini BASIC didático no host (não é ROM Microsoft); KIM-1 e Ben Eater parciais, sem ROM de monitor proprietária; o VIA da placa Ben Eater em `systems::beneater` continua o modelo de LCD, separado de `via::Via6522`.

## Ligador

`link::assemble_object` monta um módulo relocável: `.section`, `.org`, `.global`, rótulos, `.byte` e instruções implícitas, imediatas, absolutas e de desvio. Um operando que é nome vira relocação; um número é constante e não gera relocação. O desvio só aceita rótulo.

`link::link` é de duas passagens. Na primeira, cada seção com origem fica nesse endereço e cada seção sem origem ocupa o próximo intervalo livre a partir de `reloc_base`. Na segunda, a relocação absoluta grava o endereço do símbolo em little-endian e a relativa grava `alvo - (endereço_do_byte + 1)` em 8 bits com sinal. Fora de -128..=127 é erro. Um nome que não existe no próprio módulo nem como global de outro módulo é `símbolo indefinido`. O mesmo global em dois módulos é `símbolo global duplicado`. Intervalos de seção que se cruzam são `seções sobrepostas`.

`write_object` grava magic `M65O`, versão 1, seções, símbolos, notas de origem e a soma dos bytes anteriores. `read_object` rejeita checksum, versão e relocação que não cabe na seção. `Linked::line_at` usa essas notas já somadas ao endereço final da seção.

## Desmontador e mapa nomeado

`disasm::official_layout` devolve o mnemônico, o modo de endereçamento e o tamanho em bytes dos 151 opcodes oficiais NMOS que o núcleo executa. Um byte não oficial devolve `None`: a função não os lista como oficiais. `disasm_at` continua formatando o byte que está na memória, inclusive os ilegais que o núcleo realmente executa.

`bus::NamedMemory` registra regiões com nome, início e fim, inclusive. Início maior que o fim é erro. Uma região que cruza outra já registrada é erro e não é gravada. `load` copia bytes a partir do início só se couberem em `fim - início + 1`. Se o buffer passa do fim, o erro é `PastEnd` e a imagem não muda. Não é ROM de máquina nenhuma: é só um mapa de nomes sobre 64 KiB.

## Handshake, macros aninhadas e script do ligador

O PCR, e não o ACR, escolhe CA2 e CB2. CA2 são os bits 3–1. CB2 são os bits 7–5. O código de três bits é o mesmo nos dois pinos.

- `000` é handshake de entrada na borda de descida. `010` é handshake de entrada na borda de subida. A borda liga o bit 0 (CA2) ou o bit 3 (CB2) do IFR só se o bit igual do IER já estiver setado. Ler ou escrever a porta (`$01` ou `$00`) limpa esse flag. `$0F` não limpa.
- `001` e `011` são entrada independente, na descida e na subida. O acesso à porta não limpa o flag. Só a escrita no IFR limpa.
- `100` é handshake de saída. Ler ou escrever a porta baixa o pino. O `tick` de PHI2 não solta. A borda de descida de CA1 ou CB1 solta. O contador de pulsos não anda.
- `101` continua sendo o pulso na escrita, solto no `tick` seguinte. Não é o handshake.
- `110` segura baixo e `111` segura alto. Nos modos de entrada o pino não é dirigido: `ca2()` e `cb2()` devolvem o nível que o host colocou.

O timer 2 com o bit 5 do ACR conta só a borda de descida de PB6. Um `tick` nesse modo não decrementa o contador. Com o bit 5 limpo, o one-shot continua no `tick`.

Uma macro pode chamar outra até o quarto nível, contando a chamada que sai do programa. O quinto nível devolve `macro aninhada rejeitada: quinto nível` e a expansão para, inclusive quando a macro chama a si mesma. `.macro` dentro de `.macro` continua erro. Cada expansão renomeia os labels `@`, então o mesmo nome em duas chamadas, ou na macro de fora e na de dentro, não é o mesmo endereço.

O script do ligador é uma linha `section <nome> <endereço>` por seção. `link_with_script` coloca a seção nesse endereço. Seção fora do script continua com `.org` ou com `reloc_base`. Nome repetido no script, nome que não existe, ou endereço que discorda do `.org`, é erro. A nota de origem guarda o arquivo. Depois de ligar dois módulos, `line_at` devolve arquivo e linha daquele endereço. Um desvio relativo que não cabe em -128..=127 continua `desvio fora de alcance`.

O snapshot versão 5 (`at_next_instruction`) acrescenta `next_listing`, uma linha de desmontagem da instrução em `PC`. A versão 99 é rejeitada. As versões 2, 3 e 4 ainda carregam, com o listing vazio. `diff_image` lista, além dos registradores, o byte baixo e o byte alto do latch do timer 1 e do timer 2 quando eles mudam. O contador não entra nessa lista.

`examples/checksum.asm` soma `$0A + $14 + $1E` e guarda o total. O script põe o código em `$0600` e os dados em `$0030`. O resultado é `$3C`. Não é ROM de monitor.


## 2A03 e APU (`src/apu.rs`, `src/systems/nes2a03.rs`)

O 2A03 é o núcleo NMOS com o modo decimal desligado (`CpuVariant::Ricoh2A03`): a flag D existe, mas ADC/SBC são sempre binários. A APU é um modelo original: dois canais de pulso (duty de 4 formas, sweep com a diferença de complemento entre pulso 1 e 2, silêncio com período < 8 ou alvo > $7FF), triângulo com contador linear, ruído com LFSR de 15 bits (modo curto no bit 6), envelope, contador de comprimento e frame counter de 4 e 5 passos com IRQ. Registradores $4000–$4017; $4015 lê o status e limpa o IRQ de quadro. DMC com timer por taxa, busca de amostra pelo barramento da máquina numa janela de 4 ciclos em que a CPU só para nos ciclos de leitura (`Cpu::next_cycle_writes`): perde 4 ciclos numa leitura, menos quando a janela cai em escritas de JSR, RMW ou interrupção, IRQ, loop e volta de $FFFF para $8000. Mixer não linear pelas fórmulas documentadas. `NesBus` tem 2 KiB espelhados, a APU e PRG em RAM ou um cartucho.

## Mappers (`src/mapper.rs`)

Modelo da lógica de bancos do lado da CPU: NROM (16 KiB espelhado ou 32 KiB, PRG RAM opcional), UxROM (banco em $8000, último fixo em $C000, conflito de barramento = valor AND byte da ROM) e MMC1 (porta serial de 5 escritas, reset com bit 7, modos de PRG 0–3, habilitação da PRG RAM; escrita no ciclo seguinte a outra é ignorada, como no RMW). Tamanhos são validados (`MapperError`). As imagens saem do montador do projeto; não há ROMs.

## Depurador interativo (`src/repl.rs`)

REPL sobre `BufRead`/`Write` (stdin/stdout no binário, sem rede): break/delete, step, next (passa por cima de JSR), finish (sai da rotina), continue, regs, mem, dis, poke, watch, símbolos, profile e reset. Os testes alimentam scripts de comandos e conferem a saída.

## Símbolos (`src/symbols.rs`)

Formato `nome = $XXXX`, o mesmo de `AsmOutput::to_symbol_table`; carrega do montador, do linker (`.global`) ou de texto, com erros por linha. O desmontador (`disasm::format_named`) e o REPL mostram nomes e `nome+N` no lugar de endereços.

## Profiler e cobertura (`src/profiler.rs`, `src/coverage.rs`)

O profiler soma execuções e ciclos por PC e, seguindo JSR/RTS, ciclos inclusivos e exclusivos por rotina, com relatório de pontos quentes. A cobertura marca bytes executados como opcode ou operando, lista lacunas e desenha um mapa.

## Autoteste (`examples/selftest.asm`)

Programa de 14 testes no estilo Klaus, escrito no montador do projeto; cada teste grava seu número em $0200 e tem armadilha própria. Passa no NMOS; no 2A03 para no teste 5 (decimal) e no 65C02 no teste $13 ($A7 é NOP).

## Opcodes indefinidos do 65C02

Os bytes sem instrução no 65C02 são NOPs documentados (`opcodes::cmos_nop`): `x3/x7/xB/xF` 1 byte e 1 ciclo; `x2` 2 bytes e 2 ciclos; `44` 3 ciclos; `54/D4/F4` 4 ciclos; `DC/FC` 3 bytes e 4 ciclos; `5C` 8 ciclos. O passo por instrução e o passo por ciclo usam a mesma tabela. `x7`/`xF` são RMB/SMB (5 ciclos: lê, lê de novo, escreve) e BBR/BBS (5 ciclos, +1 se desvia, +1 se cruza página), como nos 65C02 Rockwell/WDC; no NMOS os mesmos bytes seguem ilegais (SLO, RLA...). O montador aceita `bbr3 zp,destino`.

## Ferramentas (`scripts/`)

`ci.sh`/`ci.ps1` repetem o CI (fmt, clippy, testes, release) e rodam as verificações abaixo; o código de saída é o número de etapas que falharam. `examples.sh`/`.ps1` montam e executam cada linha de `scripts/examples.tsv` e comparam a memória. `check_versions.sh`/`.ps1` conferem Cargo.toml, CHANGELOG e `--version`. `package.sh`/`.ps1` geram o `.tgz` sem `target/` e `.git/` e verificam o conteúdo. `report.sh`/`.ps1` contam linhas e testes. `smoke.sh` confere os dois binários de demonstração. `mirror.ps1` copia a árvore preparada para o destino e move (sem apagar) para uma pasta à parte os arquivos que só existem no destino. Os `.ps1` são UTF-8 com BOM. `tests/scripts.rs` executa os scripts bash.

## Formatos, PIA, fita e histograma (`hexfmt`, `pia`, `tape`, `profile`)

`hexfmt` lê Intel HEX e lê/escreve S-record S1/S9 com checksum. `pia` modela o 6820/6821 (DDR ou porta por CRx bit 2, bordas de CA1/CB1 com IRQ). `tape` codifica Kansas City Standard como marcas 0/1. `profile` é um histograma de opcodes e mnemônicos sobre o `Profiler`, sem repetir as contagens por PC.

## Mapa de memória (`bus`)

`SystemBus::memory_map` devolve as regiões RAM/ROM/dispositivo em ordem, cobrindo os 64 KiB sem buracos; `FlatMemory::try_load_at` recusa imagem que passaria de $FFFF.

## Fase de aprofundamento (formatos, PIA, fita, snapshot)

- `asm::operand_syntax` é o único classificador de operandos; `link` o usa e herda todos os modos, incluindo BBR/BBS (`zp,alvo` com relocação relativa) e símbolos de página zero (`AbsoluteByte`).
- 65C02: o ciclo intermediário de RMW é uma leitura (NMOS escreve o valor antigo); `next_cycle_writes` acompanha.
- `Nes2A03::dma_window` guarda a janela de DMA do DMC entre chamadas de `run_cycles`.
- `snapshot::SnapshotError` tipa todos os erros de imagem; `hexfmt::HexError` traz linha e motivo (checksum, conflito, passa de $FFFF, contagem S5, endereço largo).
- `tape`: enquadramento assíncrono (bits ociosos antes do start), erros tipados e `to_pcm` em 1200/2400 Hz.
- `pia`: mapa completo do registrador de controle do 6821, CA2/CB2 como entrada, saída manual, handshake e pulso.
- `acia::stop_halves`: 1, 1,5 ou 2 stop bits conforme o data sheet.

## Terminal (`systems::term`): editor de linha

A ROM `examples/term_echo.asm` (original, `$E000`) liga o DTR da 6551 e edita uma linha de até 64 bytes:

- `$20..=$7E` e `$80..=$FF`: guardados em `$0200+len` e ecoados; com a linha cheia, BEL (`$07`).
- BS (`$08`) ou DEL (`$7F`): apagam o último byte e ecoam BS, espaço, BS; com a linha vazia, BEL.
- CR (`$0D`): ecoa CR LF e executa BRK. A linha fica em `$0200`, o comprimento em `$10`.
- Outros controles: BEL.

`TermComputer::send` entrega um byte e roda até a ROM voltar a `getc` com o RX vazio, devolvendo todo o eco; `type_bytes` digita vários; `line()` lê o buffer. Erros tipados em `TermError`: `Halted` (já passou do CR), `Timeout { byte }` e `Cpu`. Números sem prefixo no assembler são hexadecimais, por isso a ROM compara com `#$40`.

## Cartuchos iNES / NES 2.0 (`ines`) e AxROM

`Ines::parse` valida assinatura, tamanhos (inclusive a forma expoente-multiplicador do NES 2.0), trainer de 512 bytes, PRG/CHR, e guarda bytes extras em `trailing`. Cabeçalhos antigos com lixo nos bytes 12–15 ("DiskDude!") ignoram o byte 7. Erros tipados em `InesError` (`TooShort`, `BadMagic`, `Truncated { need, have }`, `NoPrg`, `SizeOverflow`, `UnsupportedMapper`, `Mapper`). `to_bytes` serializa de volta; `cartridge()` monta NROM (0), MMC1 (1), UxROM (2) ou AxROM (7). `mos6502 ines ARQ.nes` mostra o cabeçalho. Nenhum jogo acompanha o projeto; os testes constroem os arquivos.

`Axrom`: banco de 32 KiB pelos bits 2–0 de qualquer escrita em `$8000`–`$FFFF` (módulo o número de bancos), bit 4 escolhe a página de nametable (guardada, sem PPU). Sem conflito de barramento (ANROM).

`Latch` modela placas de lógica discreta com conflito de barramento (byte escrito AND byte da ROM): CNROM (3), PRG fixa como NROM e banco de CHR de 8 KiB pelos bits 1–0; GxROM (66), PRG de 32 KiB pelos bits 5–4 e CHR pelos bits 1–0. Os bancos de CHR ficam registrados (sem PPU).

## PPU 2C02 (`ppu`) e MMC3

Modelo original do 2C02 NTSC: registradores `$2000`–`$2007` (espelhados até `$3FFF`) com latch de barramento aberto, registradores loopy `v`/`t`/x fino/`w`, buffer de leitura de `$2007` (paleta imediata, buffer recebe a nametable de baixo), OAM de 256 bytes (bits 4–2 do atributo não existem), DMA de OAM em `$4014` (513 ciclos de CPU, 514 se a escrita cair em ciclo ímpar), 4 KiB de nametable dobrados por `Mirroring` (horizontal, vertical, quatro telas, tela única baixa/alta) e paleta de 32 bytes com os espelhos `$3F10/14/18/1C`.

Temporização: 341 pontos × 262 linhas, vblank na linha 241 ponto 1, NMI na borda (inclusive ao ligar o bit 7 dentro do vblank), leitura de `$2002` no ponto anterior suprime flag e NMI do quadro, linha pré-render limpa as flags e quadros ímpares com renderização têm um ponto a menos. Os dados de cada linha visível são buscados no ponto 1 a partir de `v` e o pixel `x` é composto no ponto `x + 1` com a máscara e a paleta daquele ponto (mudanças no meio da linha dividem a linha) (fundo com atributos e scroll fino, sprites 8×8 e 8×16 com flip, prioridade e recorte dos 8 pixels da esquerda). Sprite 0 hit é marcado no ponto do primeiro pixel opaco sobreposto (nunca em x = 255); overflow com a varredura defeituosa do hardware (depois do oitavo sprite o índice de byte avança junto, gerando falsos positivos e negativos). Ênfase por pixel em `emphasis`; `ntsc_rgb` calcula RGB a partir de um modelo do sinal composto (níveis de luma, 12 fases, YIQ), sem arquivo de paleta capturado. O framebuffer guarda índices de paleta de 6 bits.

Os mappers expõem `chr_offset`, `mirroring`, `ppu_address` e `irq`: MMC1 (CHR 4/8 KiB e espelhamento pelo controle), AxROM (tela única), CNROM/GxROM (banco de CHR). `Mmc3` (mapper 4): bancos de PRG de 8 KiB em dois modos, CHR em 1/2 KiB com inversão, espelhamento, proteção de PRG RAM e contador de IRQ estilo Sharp clocado pelas bordas de A12 (com filtro: A12 baixo por pelo menos 3 ciclos de CPU) que a PPU sinaliza uma vez por linha renderizada (ponto 260 com sprites em `$1000`, 324 com fundo em `$1000`). No `Nes2A03` a PPU avança 3 pontos por ciclo, o NMI vai para a CPU e o IRQ é o OU do APU com o cartucho; `from_ines` carrega CHR ROM (somente leitura) e o espelhamento do cabeçalho.

## Expansões: VIA, APU, ACIA, assembler e depurador

- **VIA:** latch de entrada (ACR bit 0 para PA na borda ativa de CA1, bit 1 para PB na de CB1); `ira()`/`irb()` devolvem o valor travado (em PB os bits de saída seguem ORB). Os latches entram no estado e no snapshot.
- **APU:** escrita em `$4017` reinicia o sequenciador 3 ciclos depois (ciclo par) ou 4 (ímpar); modo e inibição valem na hora; `frame_reset_pending()` mostra o atraso.
- **ACIA:** receptor de linha com sobreamostragem 16x (`rxd_16x`): início confirmado no meio do bit (glitches curtos ignorados), dados LSB primeiro, paridade e primeiro stop amostrados no meio; erros de paridade e enquadramento nos bits 0/1 do status. `frame_levels` gera o sinal de um quadro.
- **Assembler:** `.lobytes`/`.hibytes` (tabelas divididas), `.dbyt` (palavras big-endian) e `.assert expr[, "msg"]`, avaliada na passada final.
- **Depurador:** condições (`cond`): comparações `== != < <= > >=`, `&&`/`||`, registradores `A X Y SP P PC`, flags `N V B D I Z C`, `CYC` e leitura de memória `[e]` sem efeitos colaterais; erros tipados (`CondError`). `break_if` (breakpoint condicional), `break_when` (condição global, `StopReason::Condition`), contagem de passagens `hits` e `ignore` por endereço.

## Estado completo do NES (`nes_state`)

`Nes2A03::save_state` grava CPU (inclusive a sequência de ciclos em curso), RAM,
PPU (registradores, VRAM, OAM, paleta, `v/t/x/w`, buffers de linha, framebuffer),
APU (todos os canais, sequenciador e reinício pendente), mapper (registradores,
PRG RAM, contador IRQ do MMC3) e CHR RAM. ROM não é gravada: PRG/CHR ROM levam
uma impressão FNV-1a que precisa bater ao carregar. Versão 2 atual; a versão 1
(sem framebuffer e buffers de linha) ainda carrega. Erros tipados em
`NesStateError`. Os testes cortam a execução em dezenas de ciclos por mapper e
provam continuação idêntica.

Também nesta fase: `.charmap` no pré-processador, `.faraddr/.dword/.pstring`,
transmissor TxD bit a bit do 6551 (`txd_16x`), RES e `ticks_until_irq` do 6522,
altura de tom por canal, reamostragem exata e a cadeia de filtros de saída do APU,
símbolos do ligador (`define`/`provide`/seções), mapa, imagem e bancos (overlays).

## Escapes de string

`escape.rs` decodifica `\" \' \\ \n \r \t \0 \xHH` em todas as diretivas de
string e em literais de caractere; o montador e o pré-processador ignoram `;`,
`:`, `=` e `,` dentro de strings e literais.

## Decodificação de registradores, log do APU e execução reversa

`via::Acr`/`via::Pcr` e `acia::AciaConfig` decodificam os registradores de controle
conforme as folhas de dados (testados sobre todos os valores). `apu::RegisterLog`
grava escritas com ciclo e as reproduz de forma exata, com formato texto e erros
tipados. O depurador ganhou histórico configurável, `run_recording` e
`reverse_continue`; a CPU ganhou `run_until`, `run_to_pc` e `run_to_return`.

## Commodore 64, SID 6581, Servidor GDB RSP e Analisador Estático

- **SID 6581 / 8580 (`src/sid.rs`):** Sintetizador analógico clássico com 3 vozes independentes (frequência de 16 bits, largura de pulso de 12 bits), 4 formas de onda (triangular, dente de serra, pulso/retangular e ruído por LFSR de 23 bits), modulação em anel, sincronização dura (hard sync), gerador de envelope ADSR autêntico com curvas exponenciais de hardware, filtro ressonante multimodo (passa-baixas, passa-faixa, passa-altas, notch) e renderizador PCM mono de 16 bits.
- **Commodore 64 (`src/systems/c64.rs`):** Computador C64 completo com emulação da porta de processador 6510 em `$0000`/`$0001` (LORAM, HIRAM, CHAREN), barramento PLA gerenciando RAM, BASIC ROM (`$A000..$BFFF`), KERNAL ROM (`$E000..$FFFF`), Character ROM e área de E/S (`$D000..$DFFF`), matriz de texto PETSCII 40×25 em `$0400..$07E7`, Color RAM de 1 KiB e chip de som SID integrado em `$D400..$D7FF`.
- **Servidor GDB RSP (`src/gdb.rs`):** Servidor de depuração remota via Remote Serial Protocol (RSP) em TCP loopback (`127.0.0.1:PORT`). Suporta conexão com ferramentas profissionais como GDB multiarch, Ghidra e IDA Pro para inspeção e escrita de registradores (`g`/`G`), memória (`m`/`M`), passo único (`s`), execução contínua (`c`) e inserção/remoção de breakpoints de software (`Z0`/`z0`).
- **Analisador Estático e CFG (`src/analyzer.rs`):** Analisador estático de código binário que reconstrói o grafo de fluxo de controle (CFG), detecta destinos de desvios, chamadas de sub-rotinas (JSR), pontos de retorno (RTS) e identifica blocos de dados ou código morto inatingível. Exposto na CLI pelo subcomando `mos6502 analyze <BIN> [--base <ADDR>]`.
- **Atari 2600 VCS e TIA (`src/tia.rs`, `src/systems/atari2600.rs`):** Emulação do console Atari 2600 com CPU 6507 (espaço de 8 KiB), chip de vídeo e áudio TIA (sincronização de linha WSYNC, VBLANK, VSYNC, cores de playfield/players, 2 geradores de áudio polinomiais de 4 bits e tabela de 15 colisões com CXCLR) e chip RIOT 6532 (128 bytes de RAM e timers com divisores 1/8/64/1024).
