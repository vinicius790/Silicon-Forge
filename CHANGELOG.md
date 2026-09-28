# Changelog

O formato segue [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/) e o projeto usa [SemVer](https://semver.org/lang/pt-BR/).

## [Unreleased]

### Added
- NES: discos FDS (`parse_fds`, `Nes2A03::from_fds`); placas MMC3/MMC6 (`Mmc3Board`), pulsos de áudio e split do MMC5, VRC7; Four Score, Arkanoid e Power Pad em `input`; instantâneo NES versão 7 (versões antigas carregam).
- Assembler: aliases de opcodes não documentados (6502X), `.repeat`/`.rept`, `.scope`/`.local`, `.ifconst`/`.ifref`, `.concat`/`.left`/`.mid`/`.right`/`.sprintf`, `.macpack`.
- Linker: mapa estilo ld65 (`map_ld65`) e objetos o65 (`O65File`); depurador: intervalos de watch e backtrace (`backtrace`, `format_backtrace`).
- CPU: ciclo morto de RMW configurável (`RmwDeadCycle`); KIM-1: rotinas TTY e fita do monitor; RIOT 6530 vs 6532.
- `systems::sbc`: computador de placa única descrito em texto (RAM, ROM, PIA/VIA/ACIA/RIOT, IRQ).
- PNG (deflate/inflate próprios, `png_rgb`, `decode_png_rgb`) e PPM; WAV 8/16 bits e estéreo (`WavFormat`, `parse_wav_any`).
- Bench: suíte com linha de base e detecção de regressão (`run_suite`, `compare`); cobertura por arquivo em LCOV.
- Formatos: C64 PRG (com stub SYS) e Atari XEX.
- CLI: `nes` (NTSC/PAL/Dendy), `kim1`, `board` (RAM + 6522 em $C000 + 6551 em $D000, reset em $8000 e IRQ em $8003), `bench-suite`, conversões prg/xex/mos.
- 6522/6551: correções de shift sob CB1/T2 e linha RxD ociosa em nível alto.
- Exemplos: `basic_tokenizer`, `float_lite`, `forth_lite`, `lz_decompress`, `wozmon_lite` (código original); `via_clock` e `acia_irq_echo` rodam no `board`.
- Mappers: MMC5 (5) e VRC6 (24/26) com áudio; áudio de expansão do Sunsoft 5B e do Namco 163 misturado por `mix_expansion`; instantâneo NES versão 6 (versões antigas carregam).
- NES: bateria `.sav` (`save_ram`, `load_save_ram`, `write_sav`, `read_sav`, `SaveRamError`), `audio_sample`/`render_audio`, paleta PAL (`pal_rgb`) e refresh de OAM.
- Assembler: `.setcpu` com `AsmCpu` (rejeita opcodes de outra CPU) e arquivo de listagem.
- Linker: bibliotecas (`Library`, `link_with_libraries`) e configuração de memória/segmentos (`MemoryArea`, `SegmentRule`).
- Depurador/REPL: `memory_diff`, `snap`/`diff`, `watch EXPR if COND`, `trace PATH`, `source PATH`, `histfile`/`commands`.
- CPU: o estado de ciclo grava o fabricante do 65C02 (byte 30).
- KIM-1: placa com mapa de RIOT, vetores e rotinas de gravação.
- Instantâneo compactado `M65Z` (LZ + CRC32, `pack`, `PackError`).
- Hex: formato MOS Technology e imagens de 32 bits; fita: decodificação KCS de PCM e tempos do Apple-1.
- Cobertura por linha de fonte e exportação LCOV; profiler com arestas de chamada, formato folded e grafo DOT.
- CLI: subcomandos `assemble`, `link`, `run`, `trace`, `profile`, `coverage` com `USAGE` e `dispatch`.
- 6522: variantes NMOS/CMOS (`ViaVariant`) e saída de IRQ (`IrqOutput`); 6551: variantes, falsos starts e detecção de break.
- Exemplos: `crc16`, `muldiv`, `sieve_256`, `via_clock`, `acia_irq_echo`, verificados por `scripts/examples.sh`.
- NES: regiões NTSC, PAL (RP2C07/RP2A07) e Dendy (`ppu::Region`, `Nes2A03::set_region`), escolhidas pelo byte 12 do NES 2.0; razão PPU/CPU, linhas e vblank por região; `run_frames`.
- APU: tabelas PAL de ruído, DMC e sequenciador de quadro (`NOISE_PERIODS_PAL`, `DMC_RATES_PAL`, `Apu::set_pal`).
- Mappers: VRC2/VRC4 (21, 22, 23, 25, `VrcWiring`), VRC1 (75), Camerica (71), Namco 163 sem áudio (19), Sunsoft FME-7 sem áudio (69); `Cartridge::ppu_target` para CIRAM/CHR mapeados.
- NES: leitura parada da DMA do DMC repete acessos a `$2002`/`$2007`; instantâneo NES versão 5 (versões antigas carregam).
- Assembler: `.proc`/`.endproc`, `.incbin`, `.ifblank`/`.ifnblank`, `.paramcount`, `.len`, `.export`.
- Linker: `collect_garbage` (remove seções não alcançadas) e relatório de overlays (`overlays`, `overlay_report`).
- Disassembler: rótulos automáticos de alvos de código (`code_targets`, `to_source_with`).
- REPL: `back`/`rc` desfazem a contagem de passagens e de `ignore`; `rc` respeita `cond` e `ignore` em chegadas reversas.
- CPU: modelos 65C02 `CmosModel` (WDC, Rockwell, Synertek) com o conjunto de opcodes de cada um.
- RIOT 6532 (`Riot6532`, exportado na raiz): portas A/B, temporizador de intervalo e flag.
- Áudio: `parse_wav` lê WAV PCM 16 bits.
- APU: o log de registradores aceita a entrada `$1F` (byte de amostra do DMC).
- Instantâneo v8 com `cycle_state` (36 bytes): retoma no meio de instrução, de sequência de IRQ ou entre metades de ciclo; rejeita 9; v2–v7 carregam.
- 6522: modelo de carga dos pinos (pull-up em entrada solta, carga open-drain; na porta A vence a saída alta), `drive_port_a/b`, `port_a_pins`/`port_b_pins`.
- `PhaseCheckBus`: recusa acesso ao barramento fora da janela de PHI2.
- 6522: `tick` é `tick_phase(Phi2)` mais `tick_phase(Phi1)`.
- `SystemBus` repassa `MemoryBus::phase` a cada dispositivo mapeado (`IoDevice::phase`); `Via6522` implementa `IoDevice`.
- Instantâneo: flags de timer segurados da 6522 (opcionais) e versão 7 com o PHI1 aberto na fronteira de instrução.
- `MemoryBus::phase` e `tick_phase` na 6522 e na 6551: sob `step_phase` os dispositivos andam por fase e o flag dos timers aparece no PHI1 de N+1,5.
- `step_phase` acha o endereço do PHI1 via `peek`, sem copiar o barramento nem exigir `Clone`.
- 6551: clock 16x externo no nibble 0 (`set_external_clock`, `clock_16x`), salvo no instantâneo.
- 6551: break corta o byte em transmissão e segura TxD baixo (`txd_high`) até acabar.
- 6522: o T2 temporizado liga o flag N+1 ticks depois de carregar N (N+1,5 do datasheet); a contagem de pulsos continua no zero.
- 6551: `set_crystal` usa o cristal de 1,8432 MHz e o divisor 16x, e um quadro dura 16 clocks por bit.
- 6551: saídas `rts_asserted`/`dtr_asserted`, break do transmissor (`tx_break`) e `receive_break`.
- 6551: níveis de DCD/DSR/CTS, fila de eco e modo cristal no instantâneo (opcionais para imagens antigas).
- CPU: `step_phase`, relógio de duas fases com o acesso ao barramento no PHI2 e o resultado de `step`.
- 6522: o T1 liga o flag N+1 ticks depois de carregar N (N+1,5 do datasheet) e o free-run tem período N+2.
- 6522: shift pelo T2 (`001`, `101`) a cada 2(N+2) PHI2 com flag no oitavo bit, e saída livre (`100`) que gira o byte sem flag.
- 6522: ler o SR apaga o bit 2 do IFR e, nos modos de entrada, começa mais oito bits.
- 6551: DTR desligado e `/DCD` alto desligam o receptor; DCD e DSR aparecem nos bits 5 e 6 e uma mudança gera IRQ.
- 6551: `/CTS` alto segura o transmissor.
- 6551: modo eco devolve cada byte recebido por `pop_echo`.
- Exemplo `term_echo.asm`: liga o DTR da 6551 antes do primeiro byte.
- CPU: NMI nos quatro primeiros ciclos de `BRK` ou da entrada de IRQ sequestra o vetor para `$FFFA`, mantendo o B empilhado.
- CPU: numa CPU nova, depois do reset ou de restaurar, a borda de NMI/IRQ cai no primeiro opcode; no `WAI` o ciclo que acorda ainda é ocioso.
- 6522: ler ou escrever ORA/ORB limpa CA1/CB1; `$0F` não.
- 6522: o bit 0/4 do PCR escolhe a borda ativa de CA1/CB1, e o flag liga mesmo com o IER desligado.
- 6522: os contadores seguem decrementando depois do estouro do disparo único, sem ligar o flag de novo.
- 6551: comando `$02` e status `$10` depois do reset de hardware.
- 6551: bit 7 do status e `irq_asserted` para RDRF e TDRE habilitados; a leitura do status apaga.
- 6551: o reset programado mantém os bits de paridade 5–7.
- Instantâneo versão 6 grava `waiting` do `WAI`; `STP` segue no `halted`. As versões 2 a 5 carregam com `waiting` falso; a 99 é recusada.
- CPU: NMI usa o ponto de amostragem da IRQ; uma borda só no último ciclo, ou depois do operando de um desvio tomado sem cruzamento, espera mais uma instrução.
- CPU: `ARR` `$6B` põe C no bit 6 e V em bit 6 xor bit 5; no decimal NMOS corrige os nibbles e tira C do nibble alto.
- CPU: instáveis NMOS `SHA` `$93`/`$9F`, `SHX` `$9E`, `SHY` `$9C` e `TAS` `$9B` gravam `reg & (H+1)` e, com cruzamento de página, usam esse valor como byte alto do endereço; em `step` e `step_cycle`.
- CPU: `ANE` `$8B` usa `ANE_MAGIC = $EE` e `LAS` `$BB` faz `SP & M` em A, X e SP; os dois entram em `step_cycle`.
- CPU: 65C02 `WAI` `$CB` dorme até IRQ ou NMI; com `I` setado a IRQ acorda sem vetor.
- CPU: 65C02 `STP` `$DB` para até o reset.
- CPU: `step_half_cycle` mostra o endereço em PHI1 e o dado em PHI2 sem mudar o resultado de `step`.
- CPU: IRQ amostrada no penúltimo ciclo; a que sobe só no último ciclo espera mais uma instrução.
- CPU: desvio tomado sem cruzamento de página adia a IRQ uma instrução.
- ALU: decimal NMOS com N e V da soma intermediária e Z binário no `ADC`; `SBC` com flags binárias.
- ALU: decimal 65C02 com N e Z do resultado e um ciclo a mais.
- 6551: `peek` do status mostra o overrun sem apagá-lo. Só a leitura do status apaga.
- 6522: escrever no IER com o bit 7 limpo desliga só os bits indicados, e a leitura devolve o bit 7 em 1.
- Montador: `.byte <$1234, >$1234` emite `34 12`.
- Desmontador: `D0 EE` em `$8010` mostra o destino absoluto `$8000`.
- Montador: `LDA #>($FF)` emite `A9 00`. `LDA #<($FF)` emite `A9 FF`.
- 6522: o timer 1 de disparo único não inverte PB7 no estouro. O free-run com ACR `$C0` continua invertendo a cada reload.
- Depurador: `break_when_flag_becomes_set` de Z não dispara quando Z passa de setado para limpo.
- Leitor de objeto: duas seções com a segunda em comprimento declarado diferente não devolvem seção nenhuma, nem a primeira.
- Montador: `LDA #>($FF+1)` emite `A9 01`. `LDA #<($FF+1)` continua `A9 00`.
- 6522: o timer 1 em free-run liga o bit 6 do IFR a cada reload. Depois de limpar o flag, o reload seguinte liga o bit 6 de novo; o disparo único não liga.
- Depurador: `run_until_halt` num programa que faz BRK no último ciclo permitido devolve a parada de BRK, não `CycleCap`.
- Ligador: relocação absoluta de palavra (baixo, alto) de `$1234` grava `34` e depois `12`.
- Montador: `LDA #<($FF+1)` emite `A9 00`.
- 6522: o timer 2 de disparo único ao chegar a zero liga o bit 5 do IFR uma vez. Limpar o bit e outro tick de PHI2 não liga o flag de novo.
- Depurador: `step_over` de um JSR que retorna exatamente no último ciclo permitido devolve esse retorno, não `CycleCap`.
- Instantâneo: um byte diferente na paleta é reportado. O mesmo conjunto de registradores não entra no diff.
- Montador: `LDA #>$1234` emite `A9 12`. `LDA #<$1234` emite `A9 34`.
- 6522: o timer 1 em free-run recarrega o contador a partir do latch. O flag de interrupção permanece setado depois desse reload até ser limpo.
- Depurador: `step_out` com teto de ciclos menor que o retorno para como `CycleCap` e não afirma que retornou.
- Ligador: o endereço `$10` e o decimal `16` no script colocam a seção no mesmo endereço.
- Montador: `LDA #hi($FFFF)` emite `A9 FF`. `LDA #lo($FFFF)` emite `A9 FF`.
- 6522: o timer 1 de disparo único ao chegar a zero liga o bit 6 do IFR e o resumo uma vez. Um segundo tick de PHI2 nesse modo não liga o flag de novo depois de limpo.
- Depurador: `run_until_halt` com um intervalo de PC armado para com o motivo do intervalo antes do teto de ciclos quando o intervalo é atingido primeiro.
- Gravador de objeto: relocação relativa cujo deslocamento é o comprimento da seção é rejeitada e nenhum byte é gravado.
- Montador: `LDA #lo($100)` emite `A9 00`. `LDA #hi($100)` emite `A9 01`.
- 6522: contagem de pulsos do timer 2 ao chegar a zero liga o bit 5 do IFR e o bit 7 (resumo). Um tick de PHI2 depois disso não altera o contador.
- Depurador: intervalo de PC e watch de leitura armados juntos; uma leitura fora do intervalo para como leitura, não como o intervalo de PC.
- Ligador: uma seção em `$0000` e outra em `$0002`, um byte cada, deixa o endereço `$0001` em 0.
- Montador: `LDA #lo($00FF)` emite `A9 FF`. `LDA #hi($00FF)` emite `A9 00`.
- 6522: contagem de pulsos do timer 2 não liga o bit 5 do IFR até a contagem chegar a zero. Um tick de PHI2 nesse modo ainda não decrementa o contador.
- Depurador: limpar os dois intervalos de PC faz entrar no intervalo antigo não parar.
- Instantâneo: carga da v5 rejeita `next_listing` ausente. Um v5 completo ainda carrega.
- Montador: `LDA #lo($1234)` emite `A9 34`. `LDA #lo('A')` continua erro.
- 6522: fim do shift-in (8 bordas de CB1 com ACR 011) liga o bit 2 do IFR e o bit 7 (resumo). Limpar o bit 2 deixa o resumo se o timer 1 ainda está pendente.
- Depurador: intervalo de PC de um endereço (lo == hi) para só nesse PC.
- Leitor de objeto: checksum zero com corpo não vazio é rejeitado.
- Montador: `LDA #hi('A')` é erro porque um caractere não é rótulo de 16 bits. `LDA #hi($1234)` continua `A9 12`.
- 6522: fim do deslocamento (8 bordas de CB1 com ACR 111) liga o bit 2 do IFR e o bit 7 (resumo). Limpar o bit 2 deixa o resumo se o timer 1 ainda está pendente.
- Depurador: dois intervalos de PC; entrar no segundo para com os limites desse intervalo, não os do primeiro.
- Instantâneo: duas imagens com os mesmos registradores e um byte diferente na name table reportam essa mudança e não um registrador.
- Montador: `LDA #lo('A')` é erro porque um caractere não é rótulo de 16 bits.
- 6522: handshake de saída em CB2 liga o bit 3 do IFR e o bit 7 (resumo). Limpar o bit 3 deixa o resumo se o timer 1 ainda está pendente.
- Depurador: parada por intervalo de PC não reporta watch de escrita quando a instrução que entrou no intervalo não escreveu.
- Ligador: duas seções com um buraco; o byte do buraco é 0 e o primeiro byte da segunda é o opcode dela, não o da primeira.
- Montador: `JMP 'A'` é erro porque um caractere não é endereço. `LDA #'A'` continua `A9 41`.
- 6522: handshake de entrada em CA2 liga o bit 0 do IFR e o bit 7 (resumo). Limpar o bit 0 deixa o resumo se o timer 1 ainda está pendente.
- Depurador: intervalo de PC e watch de escrita armados juntos; uma escrita fora do intervalo para como escrita, não como o intervalo de PC.
- Leitor de objeto: relocação cujo deslocamento passa dos bytes da seção é rejeitada e nenhuma seção é devolvida.
- Montador: `STA 'A'` é erro porque um caractere não é endereço. `LDA #'A'+'B'` continua erro e `LDA #'A' ; nota` continua `A9 41`.
- 6522: borda ativa de CB1 liga o bit 4 do IFR e o bit 7 (resumo). Limpar o bit 4 deixa o resumo se o timer 1 ainda está pendente.
- Depurador: remover `break_when_written` faz uma escrita posterior nesse endereço não parar.
- Ligador: relocação relativa para um símbolo de outra seção, dentro do alcance, fica patchada.
- Montador: `LDA #'A' ; nota` emite `A9 41`. Um comentário não consome o caractere.
- 6522: borda ativa de CA1 liga o bit 1 do IFR e o bit 7 (resumo). Limpar o bit 1 deixa o resumo se o timer 1 ainda está pendente.
- Depurador: `break_when_written` num endereço não para quando outro endereço é escrito.
- Objeto: duas seções com o mesmo nome são rejeitadas e nenhum byte de arquivo é escrito.
- Montador: `LDA #'AB'` é erro (caractere maior que um). `LDA #'A'` continua `A9 41`.
- 6522: interrupção do timer 2 liga o bit 5 do IFR e o bit 7 (resumo). Limpar o timer 2 não apaga o resumo se o timer 1 ainda está pendente.
- Depurador: remover um watch de leitura faz uma leitura posterior desse endereço não parar.
- Ligador: endereço de seção no script que invade a faixa já colocada de outra seção é erro e não produz imagem.
- Montador: `LDA #''` é erro (caractere vazio). `LDA #'A'` continua `A9 41`.
- 6522: interrupção do timer 1 liga o bit 6 do IFR e o bit 7 (resumo de IRQ), além do flag do timer. Limpar o bit do timer limpa o resumo quando não há outra interrupção pendente.
- Depurador: watch de escrita e watch de leitura no mesmo endereço; uma leitura para com o motivo de leitura, não o de escrita.
- Leitor de objeto: nome de comprimento 0 é rejeitado e nenhuma seção é devolvida.
- Montador: `LDA #'A'+'B'` é erro. Duas constantes de caractere não se somam. `LDA #'A'+1` continua `A9 42`.
- 6522: escrever 0 no bit do timer 1 do IFR não limpa o flag.
- Depurador: dois watches de leitura; a leitura do segundo endereço para e nomeia esse endereço, não o primeiro.
- Ligador: lista de seções vazia é erro e não produz imagem.
- Montador: `LDA #'A'+1` emite `A9 42`. A precedência dos operadores continua a de hoje.
- 6522: ler o IFR não limpa o flag do timer 1. Escrever 1 nesse bit do IFR limpa o flag.
- Depurador: watch de leitura não para quando um endereço diferente é lido.
- Objeto: nome de seção com mais de 16 bytes é rejeitado e nenhum byte de arquivo é escrito.
- Montador: `LDA #'A'` emite `A9 41`. Aspas simples sem fechar é erro.
- Depurador: watch de leitura num endereço para só nessa leitura e não numa escrita nele.
- Snapshot v5: campo `next_listing` que não é string é rejeitado. Um v5 válido ainda carrega.
- Depurador: `step_over` num JSR cujo callee não volta para no teto de ciclos (`CycleCap`) e não finge que houve RTS.
- 6551: um terceiro byte de loopback com dois ainda não lidos liga overrun; o receptor fica no segundo até a leitura, que limpa overrun.
- Montador: `.byte "A","B"` emite 65, 66. Sem vírgula entre elementos é erro.
- Ligador: script que nomeia a mesma seção duas vezes é erro e não produz imagem.
- Depurador: `run_until_halt` com teto 0 não executa instrução e devolve `CycleCap`.
- 6551: ler o registrador de status não limpa o byte do receptor; só a leitura do receptor limpa.
- Montador: `.word ""` é erro, distinto de `.byte ""` que não emite bytes.
- Ligador: relocação relativa na faixa fica patchada; outra fora de alcance devolve o erro sem deslocamento errado no sítio válido.
- Depurador: `step_out` com o PC fora de um callee de JSR devolve erro e não executa instrução.
- 6551: no loopback, dois bytes bons seguidos, o segundo substitui o primeiro no receptor, e o status indica dado pronto só até a leitura do receptor.
- Montador: `.byte ""` não emite bytes e não é erro.
- Ligador: script que nomeia uma seção inexistente é erro com o nome da seção, e nenhum byte de imagem é produzido.
- Depurador: `run_until_halt` num JAM para com o PC sobre o JAM, sem avançar.
- 6551: um quadro bom depois de framing limpa o bit de framing e entrega o byte.
- Montador: `.org` um endereço atrás, sobre o último byte emitido, é `org para trás`.
- Leitor de objeto: byte de versão de formato desconhecido é rejeitado e nomeia a versão.
- Depurador: `step_over` numa instrução que não é JSR executa só essa instrução.
- 6551: no loopback, stop bit baixo liga o bit de framing e o byte não entra limpo no receptor. Paridade errada continua sem entregar o byte.
- Montador: `.org` para o endereço em que o contador já está não move nada e não é `org para trás`.
- Leitor de objeto: comprimento declarado da seção diferente dos bytes é erro, antes de devolver as seções.
- Depurador: `run_until_halt` corre até BRK ou JAM, devolve o motivo e não executa a instrução seguinte. Um laço sem BRK/JAM para no teto de ciclos informado, com motivo `CycleCap`.
- 6551: no loopback, paridade errada liga o bit de erro e o byte não entra no registrador de recepção. Um quadro com a paridade programada entra.
- Montador: `.org` para trás, sobre bytes já emitidos, é o erro `org para trás`. Um intervalo à frente é preenchido com zero.
- Leitor de objeto: cabeçalho presente com corpo curto é `objeto truncado`, antes de qualquer seção. Checksum inválido continua rejeitado.
- Depurador: `step_out`, dentro de um callee de JSR, corre até o RTS que volta ao chamador desse chamador. Uma chamada aninhada não para no RTS interno.
- 6522: ACR `011` monta um byte no registrador de deslocamento a cada oito bordas de subida de CB1. PHI2 sozinho não desloca. Distinto do shift-in `010` e do shift-out `111`.
- Montador: `.word` de uma string de dois caracteres é erro. `.word label` emite o byte baixo e depois o alto.
- Ligador: com duas relocações absolutas de byte, uma na faixa e outra `fora de faixa`, o byte válido permanece patchado e o erro nomeia o símbolo.
- Depurador: `step_over` a partir de um JSR corre até o RTS correspondente voltar à instrução seguinte. Um JSR aninhado volta ao chamador externo, não ao interno.
- 6522: o modo de deslocamento do ACR `111` avança um bit a cada borda de CB1. Oito bordas deslocam um byte. Ticks de PHI2 sozinhos não deslocam esse modo.
- Montador: `.byte` e `.db` aceitam string entre aspas. `.byte "AB"` emite 65, 66. Aspas sem par são erro.
- Ligador: relocação absoluta de um byte cujo símbolo definido vale mais que 255 é erro `fora de faixa`.
- 6522: handshake de entrada e de saída em CA2/CB2 (PCR bits 3–1 e 7–5). O handshake de saída não é o pulso `101`, e o de entrada limpa o flag ao acessar a porta. O ACR não escolhe esses pinos.
- Montador: macros aninham até quatro níveis. O quinto nível, inclusive uma macro que chama a si mesma, é erro.
- Ligador: um script `section <nome> <endereço>` fixa o início de cada seção. Duas seções caem nos endereços nomeados.
- Mapa de origem sobrevive ao ligador: endereço para arquivo e linha, depois de ligar dois módulos.
- Snapshot versão 5 guarda o listing da próxima instrução. A versão 99 é rejeitada. As versões 2, 3 e 4 ainda carregam.
- Programa original `examples/checksum.asm`: o montador aceita, o script posiciona e a CPU soma três bytes.
- Timer 2 em modo pulso (ACR bit 5) não decrementa no `tick` de PHI2. O one-shot (bit 5 limpo) continua no tick.
- 6551: paridade (comando, bits 7–5) e stop bits (controle, bit 7) são conferidos no loopback, junto com o nibble de baud.
- Labels locais `@` dentro de macros aninhadas não colidem entre expansões.
- Relocação relativa que não alcança o alvo é erro.
- `Cpu::last_insn` guarda endereço, opcode e ciclos da última instrução. Os 151 opcodes oficiais batem com `step_cycle`.
- `diff_image` lista os bytes de latch dos timers da VIA que mudaram.
- Linker relocável (`link` em `src/link.rs`) que combina objetos e resolve símbolos globais.
- Arquivos objeto: `write_object` e `read_object` guardam seções, símbolos e relocações.
- Desmontador dos opcodes oficiais (`disasm` em `src/disasm.rs`).
- Depurador: `break_when_flag_becomes_set` para quando a flag escolhida passa de limpa para setada.
- Mapa de memória nomeado (`NamedMemory` em `src/bus.rs`): regiões com nome, sem sobreposição.
- Exemplo `examples/add_carry.asm`: soma dois bytes e guarda o carry.

- 2A03 com APU original (pulsos, triângulo, ruído, frame counter, envelope, comprimento).
- Mappers NROM, UxROM (conflito de barramento) e MMC1 (porta serial).
- Depurador REPL, tabela de símbolos, desmontagem com nomes, profiler de ciclos e mapa de cobertura.
- Autoteste em assembly próprio (`examples/selftest.asm`).
- Correção: referência adiante com deslocamento (`sta t+1`) tinha tamanhos diferentes nas duas passagens.
- 65C02: opcodes indefinidos são NOPs com bytes e ciclos documentados (instrução e ciclo).
- APU: canal DMC (timer, busca com parada da CPU, IRQ, loop) e mixer não linear.
- MMC1 ignora escrita serial no ciclo seguinte a outra.
- Expressões: `/`, `-` unário e `~`; BCD verificado; `Status` com `FromStr`; WAV e PCM em `audio`.
- 2A03: programa sem vetor de reset no PRG é rejeitado.
- Corrigido `examples/sieve_primes.asm` (passo guardado dentro da tabela, laço sem volta).
- `scripts/`: CI local, exemplos com resultados esperados, versões, pacote e relatório (bash e PowerShell).
- 65C02: RMB/SMB/BBR/BBS com ciclos corretos (instrução e ciclo), montador e desmontador (`Mode::ZpRel`).
- DMC: janela de DMA de 4 ciclos que só para a CPU em leitura (`Cpu::next_cycle_writes`); comprimento inicial 1 e início $C000.
- Reintegrados `hexfmt`, `pia`, `tape` e `profile` (existiam só no Drive); `profile` agora usa o `Profiler`.
- `SystemBus::memory_map`, `FlatMemory::try_load_at`; testes de propriedade da ALU sobre todas as entradas.
- `scripts/mirror.ps1`; scripts PowerShell em UTF-8 com BOM.

## [1000000.0.0] - 2026-10-03

### Added
- `Via6522`: modos de saída de CA2 e CB2 no PCR. `111` segura o pino alto, `110` segura baixo. `101` é pulso: uma escrita da CPU no registrador de saída (`$01` para CA2, `$00` para CB2) baixa o pino e incrementa o contador de pulsos. O `tick` seguinte solta o pino. `$0F` não pulsa CA2. `100` é handshake (a escrita baixa o pino até a borda de descida de CA1/CB1).
- Montador: uma macro pode chamar outra macro um nível abaixo. Dois níveis de chamada continuam erro (`macro aninhada rejeitada`). `.macro` dentro de `.macro` continua erro. O teste antigo de aninhamento foi atualizado.
- Console 32×32: name table de 256 bytes (16 colunas por 16 linhas). O host pinta um tile 8×8 do canto superior esquerdo 4×4 com `render_name_tile`. A cor 0 do padrão continua transparente.
- Registrador de paleta de 16 cores. O blit (e o tile da name table) escreve `palette[índice]` para um nibble diferente de zero. O padrão é a identidade, então os testes antigos de blit não mudam. O índice 1 passa a ser a cor programada.
- Depurador: `watch_pc_range(início, fim)` inclusive. `Debugger::run` para com `StopReason::PcRange` quando o PC está dentro, antes da busca. Fora do intervalo não para.
- 6551: escrever o registrador de controle copia o nibble de baud (bits 0–3) para o período já existente (`BAUD_PERIOD_BY_NIBBLE`). Nibble 0 é relógio externo, período 0. Não é divisor de cristal. `set_baud_period` ainda sobrescreve até a próxima escrita de controle. `Cpu::step` continua sem dar `tick`.
- Monitor original de 1 KiB em `examples/monitor_1k.asm`, montado no crate, em `$FC00..=$FFFF`. Lê um byte da 6551 em `$D000` e escreve o mesmo byte de volta. Não é WozMon. O teste roda com `run_cycles_with_acia`.
- Snapshot versão 4 (`MachineImage::at_console`): inclui os 16 bytes da paleta e os 256 bytes da name table, além do que a versão 3 já tinha. Ida e volta em JSON. Versão 0 é rejeitada. Versões 2 e 3 ainda carregam (paleta identidade, name table zerada).
- Tabela de registradores: cada opcode NMOS documentado (151) roda em `step` e em `step_cycle` com o mesmo operando, e A, X, Y, SP, PC e P têm de coincidir.

### Still not claimed
- Não há meio-ciclos de PHI2. IRQ/NMI só na fronteira de instrução.
- `WAI` e `STP` não existem.
- Os opcodes instáveis (`ANE`, `SHA`, `SHX`, `SHY`, `SHS`, `LAS`) continuam fora do traço de `step_cycle`.
- Não há PPU. A name table e a paleta são registradores de host num framebuffer 32×32.
- Não há MIPS medido, nem ROM Microsoft, KIM-1 TIM ou NES. O nibble de baud não é um gerador de cristal.


## [50.0.0] - 2026-10-03

### Added
- `Via6522`: borda de descida em CA1/CB1 liga o bit do IFR (bit 1 e bit 4) só se o bit correspondente do IER já estiver habilitado. Com o IER desligado a borda não liga o IFR. Não há handshake de CA2/CB2, e ler a porta não limpa o flag.
- Timer 2 em modo pulso (ACR bit 5): cada borda de descida em PB6 decrementa o contador; o estouro liga o bit 5 do IFR. O `tick` de PHI2 continua decrementando o timer 2 mesmo com o bit 5 setado (o teste da 5.0 arma o ACR em `$20` e espera o one-shot no tick).
- PB7 inverte no estouro do timer 1 só quando o ACR tem free-run e PB7 ao mesmo tempo (`$C0`). O bit não entra na leitura do pino da porta B.
- Shift-in sob PHI2, ACR bits 4–2 = `010`, MSB primeiro: oito `tick`, o bit de `set_sr_in_bit` entra como LSB, e o oitavo ciclo liga o bit 2 do IFR. O shift-out `110` continua. Os outros modos de shift não andam.
- Montador: `.error "mensagem"` falha a montagem com esse texto. `.warn "mensagem"` grava em `AsmOutput::warnings` e a montagem segue. `.scope`/`.endscope` isolam um label `@`. Um label de dentro não vaza.
- Depurador: `break_when_written(endereço, valor)` em `step_watched`. Para com `StopReason::WatchValue` só quando o byte escrito é esse. Outro valor não para.
- Snapshot versão 3 (`MachineImage::at_scanline`): acrescenta `scanline` e mantém `halted`. Ida e volta em JSON. Versão 99 é rejeitada. `capture` continua gravando a versão 2, que ainda carrega.
- Console 32×32: sprite de 64 bytes (8×8). A CPU escreve índice e XY em `$00F8`–`$00FA` (sprite 0) ou `$00F5`–`$00F7` (sprite 1). O host copia o tile da tabela de padrões. Cor 0 é transparente. `$00F4` é a colisão, ligada só quando pixels opacos dos dois sprites caem no mesmo ponto. Não é uma PPU.

### Still not claimed
- Não há meio-ciclos de PHI2. IRQ/NMI só na fronteira de instrução. Os opcodes instáveis continuam fora do traço. `JAM` não é um lock que relê o opcode para sempre. `WAI`/`STP` não existem.
- Não há MIPS medido, nem ROM Microsoft, KIM-1 TIM ou NES. O campo de baud da 6551 não é um gerador.


## [5.0.0] - 2026-10-03

### Added
- `via::Via6522`: portas A/B com DDR, timer 1 one-shot e free-run, timer 2 one-shot, IFR/IER (o estouro liga um bit que a CPU lê) e o modo de shift-out sob PHI2 (ACR `110`, oito ciclos, MSB primeiro, bit 2 do IFR). `ViaTimer1` one-shot permanece. Sem handshake, sem contagem de pulsos no timer 2, sem PB7.
- `systems::via_board::ViaComputer`: placa original (RAM, VIA em `$C000`, ROM montada no crate). `run_cycles` faz `tick` da VIA e `step_cycle`, e a IRQ do timer inverte o bit 0 da porta A. Sem ROM de terceiros.
- Montador: `.rept`/`.endr`, `.if`/`.else`/`.endif` sobre constante, `.incbin` no mapa em memória (`assemble_with_blobs` ou os bytes do mapa de texto) e `.align`.
- Depurador: o anel de step-back guarda um checksum FNV-1a da RAM (não uma cópia de 64 KiB). `break_when_a` para quando A já tem o valor (`StopReason::Accumulator`). A memória continua sem rollback.
- Console 32×32: registrador de scanline em `$00FD` (`0..32`), `tick_scanline` no host. Um programa pode esperar a linha 0.
- `step_cycle` em `JAM`/`KIL`: uma leitura, CPU parada, PC não avança. Os instáveis `ANE`/`SHA`/`SHX`/`SHY`/`SHS`/`LAS` continuam de fora (`NMOS_NOT_CYCLE_STEPPED`).
- Snapshot versão 2, JSON à mão: registradores, RAM, estado da 6522 e da 6551. Sem o campo `version`, a imagem é rejeitada.
- `docs/APENDICE_OPCODES.md`: quais encodings o traço cobre e quais instáveis ele recusa.

### Still not claimed
- Não é cycle-accurate. Não há meio-ciclos de PHI2. IRQ e NMI só na fronteira de instrução. Os opcodes instáveis não são traçados. `JAM` não é um lock de barramento que relê o opcode para sempre. `WAI`/`STP` não existem (`$CB` é `AXS`, `$DB` é `DCP` abs,Y).
- O campo de baud da 6551 não é um gerador. Não há MIPS medido, nem ROM Microsoft, KIM-1 TIM ou NES.

## [1.1.0] - 2026-10-03

### Added
- `Cpu::step_cycle` now traces the stable NMOS unofficial opcodes the core already executes: SLO, RLA, SRE, RRA, DCP, ISC (including `(zp,X)`, `(zp),Y`, and `abs,Y`), LAX, SAX, ANC, ALR, ARR, AXS, the unofficial immediate `SBC` (`$EB`), and the NOP forms (immediate, zero page, zero page,X, absolute, absolute,X, implied). End state matches `Cpu::step`. `Cpu::step` itself is unchanged.
- CMOS opcodes this core runs are cycle-stepped when `CpuVariant::CMOS` is selected: `BRA`, `STZ`, `TRB`/`TSB`, `BIT` immediate / zp,X / abs,X, accumulator `INC`/`DEC`, `PHX`/`PHY`/`PLX`/`PLY`, `(zp)` loads and `STA (zp)`, and `JMP (abs,X)`. `TRB`/`TSB`, indexed `BIT`, `BIT #`, and `JMP (abs,X)` are new in `Cpu::step` as well as in the trace, so the two paths stay comparable.
- `Cpu::reset` performs seven bus reads and no writes (two at the current PC, three stack reads that do not change SP, then `$FFFC`/`$FFFD`). `cycles` is set to 7. It is not a `step_cycle` sequence.
- `acia::run_cycles_with_acia` runs `step_cycle` and ticks the 6551 once per cycle. `Cpu::step` still does not tick the chip.
- Assembler expressions on `.byte`/`.word` (and other operands): `+`, `-`, `*`, parentheses, and `hi()`/`lo()`, beside the existing `<`/`>` prefixes. `AsmOutput::listing` prints address, bytes, and source line.
- `systems::term::TermComputer`: original machine, 32 KiB RAM, 6551 at `$D000`, ROM at `$E000` assembled from `examples/term_echo.asm`. Echoes a line ending in CR. No historical ROM.
- Debugger read and write watchpoints (`Watch`, `LoggingBus`, `step_watched`) and a ring of the last 32 register snapshots (`step_back`). Step-back restores registers only.
- `BusCycle::trace_line`, `format_bus_trace`, and `Cpu::trace_bus_instruction` log address, data, and R/W for one instruction.
- `via::ViaTimer1`: 6522 Timer 1 in one-shot mode. Writing the high counter byte arms the latch; the Nth `tick` sets the flag and stops. Not wired into `Cpu::step`.

### Still not claimed
- Not cycle-accurate and not a visual6502 netlist. No PHI2 half-cycles. Interrupts are sampled only on an instruction boundary.
- Excluded from `step_cycle` on NMOS: `JAM`/`KIL` (`$02 $12 $22 $32 $42 $52 $62 $72 $92 $B2 $D2 $F2`) and the unstable group `ANE`/`SHA`/`SHX`/`SHY`/`SHS`/`LAS` (`$8B $93 $9B $9C $9E $9F $BB`). On CMOS the `(zp)` bytes and `$9C`/`$9E` are the real CMOS opcodes and are traced.
- `WAI` and `STP` are not implemented. `$CB` stays NMOS `AXS`/`SBX` and `$DB` stays `DCP` abs,Y. They are not the WDC wait/stop instructions.
- The 6551 control-register baud field is still not decoded. There is no crystal divider. The countdown is whatever period the host sets.
- No Microsoft BASIC, no KIM-1 TIM ROM, no NES ROM, no measured MIPS figure.

## [1.0.0] - 2026-10-03

### Added
- `Cpu::step_cycle` now covers all 151 documented NMOS opcodes: the 0.5.0 subset plus `abs,Y`, zero-page indexed, `(zp,X)` / `(zp),Y`, the RMW group (`ASL`/`LSR`/`ROL`/`ROR`/`INC`/`DEC`, including the dummy write of the original byte), implied and accumulator ops, `PHA`/`PHP`/`PLA`/`PLP`, `JMP` absolute and indirect, and `RTI`. IRQ and NMI entry are unchanged. `Cpu::step` is unchanged.
- NMOS `JMP ($xxFF)` keeps the page-wrap bug (5 cycles). CMOS `JMP ($nnnn)` is 6 cycles and does not wrap, matching `Cpu::step`. The extra CMOS cycle re-reads the pointer high byte; that is not a WDC microcode trace.
- `Acia6551::set_baud_period` and `Acia6551::tick`: a host-driven countdown until TDRE returns and until a queued RX byte sets RDRF. Period 0 (the default) is the 0.5.0 immediate holding register, so existing ACIA tests are unchanged. `Cpu::step` does not tick the chip.

### Still not claimed
- The 1.0.0 text below described the limits of that release. 1.1.0 lifts the unofficial, CMOS, and reset limits listed here, with the exclusions in the 1.1.0 section. No half-cycles, no visual6502 netlist. No Microsoft BASIC, no KIM-1 TIM ROM, no measured MIPS figure.

## [0.5.0] - 2026-10-03

### Added
- `Cpu::step_cycle` / `BusCycle`: one bus cycle at a time for a documented subset (immediate, zero-page, and absolute loads and stores, `abs,X` including the page-cross penalty, branches taken and not taken, JSR, RTS, BRK, IRQ and NMI entry). Other opcodes remain instruction-granular. Not a visual6502 model. `Cpu::step` is unchanged.
- Assembler `.macro` / `.endmacro` (nesting is an error), numeric locals `1:` with `1b`/`1f`, `@name` locals, and `.include` from an in-memory map (`assemble_with_files`).
- MOS 6551 model (`acia`): TDRE, RDRF, overrun, loopback, and `RamAcia` so a program can `STA` a byte the host reads back. No baud generator.
- `snapshot::MachineImage`: JSON round-trip of registers and a flat 64 KiB RAM image, no extra crates.
- `debug_api::DebugServer` and CLI `serve` on `127.0.0.1` (`std::net` only): registers, step N instructions, get/put a memory range.
- `audio::square_wave`, used by `FantasyConsoleDevice::tone`. Period is what the tests check.

### Fixed
- The crate again compiles: RMW instructions no longer double-borrow the bus, and the assembler's program counter stays 32-bit across both passes.
- `documented_opcodes()` is the 151 official NMOS opcodes. The assembler no longer emits illegal opcodes that share a mnemonic (for example `NOP $10`).
- README badges: removed the crates.io badge (crates.io `mos6502` is a different project) and the GitHub Actions badge (this tree's workflow file is not a live `geraliza/01-02-mos6502` status).

### Not claimed
- Full bus-cycle accuracy, Microsoft BASIC, a KIM-1 TIM ROM, or a measured MIPS number in the docs.

## [0.4.0]
- Apple-1 / WozMon, partial Ben Eater VIA/LCD model, TUI, disassembler, `SystemBus`.

## [0.3.0]
- Unofficial NMOS opcodes, `CpuVariant`, decimal-mode ADC/SBC flag fixes.

## [0.2.0]
- Documented 6502 opcodes, `MemoryBus`, `FlatMemory`, instruction cycle counts.

## [0.1.0]
- Initial registers, flags, and a zero-dependency crate.
