; Forth-lite: a text-interpreting Forth subset (original code for this project).
; It interprets the program at `source` and writes its output to $0400, ending
; with a zero byte, then executes BRK.
;
; Cells are 16 bits. Numbers in the Forth source are unsigned decimal.
; Words:  + - * DUP DROP SWAP OVER = < > . EMIT CR
;         IF ELSE THEN BEGIN UNTIL : ;
; Comparisons are unsigned and leave -1 (true) or 0. `.` prints unsigned decimal
; and a space. `: NAME ... ;` records NAME and the text of its body; calling it
; interprets that text with the caller's input position on a return stack, so
; definitions may recurse. IF/ELSE/THEN skip text by counting nesting.
; An unknown word prints "?" and stops.
;
; Data stack: X indexes it, low bytes at $20+X, high bytes at $40+X, grows down
; from X = $20 (empty). Return stack of input positions at $60+Y / $70+Y.

IP    = $00         ; input pointer
OUTP  = $02         ; output pointer
TOK   = $04         ; current token
TLEN  = $06
W     = $08         ; scratch cells
T     = $0a
N     = $0c
DEPTH = $0e         ; IF-skip nesting, 0 = executing
UDP   = $10         ; next free user dictionary entry
FOUND = $12         ; handler or body address
RSP   = $14         ; return stack depth
CNT   = $15
STLO  = $20
STHI  = $40
RSLO  = $60
RSHI  = $70
UDICT = $0300       ; entries: len, name lo/hi, body lo/hi

.org $8000
reset:
    ldx #$ff
    txs
    cld
    lda #<source
    sta IP
    lda #>source
    sta IP+1
    lda #$00
    sta OUTP
    sta DEPTH
    sta RSP
    sta UDP
    lda #$04
    sta OUTP+1
    lda #>UDICT
    sta UDP+1
    ldx #$20

interp:
    jsr token
    lda TLEN
    bne have
halt:
    ldy #$00
    tya
    sta (OUTP),y
    brk
have:
    lda DEPTH
    bne skipping
    jsr findu
    bcs nouser
    ldy RSP         ; call a definition: save IP, interpret its body
    lda IP
    sta RSLO,y
    lda IP+1
    sta RSHI,y
    inc RSP
    lda FOUND
    sta IP
    lda FOUND+1
    sta IP+1
    jmp interp
nouser:
    jsr findb
    bcs number
    jsr callf
    jmp interp
number:
    jsr parsenum
    bcs unknown
    dex
    lda W
    sta STLO,x
    lda W+1
    sta STHI,x
    jmp interp
unknown:
    lda #$3f        ; '?'
    jsr emit
    jmp halt

; Skipping a false IF branch: only IF, ELSE and THEN matter.
skipping:
    jsr findb
    bcs skipnext
    lda FOUND+1
    cmp #>w_if
    bne skipnext    ; all three handlers share one page (asserted below)
    lda FOUND
    cmp #<w_if
    beq skipif
    cmp #<w_then
    beq skipthen
    cmp #<w_else
    bne skipnext
    lda DEPTH
    cmp #$01
    bne skipnext
    lda #$00
    sta DEPTH
    beq skipnext
skipif:
    inc DEPTH
    bne skipnext
skipthen:
    dec DEPTH
skipnext:
    jmp interp

callf:
    jmp (FOUND)

; Next blank-delimited token at IP -> TOK/TLEN (TLEN 0 at the end of text).
token:
    ldy #$00
    sty TLEN
tskip:
    lda (IP),y
    beq tend
    cmp #$21
    bcs tstart
    jsr adv
    jmp tskip
tstart:
    lda IP
    sta TOK
    lda IP+1
    sta TOK+1
tscan:
    lda (IP),y
    cmp #$21
    bcc tend
    jsr adv
    inc TLEN
    bne tscan
tend:
    rts

; IP += 1, keeping Y.
adv:
    inc IP
    bne adv1
    inc IP+1
adv1:
    rts

emit:
    sty CNT+1
    ldy #$00
    sta (OUTP),y
    inc OUTP
    bne emit1
    inc OUTP+1
emit1:
    ldy CNT+1
    rts

; Built-in dictionary lookup: C clear and FOUND = handler on a match.
findb:
    lda #<dict
    sta T
    lda #>dict
    sta T+1
fb1:
    ldy #$00
    lda (T),y
    beq fbno
    cmp TLEN
    bne fbskip
