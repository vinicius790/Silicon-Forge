# Ferramentas de linha de comando e formatos

Todos os subcomandos abaixo usam os mesmos códigos de saída:

| Código | Significado |
|---|---|
| 0 | sucesso |
| 1 | erro na entrada: arquivo ilegível, erro de montagem ou ligação, formato corrompido, falha da CPU, limite de instruções atingido sem `BRK` |
| 2 | uso inválido: opção desconhecida, valor ausente ou malformado, argumento a mais. O resumo de uso vai para stderr |

As mensagens de erro vão para stderr no formato `mos6502 <comando>: <mensagem>`.

## Subcomandos

```
mos6502 assemble prog.asm -o prog.bin [--format bin|ihex|srec|mos] [--sym prog.sym]
mos6502 link a.asm b.o65 -o out.bin [--base $8000] [--map out.map]
mos6502 run   PROG [--load $8000] [--start ADDR] [--variant nmos|cmos|nes] [-n MAX]
mos6502 trace PROG [...]                 # uma linha por instrução, antes de executá-la
mos6502 profile PROG [...] [--sym S] [--folded F] [--dot D]
mos6502 coverage prog.asm [-n MAX] [--lcov prog.info]
mos6502 hex ENTRADA -o SAIDA [--to bin|ihex|srec|mos] [--load ADDR]
mos6502 pack ENTRADA -o SAIDA   |   mos6502 unpack ENTRADA -o SAIDA
```

`PROG` é binário bruto (carregado em `--load`) ou texto Intel HEX, S-record ou MOS
hex, reconhecido pelo primeiro caractere (`:`, `S`, `;`). A execução começa em
`--start`, no endereço de início do arquivo ou na origem, e para *antes* de um `BRK`
(ou em JAM/`STP`). Sem `BRK` dentro de `-n` instruções (padrão 1 000 000) a saída é 1.

`link` aceita fontes (`.asm`, `.s`, montadas como objeto) e objetos binários, liga em
`--base` e grava a imagem plana do menor ao maior byte usado, com lacunas em `$FF`.

## Formatos hexadecimais (`hexfmt`)

* **MOS Technology** (fita perfurada do KIM-1): `;LLAAAA<dados>CCCC`, até 24 bytes por
  registro, `CCCC` = soma de 16 bits de contagem, endereço e dados. O último registro
  é `;00NNNN CCCC`, com `NNNN` o número de registros de dados; o leitor confere esse
  número e exige o registro final.
* **Intel HEX 32 bits**: `to_intel_hex32`/`from_intel_hex32` com registros `04`
  (linear), `02` (segmento), `05`/`03` (início). Um registro nunca cruza 64 KiB. O
  leitor de 16 bits continua recusando base diferente de zero.
* `detect_format`/`read_any` escolhem o leitor pelo primeiro caractere.

## Fita (`tape`)

* **Kansas City Standard**: `kcs_decode_pcm` mede semiciclos entre trocas de sinal,
  separa 1200/2400 Hz no ponto médio de 1800 Hz e decodifica os quadros. Funciona com
  polaridade invertida e qualquer taxa a partir de 4800 Hz.
* **Apple II**: `apple_encode_pcm` gera tom de cabeçalho de 770 Hz, sincronismo
  (200 µs + 250 µs), bits MSB primeiro (`0` = ciclo de 2000 Hz, `1` = 1000 Hz) e o
  checksum XOR semeado com `$FF`. `apple_decode_pcm` segue a rotina `READ` da ROM: ao
  menos 16 semiciclos de cabeçalho, sincronismo, bits pela duração do ciclo inteiro
  (limiar 750 µs) e verificação do checksum.

## Perfil (`profiler`)

Além de ciclos por `PC` e por rotina (inclusivo/exclusivo), o perfilador guarda o
grafo de chamadas (`call_edges`, `edge(caller, callee)`: chamadas e ciclos inclusivos
por aresta, `None` = nível superior), exporta pilhas no formato *folded* de Brendan
Gregg (`folded`, pronto para `flamegraph.pl`; a soma das linhas é o total de ciclos)
e o grafo em Graphviz (`call_graph_dot`).

## Cobertura (`coverage`)

`hits(addr)` conta execuções de cada opcode. `source_lines` liga a listagem do
montador às linhas do fonte (comentários e linhas vazias ignorados; diretivas,
dados e equates não contam como código). `to_lcov` gera um tracefile LCOV
(`TN`, `SF`, `DA`, `LF`, `LH`) para `genhtml`, Codecov ou Coveralls.

