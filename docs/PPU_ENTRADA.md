# PPU ponto a ponto e portas de controle

## Pipeline do fundo (2C02)

Com a renderização ligada, nas linhas visíveis e na pré-renderização, cada grupo de
8 pontos em 1–256 e 321–336 faz quatro leituras de 2 pontos, com o dado chegando no
segundo ponto:

| ponto do grupo | leitura |
|---|---|
| 2 | byte da nametable em `$2000 \| (v & $0FFF)` |
| 4 | byte de atributo em `$23C0 \| (v & $0C00) \| ((v >> 4) & $38) \| ((v >> 2) & 7)` |
| 6 | plano baixo do padrão |
| 8 | plano alto do padrão, seguido do incremento de X grosso |

- X grosso de `v` sobe nos pontos 8, 16, …, 256, 328 e 336. O Y sobe no ponto 256.
- Os bits horizontais de `t` vão para `v` no ponto 257. Os verticais vão nos pontos
  280–304 da linha de pré-renderização.
- Dois registradores de deslocamento de 16 bits (planos do padrão) e dois de atributo
  deslocam nos pontos 2–257 e 322–337. O byte baixo recarrega nos pontos 9, 17, …,
  257, 329 e 337.
- O pixel `x` sai no ponto `x + 1`, do bit `15 - fine_x`.

Consequência: uma escrita em `$2005`/`$2006`/`$2000`, ou uma mudança de CHR ou de
nametable no meio da linha, vale a partir do primeiro tile lido depois dela, que
aparece 16 pontos depois. O fine X vale no ponto seguinte. Com a renderização
desligada, os registradores param.

## Sprites

- A OAM secundária é limpa para `$FF` (pontos 1–64).
- A avaliação começa no ponto 65 com a OAM como está naquele ponto: 2 pontos por
  checagem de Y, mais 6 por sprite copiado, até 8.
- Depois do oitavo, a varredura com o bug do hardware compara bytes de tile,
  atributo e X como se fossem Y. O flag de overflow sobe no segundo ponto da
  checagem que casou.
- Pontos 257–320 leem o padrão de cada slot (baixo em 262 + 8i, alto em 264 + 8i).
  Os pixels da linha seguinte ficam travados no ponto 320.
- Não há avaliação na pré-renderização, então a linha 0 não tem sprites.
- O sprite 0 hit sai do pixel dos registradores de deslocamento, no ponto do
  primeiro pixel opaco sobreposto (nunca em x = 255 nem nos 8 pixels cortados).

As bordas de subida de A12 para o MMC3 continuam sinalizadas uma vez por linha, no
ponto documentado (260 ou 324), e não leitura por leitura.

O snapshot da máquina NES (versão 2 ou mais nova) guarda os registradores de
deslocamento, as travas, a OAM secundária, os padrões dos slots e o ponto de
overflow pendente.

## `$4016` e `$4017` (`src/input.rs`)

- Escrever em `$4016` define o strobe (bit 0) das duas portas. Com o strobe alto, o
  registrador recarrega sem parar e toda leitura devolve o botão A.
- Com o strobe baixo, cada leitura tira um bit: A, B, Select, Start, Cima, Baixo,
  Esquerda, Direita. Depois dos oito, a leitura devolve 1.
- Os bits altos vêm do barramento aberto, `$40`.
- O segundo controle fica em `$4017`, independente do primeiro. O estado dos dois
  controles e o strobe entram no snapshot.
- **Zapper** na porta 2 (`NesBus::zapper`): D3 = 0 quando a luz é vista, D4 = 1
  quando o gatilho está puxado, D0 = 0. A luz é vista se o pixel mirado foi
  desenhado claro neste quadro (linhas `$2x`/`$3x`, matiz `$0`–`$C`) e o feixe passou
  por ele há no máximo 26 linhas. Ele não tem estado travado e por isso não entra no
  snapshot.

## Barramento de padrões e A12

