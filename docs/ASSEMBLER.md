# Assembler e ligador: extensões

Este documento descreve o pré-processador do assembler (`src/asm_pre.rs`), a listagem
com ciclos (`src/asm.rs`) e as opções do ligador (`src/link.rs`). Os exemplos são
cobertos por `tests/asmx_preproc.rs` e `tests/linkx_report.rs`.

> Números sem prefixo são **hexadecimais** neste assembler (`10` vale 16). Use `$`
> para deixar claro. Valores gerados pelo pré-processador (`.strlen`, `.sizeof`,
> campos e membros) são escritos como `$hex`.

## Ordem do pré-processamento

1. `.include`
2. `A::b` vira `A__b` (fora de strings)
3. `.macro` / `.endmacro`
4. `.define`, `.strlen`, `.if`/`.ifdef`/`.ifndef`/`.elseif`/`.else`/`.endif`, `.rept`,
   `.incbin`, `.error`, `.warn`, numa só passada em ordem do texto
5. `.struct`, `.enum`, `.sizeof`
6. rótulos anônimos
7. `.charmap`, rótulos locais `@x` e numéricos `1:`

## `.define`

```asm
.define VAL   $42
.define ADD(a, b)  a+b
    lda #ADD(1, VAL)
.undefine VAL
```

- Nomes diferenciam maiúsculas e minúsculas (como no ca65).
- A troca é por palavra inteira, fora de strings e caracteres, e não ocorre depois
  de `.`, `$`, `%`, `@` ou de um dígito.
- O resultado é expandido de novo, até 16 níveis; além disso é erro
  (`.define R R+1` falha em vez de travar).
- Redefinir sem `.undefine` é erro. Número errado de argumentos é erro.

## Condicionais

- `.if expr` / `.elseif expr` / `.else` / `.endif`: verdadeiro quando diferente de zero.
  As expressões veem as constantes (`nome = expr`) de linhas anteriores ativas.
- `.ifdef nome` / `.ifndef nome`: verdadeiro se o nome foi definido **antes**, numa
  linha ativa, por rótulo, constante ou `.define`. Definições em ramo inativo não contam.
- Ramos inativos não são avaliados (um nome indefinido lá dentro não é erro).
- `.else` ou `.elseif` depois de `.else` é erro.
- Correção: um `.if` aninhado que termina antes de um `.else` não toma mais o
  `.else` do bloco de fora.
- Não há operadores de comparação; use subtração ou máscara (`.if v & 4`).

## `.strlen("texto")`

Número de bytes da string depois de decodificar os escapes. Aceita só uma string
literal (ou um `.define` que vire uma). Funciona em qualquer expressão, inclusive em `.if`.

## `.struct` e `.sizeof`

```asm
.struct Point
x     .word
y     .word
flags .byte
.endstruct

.struct Sprite, 0        ; base opcional dos deslocamentos
pos   .tag Point         ; campos aninhados: Sprite::pos::y
      .res 3             ; preenchimento sem nome
tiles .byte 4            ; contagem opcional
next  .addr
.endstruct

    lda obj+Point::y
    ldx #.sizeof(Sprite)
    lda #.sizeof(Sprite::tiles)
```

Tipos: `.byte` (1), `.word`/`.addr` (2), `.faraddr` (3), `.dword` (4), `.res n`, `.tag T`.
Cada campo vira a constante `Struct__campo`. `.tag T n` multiplica o tamanho, mas não
cria os nomes aninhados. `.sizeof` aceita um struct ou um campo e pode vir antes da
definição. Erros: campo repetido, tipo desconhecido, `.tag` de struct ainda não
definida, struct maior que 64 KiB, bloco sem fechamento.

## `.enum`

```asm
.enum Color
Black          ; 0
Red            ; 1
Green = $10
Blue           ; $11
.endenum
```

Membros viram `Color__Black` etc. (ou nomes simples num `.enum` sem nome). Um valor
pode usar membros anteriores. Passar de `$FFFF` é erro.

## Rótulos anônimos

```asm
:   dex
    bne :-      ; o anônimo mais próximo nesta linha ou antes
    beq :+      ; o próximo depois desta linha
    jmp :--     ; o segundo para trás
```

Funcionam dentro de macros e de `.rept` (cada cópia ganha o seu) e não abrem novo
escopo para rótulos `@locais`. Referência sem destino é erro.