## Contêiner comprimido (`snapshot::pack`)

`M65Z` + versão + método (0 armazenado, 1 LZ77) + tamanho original (u32 LE) +
CRC-32 IEEE (u32 LE) + carga. Serve para qualquer bloco: snapshot JSON, imagem de
RAM, estado de ciclo. `unpack` detecta assinatura, versão, método, fluxo corrompido,
tamanho e CRC errados. Uma RAM zerada de 64 KiB cabe em menos de 1,7 KB.

## Sistemas pela linha de comando

```
mos6502 nes jogo.nes --frames 120 --region pal --png quadro.png --ppm quadro.ppm \
    --wav som.wav --rate 44100 --bits 8 --channels 2 --sav jogo.sav
mos6502 kim1 --keys "AD 0200 DA A9" -n 5000          # mostra o display de 6 dígitos
mos6502 kim1 --load prog.bin --at $0200 --go $0200 -n 1000
mos6502 board examples/via_clock.asm --cycles 100500 --dump 0:2
mos6502 board examples/acia_irq_echo.asm --input "Hi!" --dump $0200:3
mos6502 bench-suite --seconds 0.5 --write base.txt
mos6502 bench-suite --baseline base.txt --tolerance 15
```

* `nes`: roda N quadros (NTSC, PAL ou Dendy) e grava o último quadro em PPM e/ou
  PNG, o áudio mixado em WAV (amostrado no clock da CPU da região) e a RAM de
  bateria em `.sav`, que é lida antes da execução quando existe. Cartucho sem
  bateria não grava `.sav`; `.sav` de tamanho errado sai com código 1.
* `kim1`: teclas `AD`, `DA`, `+`, `GO`, `PC`, `RS`, `ST` e dígitos hexadecimais;
  `--load` só aceita RAM gravável (conferida byte a byte).
* `board`: RAM com 6522 em `$C000` e 6551 em `$D000`, vetores de reset na origem e
  IRQ em origem+3 (layout dos exemplos), um byte de `--input` a cada 200 ciclos;
  imprime ORB, pinos de PB, o que a ACIA transmitiu e o `--dump`.
* `bench-suite`: MIPS de cada carga e a impressão digital de ciclos de 10 000
  instruções. Contra uma linha de base, queda acima de `--tolerance` % ou ciclos
  diferentes (mudança de temporização) saem com código 1.

## Formatos com endereço de carga

* **C64 `.prg`**: endereço LE + bytes (`to_prg`, `from_prg`, `c64_sys_stub` gera
  `10 SYS nnnn`).
* **Atari `.xex`**: `$FFFF`, segmentos `início fim` inclusivos, `$FFFF` opcional
  entre segmentos; `RUNAD` (`$02E0`) e `INITAD` (`$02E2`) são reconhecidos.
* `run`, `hex`, `board` e `kim1` leem `.prg`/`.xex` pela extensão (ou `$FFFF`
  inicial); `--from` força o formato. `assemble --format prg|xex` e
  `hex --to prg|xex` escrevem.

## PNG, zlib e WAV

`graphics::png_rgb` escreve PNG RGB 8 bits escolhendo por linha o filtro (None,
Sub, Up, Average, Paeth) de menor soma absoluta, com DEFLATE próprio (blocos
armazenados no nível 0, Huffman fixo com LZ77 de janela de 32 KiB nos demais).
`inflate` lê blocos armazenados, fixos e dinâmicos (testado contra fluxos do
zlib), e `decode_png_rgb` confere CRC de cada bloco e desfaz os cinco filtros.
`audio::wav_bytes_with` grava mono/estéreo em 8 bits (sem sinal, centro 128) ou
16 bits, com byte de alinhamento RIFF; `parse_wav_any` percorre os blocos e pula
`LIST` e afins.

## Cobertura com macros e `.include`

`coverage::line_map` usa `origin_line` da listagem: cada linha de chamada de
macro recebe todos os endereços da expansão (conta como executada pelo endereço
mais executado) e linhas de arquivos incluídos vão para o próprio arquivo.
`to_lcov_files` grava um registro LCOV por arquivo. O comando `coverage` lê os
`.include` do disco relativos ao arquivo que os inclui.