- Toda busca real vai para `Cartridge::ppu_address`, na ordem do 2C02: no ponto 0 o
  endereço de CHR da busca baixa do ponto 5; em cada grupo de 8 pontos, nametable,
  atributo, plano baixo e plano alto; nos pontos 257–320 as oito vagas de sprite
  (nametable de lixo, atributo de lixo, dois planos), inclusive as vazias, que buscam
  o tile `$FF`; e as duas leituras de nametable em 338 e 340.
- O MMC3 conta só subidas reais de A12 depois de A12 ficar baixo por 3 ciclos de CPU.
  Fundo em `$0000` e sprites em `$1000` dão uma subida por linha (241 por quadro);
  fundo em `$1000` e sprites em `$0000` dão 242, pela subida que sai do vblank; as
  duas tabelas iguais dão 0 ou 1. Sprites 8x16 com tile par puxam a busca para
  `$0000` e escondem a subida daquela linha.
- O MMC3 com submapper 4 do NES 2.0 usa o IRQ antigo (MMC3A/NEC): recarregar um
  latch 0 sem a flag de `$C001` não dispara.

## Avaliação de sprites ponto a ponto

- Pontos 1–64: secondary OAM recebe `$FF` nos pontos pares, e `$2004` lê `$FF`.
- Pontos 65–256: pontos ímpares leem a OAM em `OAMADDR(65) + 4n + m`, pontos pares
  agem. Com menos de oito sprites achados, o Y é copiado para a próxima vaga e, se
  estiver no alcance, os outros três bytes também. Com oito, a secondary vira só
  leitura e cada teste fora de alcance avança `n` e `m` juntos: o bug do overflow,
  que acerta ou erra pelo byte de tile e atributo.
- A OAM é lida ao vivo: uma escrita antes do ponto de leitura daquele sprite é vista.
- `$2004` lido durante a renderização devolve o byte do barramento interno da OAM.
  Escrito durante a renderização, não grava e soma 4 ao OAMADDR. O OAMADDR vai a 0
  nos pontos 257–320.
- Desligar a renderização no meio da linha marca uma linha de 8 bytes da OAM: nos
  pontos 0–63 a linha `ponto / 2`, nos pontos 256–319 a vaga e o byte de secondary
  sendo lidos. Quando a renderização volta, a linha 0 da OAM é copiada sobre ela.
  Desligar durante 64–255 ou depois de 320 não corrompe nada.

## DMC contra controle

- Quando o DMA do DMC para a CPU num ciclo de leitura, o 2A03 repete aquela leitura.
  Se ela for `$4016`/`$4017` com o strobe baixo, o controle avança um bit a mais.
  Leituras seguidas mantêm /OE baixo e contam uma vez só, então o jogo perde
  exatamente um bit (`NesBus::dmc_conflicts` conta). O estado entra no snapshot v4.

## Mappers novos e submappers

| Mapper | Placa | Registradores |
|---|---|---|
| 9 | MMC2 (PxROM) | `$A000` PRG 8 KiB; `$B000`–`$E000` CHR 4 KiB por latch; `$F000` espelhamento. Latch 0 só em `$0FD8`/`$0FE8`, latch 1 em `$1FD8`–`$1FDF`/`$1FE8`–`$1FEF`. |
| 10 | MMC4 (FxROM) | Como o MMC2, com PRG de 16 KiB, 8 KiB de PRG RAM e as duas faixas de 8 bytes. |
| 34 | NINA-001 | `$7FFD` PRG 32 KiB, `$7FFE`/`$7FFF` CHR 4 KiB, gravados também na PRG RAM. Submapper 1 NINA, 2 BNROM; sem submapper, CHR maior que 8 KiB é NINA. |
| 71 | Camerica/Codemasters | `$C000`–`$FFFF` PRG 16 KiB; submapper 1 (Fire Hawk) com tela única por `$8000`–`$9FFF` bit 4. |
| 75 | Konami VRC1 | PRG 8 KiB em `$8000`/`$A000`/`$C000`; `$9000` espelhamento e bit alto de CHR; `$E000`/`$F000` CHR 4 KiB. |