## `.res n[, byte]`

Reserva `n` bytes (zero por padrão). No assembler é o mesmo que `.fill`; no ligador
também está disponível dentro de seções.

## Listagem com ciclos

`AsmOutput::listing_with_cycles()` acrescenta a coluna de ciclos:

```
0202: BD 34 12     4+p   lda $1234,x
0208: D0 F6        2+t   bne start
```

- O valor vem da tabela de opcodes NMOS, ou da 65C02 quando o mnemônico só existe nela.
- `+p`: mais um ciclo se o acesso indexado cruzar página.
- `+t`: desvio, mais um ciclo se tomado e outro se cair em outra página.
- Diretivas e dados ficam sem valor. `AsmOutput::base_cycles()` soma a base de todas
  as linhas; `AsmOutput::line_cycles()` dá o valor de uma linha.

## Ligador

### `.segment`

`.segment "CODE"` (ou `.segment CODE`) entra na seção e, ao contrário de `.section`,
pode reabrir uma seção já existente no mesmo objeto, anexando bytes ao fim dela.

### Limite de tamanho

Script: `limit <seção> <máx>` ou `LinkOptions::limits`. A soma das seções com esse
nome não pode passar de `máx`: `seção code excede o limite: 5 > 4 bytes`.

### Preenchimento até a fronteira do banco

Script: `fill <seção> <fronteira> <byte>` ou `LinkOptions::fills`. A última seção com
esse nome é estendida com `byte` até o próximo múltiplo de `fronteira` (por exemplo
`$4000` para bancos de 16 KiB, `65536` para o espaço inteiro). O preenchimento não
pode sobrepor outra seção, sair de `$FFFF` ou sair da região de memória (quando há
`region`). `Placement::padding` guarda quantos bytes entraram. Os símbolos
`__seção_end`/`__seção_size` não incluem o preenchimento.

### Exportação `.dbg`

`Linked::to_dbg()` escreve o formato de texto do `--dbgfile` do ld65 (versão 2.0),
restrito aos registros `version`, `info`, `seg` e `sym`:

```
version	major=2,minor=0
info	seg=2,sym=3
seg	id=0,name="zp",start=0x000010,size=0x0001,addrsize=zeropage,type=rw
sym	id=0,name="tbl",addrsize=zeropage,val=0x0010,seg=0,type=lab
```

Registros `file`, `line`, `span` e `scope` não são gerados. Um símbolo fora de
qualquer seção (por exemplo um `define` do script) sai sem `seg`.

### Relatório de relocações

`relocation_report(&objetos, &linked)` lista cada relocação: endereço, tipo,
módulo, seção, símbolo, destino final e os bytes gravados.
`format_relocation_report` imprime:

```
RELOCAÇÕES
  $8001 ABS16 a:code tbl -> $0300 [00 03]
  $8006 REL8  a:code loop -> $8005 [FE]
  total: 4
```

Os nomes de módulo devem ser únicos: cada posicionamento é associado ao primeiro
objeto com o mesmo módulo e a mesma seção.

## Operadores lógicos e de comparação

Do mais frouxo ao mais forte (ordem do ca65): `||`/`.or`; `.xor`; `&&`/`.and`;
`==` `!=` `<>` `<` `>` `<=` `>=` (ou `.eq` `.ne` `.lt` `.gt` `.le` `.ge`); `|`/`.bitor`;
`^`/`.bitxor`; `&`/`.bitand`; `<<` `>>` (`.shl` `.shr`); `+` `-`; `*` `/` `.mod`.
Comparações são sem sinal em 16 bits e dão 1 ou 0. `&&` e `||` avaliam os dois lados.
Prefixos: `!x` e `.not x` dão 1 se `x` é 0; `-`, `~`, `<` e `>` como antes.

Um operador binário só é reconhecido logo depois de um operando. Por isso `a < <b`
compara `a` com o byte baixo de `b`. Um `<`/`>` no começo da expressão continua
pegando o byte da expressão inteira: `<a == 3` é `<(a == 3)`; use `(<a) == 3`.
Operadores com ponto precisam estar separados (`a .lt b`, não `a.ltb`). Uma linha
`nome = expr` só é atribuição com um único nome antes de um `=` sozinho, então
`.byte a == 1` é dado, não atribuição.

## `.union` e agregados aninhados

