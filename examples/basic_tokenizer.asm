; BASIC-lite tokenizer and lister for an integer BASIC (original code for this
; project). Pass 1 crunches the program text at `source` into tokens at $0600;
; pass 2 lists the tokens back as text at $1000 (zero-terminated). Then BRK.
;
; Tokenized line: line number (16-bit, low byte first), body bytes, $00.
; The program ends with line number $FFFF.
; Keywords become one byte $80-$8D (table `kwtab`). Outside strings and REM,
; spaces are dropped; string literals and the text after REM are kept verbatim.
; The lister prints a space before a keyword unless one precedes it and a space
; after it unless it ends the line (none after REM, whose text keeps its own).
; $10 receives the status: 0 ok, 1 missing or non-ascending line number,
; 2 unterminated string. On an error pass 2 is skipped.

SRC  = $00
DST  = $02
P    = $04
NUM  = $06
LAST = $08
KL   = $0a
PREV = $0b
CNT  = $0c
ERR  = $10
TOKS = $0600
TEXT = $1000
T_REM = $8b

.org $8000
reset:
    ldx #$ff
    txs
    cld
    lda #<source
    sta SRC
    lda #>source
    sta SRC+1
    lda #<TOKS
    sta DST
    lda #>TOKS
    sta DST+1
    lda #$00
    sta LAST
    sta LAST+1
    sta ERR
    jmp line
badnum:
    lda #$01
    sta ERR
    brk
badstr:
    lda #$02
    sta ERR
    brk
finish:
    lda #$ff
    jsr out
    jsr out
    jmp lister

line:
    jsr skipsp
    ldy #$00
    lda (SRC),y
    beq finish
    jsr getnum
    bcs badnum
    lda LAST        ; must ascend: LAST < NUM
    cmp NUM
    lda LAST+1
    sbc NUM+1
    bcs badnum
    lda NUM
    sta LAST
    jsr out
    lda NUM+1
    sta LAST+1
    jsr out
body:
    ldy #$00
    lda (SRC),y
    beq endline
    cmp #$0d
    beq crline
    cmp #$20
    bne body1
    jsr advs
    jmp body
body1:
    cmp #$22
    beq string
    jsr matchkw
    bcs plain
    jsr out
    cmp #T_REM
    beq rem
    jmp body
plain:
    ldy #$00
    lda (SRC),y
    jsr out
    jsr advs
    jmp body
string:
    jsr out         ; opening quote
    jsr advs
str1:
    ldy #$00
    lda (SRC),y
    beq badstr
    cmp #$0d
    beq badstr
    jsr out
    jsr advs
    cmp #$22
    bne str1
    jmp body
rem:
    ldy #$00
    lda (SRC),y
    beq endline
    cmp #$0d
    beq crline
    jsr out
    jsr advs
    jmp rem
crline:
    jsr advs
endline:
    lda #$00
    jsr out
    jmp line

skipsp:
    ldy #$00
    lda (SRC),y
    cmp #$20
    beq skip1
    cmp #$0d
    bne skip2
skip1:
    jsr advs
    jmp skipsp
skip2:
    rts

advs:
    inc SRC
    bne advs1
    inc SRC+1
advs1:
    rts

out:
    ldy #$00
    sta (DST),y
    inc DST
    bne out1
    inc DST+1
out1:
    rts

; Decimal digits at SRC -> NUM, C set if there is no digit.
getnum:
    lda #$00
    sta NUM
    sta NUM+1
    sta CNT
gn1:
    ldy #$00
    lda (SRC),y
    sec
    sbc #$30
    cmp #$0a
    bcs gndone
    pha
    asl NUM
    rol NUM+1
    lda NUM
    ldx NUM+1
    asl NUM
    rol NUM+1
    asl NUM
    rol NUM+1
    clc
    adc NUM
    sta NUM
    txa
    adc NUM+1
    sta NUM+1
    pla
    clc
    adc NUM
    sta NUM
    bcc gn2
    inc NUM+1
gn2:
    inc CNT
    jsr advs
    jmp gn1
gndone:
    lda CNT
    cmp #$01        ; C set (error) when no digit: invert below
    lda #$00
    rol a
    eor #$01
    lsr a
    rts

; Keyword at SRC? C clear, A = token and SRC past it; otherwise C set.
matchkw:
    lda #<kwtab
    sta P
    lda #>kwtab
    sta P+1