- Mappers 2, 3 e 7: submapper 1 sem conflito de barramento, 2 com conflito.
- Submapper não documentado para esses mappers dá `InesError::UnsupportedSubmapper`.

## Snapshot

- A versão 4 guarda a avaliação em andamento (`n`, `m`, byte do barramento, base,
  fase), as linhas de OAM a corromper e a leitura parada pelo DMC. A versão 3 ainda
  carrega, com a avaliação parada e nada pendente.

## Regiões: NTSC, PAL e Dendy

| | NTSC (2C02/2A03) | PAL (2C07/2A07) | Dendy |
|---|---|---|---|
| Linhas por quadro | 262 | 312 | 312 |
| Vblank (flag no ponto 1) | linha 241 | linha 241 | linha 291 |
| Pré-render | 261 | 311 | 311 |
| Pulo de ponto em quadro ímpar | sim | não | não |
| Pontos por ciclo de CPU | 3 | 16/5 (3,2) | 3 |
| CPU | 21,477 MHz / 12 | 26,601 MHz / 16 | 26,601 MHz / 15 |
| Tabelas do APU | NTSC | 2A07 (ruído, DMC, sequenciador 8313/16627/24939/33253, 5 passos 41565) | NTSC |

- O byte 12 do NES 2.0 escolhe a região (`Timing::region`; multi-região roda como
  NTSC). `Nes2A03::set_region` troca à mão. O snapshot v5 guarda a região e a fase
  do divisor PAL e recusa carregar numa máquina de outra região; o v4 é só NTSC.
- No 2C07 os bits de ênfase vermelho e verde do PPUMASK são trocados.

## 2C02G: OAMADDR desalinhado

- Se OAMADDR for 8 ou mais quando a renderização começa (ponto 1 da linha de
  pré-render), os 8 bytes em `OAMADDR & $F8` são copiados sobre os bytes 0–7 da OAM.
  Vale só para NTSC (`Ppu::oamaddr_bug`); nas outras não está documentado.

## Leituras repetidas do PPU durante o DMA do DMC

- O /CS do PPU depende do M2, então cada ciclo em que a CPU fica parada numa leitura
  de `$2002`/`$2007`, menos o próprio fetch do DMA, repete a leitura. `$2002` apaga o
  vblank e o toggle de novo; `$2007` avança o endereço e pula bytes
  (`NesBus::dmc_ppu_repeats`). O controle continua contando uma vez só.

## Mais mappers

| Mapper | Placa | Detalhes |
|---|---|---|
| 21, 22, 23, 25 | Konami VRC2/VRC4 | Tabela de submappers do NESdev (VRC4a/c, VRC2a, VRC4f/e, VRC2b, VRC4b/d, VRC2c; submapper 0 junta as duas fiações). PRG com troca de modo, CHR 1 KiB, IRQ VRC com prescaler 341/3 ou por ciclo. |
| 69 | Sunsoft FME-7 | 16 comandos, `$6000` ROM/RAM, IRQ de 16 bits contando ciclos de CPU para baixo. Sem o áudio 5B. |
| 19 | Namco 163 | PRG 8 KiB, CHR 1 KiB com páginas da RAM de nametable (`$E0`+), nametables de CHR ROM, proteção de escrita, porta de 128 bytes de RAM de som, IRQ de 15 bits. Sem a síntese de áudio. |

## Execução sem tela

- `Nes2A03::run_frames(n)`, `frame_ppm()` (P6 256x240) e
  `render_frames_ppm(n, pasta, nome)` gravam cada quadro em `nome_NNNN.ppm`.

## Fora deste modelo

- Áudio de expansão (Sunsoft 5B, Namco 163): precisa ser mixado na saída do APU.
- Refresh forçado de OAM do 2C07 no vblank longo.
- A paleta RGB do PAL usa o mesmo modelo de sinal NTSC.
- Controles de expansão do Famicom.

## Áudio de expansão, MMC5, VRC6, PAL e `.sav`