`.union Nome[, base]` ... `.endunion`: todo membro começa no mesmo deslocamento e o
tamanho é o do maior. Dentro de `.struct` ou `.union`:

- `.struct` / `.union` sem nome, até `.endstruct` / `.endunion`, junta os campos ao
  agregado de fora no deslocamento atual;
- `campo .struct` / `campo .union` cria um membro com nome; os campos internos são
  `Nome::campo::interno` e `.sizeof(Nome::campo)` dá o tamanho dele.

No máximo 16 níveis. Erros: falta de fechamento, fechamento trocado, campo repetido
(também entre níveis achatados) e agregado maior que 64 KiB.

## Debug `.dbg` completo

`Linked::to_dbg_full()` escreve o formato 2.0 do ld65 com `file`, `mod`, `seg`,
`span`, `scope`, `line` e `sym` (com `scope` e `def`). `file` tem `size=0` e
`mtime=0x00000000`, porque os objetos não guardam isso. Cada linha que gera bytes
vira um `span` até a próxima linha da mesma seção; os spans cobrem os bytes de cada
segmento sem buraco. `to_dbg()` continua só com `seg` e `sym`.

## Desmontagem que remonta

`disasm::to_source(mem, start, end, variante, símbolos, cobertura)` gera fonte que,
montado em `start`, devolve exatamente os mesmos bytes. Com cobertura, só o que rodou
como opcode vira instrução; o resto vira `.byte`. Uma instrução só sai como mnemônico
se aquela linha sozinha remonta igual; senão vai para `.byte`. Com símbolos, os
nomes dentro da faixa viram `nome:` e os de fora viram `nome = $XXXX`; se o texto com
nomes não remontar igual, sai o texto numérico (`named == false`).

## `.proc` / `.endproc` e `.export`

`.proc nome` define `nome` no endereço atual e abre um escopo até `.endproc`
(até 16 níveis). Rótulos e constantes `=` de dentro viram `nome::rótulo` para o
resto do programa; dentro do escopo, e dos escopos aninhados, vale o nome curto, e a
definição mais interna ganha. `inner::x` funciona a partir do escopo de fora. Cada
`.proc` também é um `.scope`, então rótulos `@` não vazam. `.export a, b` dentro de
um `.proc` define `a` e `b` globais, como apelidos, depois do `.endproc`. Só nomes do
próprio escopo podem ser exportados. No nível de cima, `.export` não muda nada.

Também corrigido: `b = a` depois de `a:` agora vale o endereço de `a`. Antes valia 0.

## Macros variádicas, `.paramcount`, `.ifblank` / `.ifnblank`

Um último parâmetro `resto...` recebe todos os argumentos a partir da sua posição,
unidos por `, `, e pode ficar vazio. `.paramcount` no corpo é o número de argumentos
passados. `.ifblank texto` é verdadeiro quando não há texto depois da diretiva, e
`.ifnblank` é o contrário. O texto não é avaliado. Os parâmetros continuam sendo
escritos como `\nome` no corpo.

## `.incbin` com início e tamanho

`.incbin "arq"[, início[, tamanho]]` copia só aquela faixa. Os dois valores são
expressões constantes. Dá erro se o início passar do fim, se a faixa passar do fim
ou se houver operandos sobrando.

## Ligador: `.import` / `.export`, coleta de seções e mapa de sobreposição

- `.export a, b` é o mesmo que `.global`. `.import a, b` declara nomes externos. Num
  objeto com algum `.import`, toda referência precisa ser local ou importada, e um
  nome importado não pode ser definido ali. Um import que ninguém exporta falha na
  ligação.
- `gc` e `keep <nome>` no script, ou `LinkOptions { gc, keep }`: só ficam as seções
  alcançáveis a partir das raízes pelas relocações. Uma raiz é o nome de uma seção
  ou de um símbolo. Seções fixas, como vetores, precisam de `keep`. Sem raiz, ou com
  uma raiz que não existe, é erro. `collect_garbage` devolve os objetos filtrados.
- `BankedLink::overlays()` e `overlay_report()` listam as janelas de endereço
  disputadas por seções de bancos diferentes, agrupadas de forma transitiva, e as
  seções fixas.

## Desmontagem com rótulos automáticos