mk1:
    ldy #$00
    lda (P),y
    beq mkno
    sta KL
mkcmp:
    lda (SRC),y
    iny
    cmp (P),y
    bne mknext
    cpy KL
    bne mkcmp
    iny
    lda (P),y       ; token
    pha
    clc
    lda SRC
    adc KL
    sta SRC
    bcc mk2
    inc SRC+1
mk2:
    pla
    clc
    rts
mknext:
    ldy #$00
    lda (P),y
    clc
    adc #$02
    adc P
    sta P
    bcc mk1
    inc P+1
    bne mk1
mkno:
    sec
    rts

; ---- pass 2: list ----
lister:
    lda #<TOKS
    sta SRC
    lda #>TOKS
    sta SRC+1
    lda #<TEXT
    sta DST
    lda #>TEXT
    sta DST+1
lline:
    ldy #$00
    lda (SRC),y
    sta NUM
    iny
    lda (SRC),y
    sta NUM+1
    and NUM
    cmp #$ff
    beq ldone
    jsr advs
    jsr advs
    jsr prdec
    lda #$20
    sta PREV
    jsr out
lbody:
    ldy #$00
    lda (SRC),y
    beq lend
    bmi lkw
    sta PREV
    jsr out
    jsr advs
    jmp lbody
lkw:
    pha
    lda PREV
    cmp #$20
    beq lkw1
    lda #$20
    jsr out
lkw1:
    pla
    jsr kwname      ; prints the keyword, leaves its last letter in PREV
    jsr advs
    ldy #$00
    lda (SRC),y
    beq lbody
    lda KL
    cmp #T_REM
    beq lbody
    lda #$20
    sta PREV
    jsr out
    jmp lbody
lend:
    lda #$0d
    jsr out
    jsr advs
    jmp lline
ldone:
    lda #$00
    jsr out
    brk

; Print the name of token A; KL receives the token.
kwname:
    sta KL
    lda #<kwtab
    sta P
    lda #>kwtab
    sta P+1
kn1:
    ldy #$00
    lda (P),y
    tay
    iny
    lda (P),y
    cmp KL
    beq kn2
    dey
    tya
    clc
    adc #$02
    adc P
    sta P
    bcc kn1
    inc P+1
    bne kn1
kn2:
    ldx #$01
kn3:
    txa
    tay
    lda (P),y
    sta PREV
    jsr out
    inx
    txa
    ldy #$00
    cmp (P),y
    bcc kn3
    beq kn3
    rts

; NUM in decimal (destroys NUM).
prdec:
    lda #$00
    sta CNT
pd1:
    lda #$00
    ldx #$10
pd2:
    asl NUM
    rol NUM+1
    rol a
    cmp #$0a
    bcc pd3
    sbc #$0a
    inc NUM
pd3:
    dex
    bne pd2
    pha
    inc CNT
    lda NUM
    ora NUM+1
    bne pd1
pd4:
    pla
    ora #$30
    jsr out
    dec CNT
    bne pd4
    rts

; len, name, token. GOSUB/GOTO precede TO so the longer words win.
kwtab:
    .byte 5, "PRINT", $80
    .byte 3, "LET", $81
    .byte 2, "IF", $82
    .byte 4, "THEN", $83
    .byte 4, "GOTO", $84
    .byte 5, "GOSUB", $85
    .byte 6, "RETURN", $86
    .byte 3, "END", $87
    .byte 3, "FOR", $88
    .byte 2, "TO", $89
    .byte 4, "NEXT", $8a
    .byte 3, "REM", $8b
    .byte 5, "INPUT", $8c
    .byte 4, "STEP", $8d
    .byte 0

source:
    .byte "10 REM COUNT TO FIVE", $0d
    .byte "20 LET  A = 1", $0d
    .byte "30 PRINT ", $22, "A IS ", $22, ";A", $0d
    .byte "40 LET A=A+1", $0d
    .byte "50 IF A<6 THEN GOTO 30", $0d
    .byte "60 FOR I=1 TO 3 STEP 1", $0d
    .byte "70 GOSUB 100", $0d
    .byte "80 NEXT I", $0d
    .byte "90 END", $0d
    .byte "100 PRINT ", $22, "SUB", $22, ":RETURN", $0d
    .byte 0