* **Gancho de mixagem** (`apu::mix_expansion`, `Cartridge::audio`,
  `Nes2A03::audio_sample`/`render_audio`): o som do cartucho entra depois do
  mixer não linear do APU e soma linearmente. Unidade comum: `APU_PULSE_FULL`
  (um pulso do APU no volume 15).
* **Sunsoft 5B** (dentro do FME-7, mapper 69; `$C000` seleciona, `$E000` escreve):
  três tons `CPU/(32·P)`, ruído LFSR de 17 bits (taps 0 e 3) a cada `32·N` ciclos,
  envelope de 32 passos a cada `16·E` ciclos com as 16 formas CONT/ATT/ALT/HOLD,
  mixer `$07`, DAC logarítmico de 1,5 dB por nível (volume `v` → nível `2v+1`).
  O ganho relativo ao APU não tem número único documentado: `gain` = 1,0 por padrão.
* **Namco 163** (mapper 19): wavetable sobre a RAM interna de 128 bytes, um canal a
  cada 15 ciclos (canais 7 até `8−N`), fase de 24 bits, comprimento
  `256 − (reg & $FC)`, amostras de 4 bits nibble baixo primeiro, saída
  `(s−8)·vol` multiplexada no tempo; `$E000` bit 6 desliga. Nível pelo submapper
  NES 2.0: 3 → 12 dB, 4 → 16,5 dB, 5 → 18,75 dB acima do APU (pontos médios das
  faixas documentadas), 2 → sem som.
* **VRC6** (24 VRC6a, 26 VRC6b com A0/A1 trocados): PRG 16+8+8 fixo, CHR nos modos
  0–3 de `$B003`, espelhamento, RAM de PRG, IRQ VRC; áudio com dois pulsos (duty
  `(d+1)/16`, bit de modo) e dente de serra (6 somas e reset no 14º passo), `$9003`
  halt e deslocamentos ×16/×256. Nível linear, 1 passo = `APU_PULSE_FULL/15`.
* **MMC5** (mapper 5): modos de PRG 0–3 com bit ROM/RAM, 64 KiB de RAM com proteção
  `$5102=2`/`$5103=1`, modos de CHR 8/4/2/1 KiB com os conjuntos A/B (8×16: sprites
  em A, fundo em B; 8×8: último conjunto escrito), `$5130`, ExRAM nos modos 0–3,
  mapa de nametables `$5105` com fill (`$5106`/`$5107`), atributos estendidos,
  detector de scanline por três leituras iguais de nametable (IRQ `$5203`/`$5204`,
  in-frame, saída após 3 ciclos sem leituras), multiplicador `$5205`/`$5206`. Para
  isso a PPU agora informa leituras reais (`Cartridge::ppu_read`, sem o ponto 0
  ocioso), destinos internos do mapper (`PpuTarget::Mapper`, `ppu_write`) e as
  escritas da CPU em `$2000`/`$2001` (`snoop_ppu_register`). No quadro ímpar o ponto
  pulado ainda completa a última leitura de nametable. Áudio do MMC5 e split
  vertical: fora.
* **2C07**: com renderização ligada, o refresh forçado da OAM começa na linha 265
  (24 linhas após o início do vblank) e vai até a pré-renderização; `$2004` e o
  DMA de OAM nessa janela não gravam (`Ppu::oam_refresh`).
* **Paleta PAL** (`ppu::pal_rgb`, usada por `rgb_frame` em PAL e Dendy): mesmo sinal
  de 12 fases, decodificação YUV com a média das linhas alternadas (que cancela a
  inversão de V) e rotação de matiz `PAL_HUE_SHIFT_DEG` (−15°, aproximação).
* **`.sav`**: `Cartridge::save_ram`/`load_save_ram` em todas as placas com RAM de PRG
  (N163 inclui os 128 bytes internos; MMC5 64 KiB); `Nes2A03::save_ram`, `write_sav`
  (escrita atômica via arquivo temporário) e `read_sav` (arquivo ausente = primeiro
  boot, tamanho errado = `InvalidData`) só com o bit de bateria do cabeçalho.
