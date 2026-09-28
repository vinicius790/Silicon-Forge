; Wozmon-style hex monitor (original code written for this project, not the
; Apple-1 ROM). It reads command lines from a script in ROM instead of a
; keyboard and writes everything it would print to a buffer at $0400, ending
; with a zero byte, then executes BRK.
;
; Command language, like Wozmon:
;   AAAA          examine: prints CR "AAAA: BB"
;   AAAA.BBBB     block examine: after the first address, prints every byte up
;                 to BBBB, starting a new "AAAA:" line on each multiple of 8
;   AAAA: BB CC   store: prints the old byte at AAAA, then stores BB, CC, ...
;   R             run: JSR to the last examined address (the routine RTSes)
; Spaces separate items, CR ends a line, other characters are ignored.
; Numbers are hex; only the last four digits count.

INP  = $00          ; script pointer
OUTP = $02          ; output pointer
XAML = $04          ; examine address
XAMH = $05
STL  = $06          ; store address
STH  = $07
L    = $08          ; number being parsed
H    = $09
MODE = $0a          ; 0 examine, '.' block, ':' store

.org $8000
reset:
    ldx #$ff
    txs
    cld
    lda #<script
    sta INP
    lda #>script
    sta INP+1
    lda #$00
    sta OUTP
    lda #$04
    sta OUTP+1
    lda #$00
    sta MODE
    beq item
setmode:
    sta MODE
    jmp item
run:
    jsr runxam
    jmp item
item:
    ldy #$00
    lda (INP),y
    bne notend
    sta (OUTP),y    ; terminate the output text
    brk
notend:
    jsr adv
    cmp #$0d
    bne notcr
    lda #$00
    sta MODE
    beq item
notcr:
    cmp #$20
    beq item
    cmp #$2e        ; '.'
    beq setmode
    cmp #$3a        ; ':'
    beq setmode
    cmp #$52        ; 'R'
    beq run
    jsr hexval
    bcs item        ; not a hex digit: ignore
    ldx #$00
    stx L
    stx H
digit:
    asl a           ; digit to the high nibble
    asl a
    asl a
    asl a
    ldx #$04
shl:
    asl a           ; shift it into H:L one bit at a time
    rol L
    rol H
    dex
    bne shl
    ldy #$00
    lda (INP),y     ; peek at the next character
    jsr hexval
    bcs gotnum
    jsr adv
    jmp digit
gotnum:
    lda MODE
    cmp #$3a
    beq store
    cmp #$2e
    beq block
    lda L           ; examine: both pointers follow the number
    sta XAML
    sta STL
    lda H
    sta XAMH
    sta STH
    jsr header
    jsr prbyte
    jmp item
store:
    ldy #$00
    lda L
    sta (STL),y
    inc STL
    bne store1
    inc STH
store1:
    jmp item
block:
    lda XAML        ; stop once XAM >= number
    cmp L
    lda XAMH
    sbc H
    bcs blkdone
    inc XAML
    bne blk1
    inc XAMH
blk1:
    lda XAML
    and #$07
    bne blk2
    jsr header
blk2:
    jsr prbyte
    jmp block
blkdone:
    lda #$00
    sta MODE
    jmp item
runxam:
    jmp (XAML)

; A = ASCII hex digit -> A = 0..15 with C clear, otherwise C set.
hexval:
    cmp #$30
    bcc hexno
    cmp #$3a
    bcc hexdec
    cmp #$41
    bcc hexno
    cmp #$47
    bcs hexno
    sbc #$36        ; C is clear here: A - $37
    clc
    rts
hexdec:
    and #$0f
    clc
    rts
hexno:
    sec
    rts

adv:
    inc INP
    bne adv1
    inc INP+1
adv1:
    rts

emit:
    ldy #$00
    sta (OUTP),y
    inc OUTP
    bne emit1
    inc OUTP+1
emit1:
    rts

header:
    lda #$0d
    jsr emit
    lda XAMH
    jsr prhex
    lda XAML
    jsr prhex
    lda #$3a
    jmp emit

prbyte:
    lda #$20
    jsr emit
    ldy #$00
    lda (XAML),y
prhex:
    pha
    lsr a
    lsr a
    lsr a
    lsr a
    jsr prnib
    pla
    and #$0f
prnib:
    ora #$30
    cmp #$3a
    bcc prnib1
    adc #$06        ; C set: +7 turns ':' into 'A'
prnib1:
    jmp emit

; A routine the script stores and runs: LDA #$5A / STA $0310 / RTS.
script:
    .byte "0300: 11 22 33 44 55 66 77 88 99", $0d
    .byte "0300.0309", $0d
    .byte "0301", $0d
    .byte "0320: A9 5A 8D 10 03 60", $0d
    .byte "0320 R", $0d
    .byte "0310", $0d
    .byte "x0300", $0d
    .byte 0