`disasm::to_source_with(..., auto_labels = true)` dá um rótulo `L_XXXX` a cada alvo
de desvio, `JMP abs`, `JSR` e `BBR`/`BBS` que fica dentro da faixa e começa uma linha
de código. Símbolos que já existem têm prioridade. `JMP (ind)` não é seguido. O
resultado ainda é conferido montando de novo.

## Seleção de CPU

`.p02` (6502, só os opcodes documentados), `.psc02` (65SC02), `.pc02` (65C02, que é o
65SC02 mais `RMB`/`SMB`/`BBR`/`BBS` da Rockwell), `.pwc02` (W65C02, que soma `WAI` e
`STP`) e `.setcpu "6502"|"6502X"|"65SC02"|"65C02"|"R65C02"|"W65C02"`. Valem a partir
da linha onde aparecem e podem mudar no meio do arquivo. Sem seleção, tudo é aceito.
Uma instrução que a CPU atual não tem dá erro com a linha, o opcode e a CPU exigida.
Dados (`.byte` e semelhantes) nunca são conferidos. O assembler não tem mnemônicos
para os opcodes indocumentados do NMOS, então `6502X` aceita o mesmo que `6502`.
Não existe um nome próprio para chips da Synertek: um 65C02 com as instruções de bit
da Rockwell usa `R65C02`.

## Listagem com macros expandidas

`AsmOutput::listing_file(fonte)` mostra a linha de origem, `+` nas linhas vindas de
macro, o endereço, até 4 bytes por linha (o resto continua nas linhas seguintes) e o
texto. A linha da chamada da macro aparece antes da expansão. Linhas de `.include`
aparecem como `arquivo:n`. `ListingLine` agora tem `origin_line` e `expanded`.

## Bibliotecas `.lib`

`Library { members }`, `to_bytes`/`from_bytes` (`M65L`, versão 1, índice de exports
conferido na leitura). `resolve_libraries`/`link_with_libraries` puxam só os membros
que definem um nome ainda não resolvido, repetindo até não mudar mais nada. Se duas
bibliotecas exportam o mesmo nome, vale a primeira.

## Config do ligador no estilo ld65

`parse_link_config` aceita `MEMORY { NOME: start, size, fill, fillval; }` e
`SEGMENTS { NOME: load, run, align, start, define; }`. `link_with_config` liga no
endereço de execução (`run`) e grava os bytes no endereço de carga (`load`). As
imagens das áreas ficam em `images`. `define = yes` cria `__nome_load__`,
`__nome_run__` e `__nome_size__`. `copy_table()` / `copy_table_bytes()` listam o que o
código de partida precisa copiar: carga, execução e tamanho, em palavras little
endian, terminando com tamanho zero. Seções com `.org` próprio, seções sem segmento
e estouro de área dão erro.

## 6502X e opcodes indocumentados

`.setcpu "6502X"` monta os opcodes indocumentados do NMOS (`SLO RLA SRE RRA SAX
LAX DCP ISC ANC ALR ARR ANE SBX SHA SHS SHX SHY LAS JAM`, etc.) e os apelidos
comuns (`ASO LSE AXS XAA AHX TAS LXA DCM ISB INS ASR KIL HLT`). `.p02` /
`.setcpu "6502"` rejeitam esses mnemônicos. Sem seleção de CPU, tudo é aceito.
`inc a` / `dec a` tratam o `A` nu como acumulador, nunca como `$0A`.

## `.macpack`, `.ifconst` / `.ifref`, `.repeat`, `.local`, strings

- `.macpack longbranch|generic`: macros `jcc`…`jvs`/`jbra` e `add16`/`sub16`.
- `.ifconst expr`: verdadeiro se a expressão já é constante; `.ifref nome`: se o
  nome já apareceu num operando.
- `.repeat n, var` / `.endrep` (também `.rept` / `.endr`): `var = 0d0…` a cada volta.
- `.local a, b` no `.scope` atual: rótulos com esses nomes ficam locais ao escopo.
- `.concat`, `.left`, `.right`, `.mid`, `.sprintf` (lite: `%s %d %x %X %02X`).

## Ligador: `.weak`, mapa ld65, o65

`.weak` exporta um global fraco (vários fracos ok; um forte substitui).
`Linked::map_ld65()` escreve o mapa no layout do ld65. `O65File` lê/escreve um
subconjunto o65 32-bit (text+data+exports, sem relocações).
