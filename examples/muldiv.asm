; Shift-and-add multiply and restoring division.
;   mul8:  $10 * $11 -> $00 (low), $01 (high)          8 x 8 -> 16 bits
;   div16: $02/$03 / $12 -> quotient in $02/$03, remainder in $04
;          16 / 8 bits; the divisor must be below $80 so the partial
;          remainder never needs a ninth bit.
; Demo: 200 * 123 = 24600 ($6018) and 50000 / 7 = 7142 ($1BE6) remainder 6.

.org $8000
    lda #$c8        ; 200
    sta $10
    lda #$7b        ; 123
    sta $11
    jsr mul8
    lda #$50        ; dividend 50000 = $C350
    sta $02
    lda #$c3
    sta $03
    lda #7
    sta $12
    jsr div16
    brk

mul8:
    lda #0          ; A = running high byte
    ldx #8
    lsr $10         ; multiplier bit 0 -> C
mloop:
    bcc mskip
    clc
    adc $11         ; add the multiplicand into the high byte
mskip:
    ror a           ; shift the 16-bit product right; low half lands in $10
    ror $10
    dex
    bne mloop
    sta $01
    lda $10
    sta $00
    rts

div16:
    lda #0
    sta $04
    ldx #$10        ; 16 bits
dloop:
    asl $02         ; dividend/quotient left, top bit into the remainder
    rol $03
    rol $04
    lda $04
    sec
    sbc $12
    bcc dskip       ; remainder < divisor: quotient bit stays 0
    sta $04
    inc $02         ; quotient bit 1
dskip:
    dex
    bne dloop
    rts