* Snapshot versão 6: estado de áudio do 5B e do N163; versão 5 carrega com o áudio
  zerado. MMC5 e VRC6 são novos e só existem na versão 6.

## Rodada 4: MMC5 áudio/split, VRC6 nametables, VRC7, placas MMC3, FDS, entradas

- **MMC5 áudio** (`Mmc5Pulse`, `$5000`–`$5007`, `$5010`, `$5011`, `$5015`): dois
  pulsos sem sweep (períodos < 8 não silenciam), envelope e comprimento num
  relógio fixo de 240 Hz (`MMC5_FRAME_CYCLES` = 7457), tabela de comprimento com
  `$1C` = `$10`; PCM de 8 bits em modo escrita ou leitura (leituras de
  `$8000`–`$BFFF`), `$00` dispara a IRQ de PCM (`$5010` bit 7, lida e limpa).
  Mistura com polaridade invertida, pulsos no volume do APU, PCM na escala do DMC.
- **MMC5 split vertical** (`$5200`–`$5202`, `Mmc5::split_fetch`): colunas 0..T-1
  (esquerda) ou T.. (direita) usam ExRAM como nametable/atributos e o banco 4 KiB de
  `$5202`, com Y = `$5201` + linha (mod 240) também nos 3 bits finos. Desligado com
  ExRAM em modo 2/3.
- **VRC6**: nametables de CIRAM ou CHR ROM (`$B003` bit 4) pelas tabelas do NESdev
  para todos os modos, com a regra de A10 do bit 5; bancos de 2 KiB de CHR contam
  em páginas de 1 KiB (A10 do PPU com bit 5, LSB do registrador sem ele).
- **VRC7** (mapper 85, submappers 1 = VRC7b/A3, 2 = VRC7a/A4): PRG 3×8 KiB, CHR
  8×1 KiB, espelhamento, WRAM, IRQ VRC. Registradores OPLL (`$9010`/`$9030`) são
  guardados, mas a síntese FM não é gerada (o ROM de instrumentos só é conhecido
  por medições).
- **MMC6** (mapper 4.1): 1 KiB em `$7000`, proteção por metade, `$8000` bit 5.
  **TxSROM** (118): bit 7 do banco de CHR escolhe a página de CIRAM.
  **TQROM** (119): bit 6 do banco escolhe 8 KiB de CHR RAM.
- **Correção**: `PpuMemory::target` mascarava endereços de padrão com `$2FFF`,
  perdendo A12 (`$1xxx` virava `$0xxx`) para mapeadores com `ppu_target`.
- **FDS** (`Nes2A03::from_fds(bios, .fds)`): `parse_fds` (cabeçalho fwNES opcional),
  fluxo de bits com lacunas de 28300/976 bits, `$80` e CRC-16/KERMIT
  (`fds_raw_side`), timer IRQ, drive com leitura/escrita byte a byte a cada 149
  ciclos, erro de CRC, fim de cabeça, IRQ de disco e seus reconhecimentos, 32 KiB de
  PRG RAM, BIOS do usuário, 8 KiB CHR RAM; áudio `FdsAudio` com wavetable,
  envelopes `8*(e+1)*(m+1)`, unidade de modulação e a fórmula de pitch documentada.
  O bit 7 de `$4083` (envelopes 4× mais rápidos) está marcado como TODO no NESdev e
  não é modelado além de parar/zerar a onda.
- **Entradas**: `FourScore` (24 bits por porta com assinatura, depois 1s),
  `ArkanoidPaddle` (NES: 9 bits invertidos MSB primeiro em D4, botão em D3; a
  conversão termina na hora), `PowerPad` (D3: 2,1,5,9,6,10,11,7; D4: 4,3,12,8).
  Snapshot versão 7 guarda esses dispositivos (e recusa um snapshot com outros);
  versões ≤ 6 continuam carregando.