fbcmp:
    lda (TOK),y
    iny
    cmp (T),y
    bne fbskip
    cpy TLEN
    bne fbcmp
    iny
    lda (T),y
    sta FOUND
    iny
    lda (T),y
    sta FOUND+1
    clc
    rts
fbskip:
    ldy #$00
    lda (T),y
    clc
    adc #$03
    adc T
    sta T
    bcc fb1
    inc T+1
    bne fb1
fbno:
    sec
    rts

; User dictionary lookup, newest match wins: C clear and FOUND = body.
findu:
    lda #<UDICT
    sta T
    lda #>UDICT
    sta T+1
    lda #$ff
    sta CNT         ; $FF: nothing found yet
fu1:
    lda T
    cmp UDP
    bne fu2
    lda T+1
    cmp UDP+1
    beq fudone
fu2:
    ldy #$00
    lda (T),y
    cmp TLEN
    bne funext
    iny
    lda (T),y
    sta N
    iny
    lda (T),y
    sta N+1
    ldy #$00
fucmp:
    lda (TOK),y
    cmp (N),y
    bne funext
    iny
    cpy TLEN
    bne fucmp
    ldy #$03
    lda (T),y
    sta FOUND
    iny
    lda (T),y
    sta FOUND+1
    lda #$00
    sta CNT
funext:
    clc
    lda T
    adc #$05
    sta T
    bcc fu1
    inc T+1
    bne fu1
fudone:
    lda CNT
    cmp #$01        ; C clear only when CNT = 0 (found)
    rts

; Unsigned decimal token -> W, C clear; C set if a character is not a digit.
parsenum:
    lda #$00
    sta W
    sta W+1
    ldy #$00
pn1:
    lda (TOK),y
    sec
    sbc #$30
    cmp #$0a
    bcs pnbad
    pha
    asl W           ; W*2
    rol W+1
    lda W
    sta T
    lda W+1
    sta T+1
    asl W           ; W*8
    rol W+1
    asl W
    rol W+1
    clc
    lda W
    adc T
    sta W
    lda W+1
    adc T+1
    sta W+1
    pla
    clc
    adc W
    sta W
    bcc pn2
    inc W+1
pn2:
    iny
    cpy TLEN
    bne pn1
    clc
    rts
pnbad:
    sec
    rts

; ---- word handlers (X = data stack) ----
w_if:
    lda STLO,x
    ora STHI,x
    inx
    cmp #$00
    bne wif1
    lda #$01
    sta DEPTH
wif1:
    rts
w_else:
    lda #$01        ; the true branch ran: skip to THEN
    sta DEPTH
    rts
w_then:
    rts

w_add:
    clc
    lda STLO,x
    adc STLO+1,x
    sta STLO+1,x
    lda STHI,x
    adc STHI+1,x
    sta STHI+1,x
    inx
    rts
w_sub:
    sec
    lda STLO+1,x
    sbc STLO,x
    sta STLO+1,x
    lda STHI+1,x
    sbc STHI,x
    sta STHI+1,x
    inx
    rts
w_mul:
    lda STLO,x
    sta W
    lda STHI,x
    sta W+1
    lda STLO+1,x
    sta T
    lda STHI+1,x
    sta T+1
    lda #$00
    sta N
    sta N+1
    ldy #$10
wm1:
    lsr T+1
    ror T
    bcc wm2
    clc
    lda N
    adc W
    sta N
    lda N+1
    adc W+1
    sta N+1
wm2:
    asl W
    rol W+1
    dey
    bne wm1
    inx
    lda N
    sta STLO,x
    lda N+1
    sta STHI,x
    rts
w_dup:
    dex
    lda STLO+1,x
    sta STLO,x
    lda STHI+1,x
    sta STHI,x
    rts
w_drop:
    inx
    rts
w_swap:
    lda STLO,x
    ldy STLO+1,x
    sta STLO+1,x
    sty STLO,x
    lda STHI,x
    ldy STHI+1,x
    sta STHI+1,x
    sty STHI,x
    rts
w_over:
    dex
    lda STLO+2,x
    sta STLO,x
    lda STHI+2,x
    sta STHI,x
    rts
w_eq:
    lda STLO,x
    cmp STLO+1,x
    bne wfalse2
    lda STHI,x
    cmp STHI+1,x
    bne wfalse2
    beq wtrue2
w_lt:               ; second < top
    lda STLO+1,x
    cmp STLO,x
    lda STHI+1,x
    sbc STHI,x
    bcc wtrue2
    bcs wfalse2
w_gt:               ; second > top
    lda STLO,x
    cmp STLO+1,x
    lda STHI,x
    sbc STHI+1,x
    bcc wtrue2
wfalse2:
    inx
    lda #$00
    beq wflag
wtrue2:
    inx
    lda #$ff
wflag:
    sta STLO,x
    sta STHI,x
    rts
w_dot:
    lda STLO,x
    sta W
    lda STHI,x
    sta W+1
    inx
    lda #$00
    sta CNT
wd1:
    lda #$00        ; W /= 10, remainder in A
    ldy #$10
wd2:
    asl W
    rol W+1
    rol a
    cmp #$0a
    bcc wd3
    sbc #$0a
    inc W
wd3:
    dey
    bne wd2
    pha
    inc CNT
    lda W
    ora W+1
    bne wd1
wd4:
    pla
    ora #$30
    jsr emit
    dec CNT
    bne wd4
    lda #$20
    jmp emit
w_emit:
    lda STLO,x
    inx
    jmp emit
w_cr:
    lda #$0d
    jmp emit
w_begin:
    ldy RSP
    lda IP
    sta RSLO,y
    lda IP+1
    sta RSHI,y
    inc RSP
    rts
w_until:
    lda STLO,x
    ora STHI,x
    inx
    ldy RSP
    cmp #$00
    bne wu1
    lda RSLO-1,y    ; false: loop back, keep the saved position
    sta IP
    lda RSHI-1,y
    sta IP+1
    rts
wu1:
    dec RSP
    rts
w_colon:
    jsr token       ; the name
    ldy #$00
    lda TLEN
    sta (UDP),y
    iny
    lda TOK
    sta (UDP),y
    iny
    lda TOK+1
    sta (UDP),y
    iny
    lda IP          ; body starts right after the name
    sta (UDP),y
    iny
    lda IP+1
    sta (UDP),y
    clc
    lda UDP
    adc #$05
    sta UDP
    bcc wc1
    inc UDP+1
wc1:
    jsr token       ; skip the body up to ';'
    lda TLEN
    beq wc2
    cmp #$01
    bne wc1
    ldy #$00
    lda (TOK),y
    cmp #$3b
    bne wc1
wc2:
    rts
w_semi:
    lda RSP
    beq ws1
    dec RSP
    ldy RSP
    lda RSLO,y
    sta IP
    lda RSHI,y
    sta IP+1
ws1:
    rts

    .assert hi(w_if) == hi(w_else), "IF/ELSE handlers must share a page"
    .assert hi(w_if) == hi(w_then), "IF/THEN handlers must share a page"

dict:
    .byte 1, "+"
    .word w_add
    .byte 1, "-"
    .word w_sub
    .byte 1, "*"
    .word w_mul
    .byte 3, "DUP"
    .word w_dup
    .byte 4, "DROP"
    .word w_drop
    .byte 4, "SWAP"
    .word w_swap
    .byte 4, "OVER"
    .word w_over
    .byte 1, "="
    .word w_eq
    .byte 1, "<"
    .word w_lt
    .byte 1, ">"
    .word w_gt
    .byte 1, "."
    .word w_dot
    .byte 4, "EMIT"
    .word w_emit
    .byte 2, "CR"
    .word w_cr
    .byte 2, "IF"
    .word w_if
    .byte 4, "ELSE"
    .word w_else
    .byte 4, "THEN"
    .word w_then
    .byte 5, "BEGIN"
    .word w_begin
    .byte 5, "UNTIL"
    .word w_until
    .byte 1, ":"
    .word w_colon
    .byte 1, ";"
    .word w_semi
    .byte 0

source:
    .byte ": SQ DUP * ; "
    .byte ": FACT DUP 1 > IF DUP 1 - FACT * ELSE THEN ; "
    .byte "5 FACT . 7 SQ . "
    .byte "10 BEGIN DUP . 1 - DUP 0 = UNTIL DROP "
    .byte "3 4 < IF 65 EMIT ELSE 66 EMIT THEN "
    .byte "4 3 < IF 67 EMIT ELSE 68 EMIT THEN CR "
    .byte "200 300 * . 7 3 OVER - SWAP . . "
    .byte 0
