; Four-byte binary floating point: add and multiply (original code for this
; project, in the layout of Microsoft BASIC's 32-bit format).
;
;   byte 0  exponent, bias $80; 0 means the value is zero
;   byte 1  sign (bit 7) and mantissa bits 22-16 (the leading 1 is implied)
;   byte 2  mantissa bits 15-8
;   byte 3  mantissa bits 7-0
;   value = (-1)^sign * (1.m22..m0 / 2) * 2^(exponent - $80), so 1.0 = $81 00 00 00
;
; Results are truncated, not rounded. Overflow saturates to the largest
; magnitude and counts in $2B; underflow gives zero.
;
; The driver walks `optable`: entries of an operator byte ('+' or '*') and two
; operands, until a zero operator, writing each 4-byte result from $0400.

E1  = $10           ; FAC1: exponent, mantissa high/mid/low, sign
M1H = $11
M1M = $12
M1L = $13
S1  = $14
E2  = $18           ; FAC2, same layout
M2H = $19
M2M = $1a
M2L = $1b
S2  = $1c
PR  = $20           ; 48-bit product, PR+0 most significant
TP  = $26           ; table pointer
RP  = $28           ; result pointer
ERR = $2b

.org $8000
reset:
    ldx #$ff
    txs
    cld
    lda #<optable
    sta TP
    lda #>optable
    sta TP+1
    lda #$00
    sta RP
    sta ERR
    lda #$04
    sta RP+1
next:
    ldy #$00
    lda (TP),y
    beq done
    pha
    ldy #$01
    ldx #$00
    jsr unpack      ; operand A -> FAC1
    ldy #$05
    ldx #$08
    jsr unpack      ; operand B -> FAC2
    pla
    cmp #$2a        ; '*'
    bne doadd
    jsr fmul
    jmp store
doadd:
    jsr fadd
store:
    jsr pack
    clc
    lda TP
    adc #$09
    sta TP
    bcc next
    inc TP+1
    bne next
done:
    brk

; Four bytes at (TP),Y -> FAC at E1+X.
unpack:
    lda (TP),y
    sta E1,x
    iny
    lda (TP),y
    and #$80
    sta S1,x
    lda (TP),y
    ora #$80        ; implied leading 1
    sta M1H,x
    iny
    lda (TP),y
    sta M1M,x
    iny
    lda (TP),y
    sta M1L,x
    rts

; FAC1 -> four bytes at (RP), RP += 4.
pack:
    ldy #$00
    lda E1
    bne pack1
    sta (RP),y
    iny
    sta (RP),y
    iny
    sta (RP),y
    iny
    sta (RP),y
    beq pack2
pack1:
    sta (RP),y
    iny
    lda M1H
    and #$7f
    ora S1
    sta (RP),y
    iny
    lda M1M
    sta (RP),y
    iny
    lda M1L
    sta (RP),y
pack2:
    clc
    lda RP
    adc #$04
    sta RP
    bcc pack3
    inc RP+1
pack3:
    rts

zero:
    lda #$00
    sta E1
    sta S1
    rts

overflow:
    inc ERR
    lda #$ff
    sta E1
    sta M1H
    sta M1M
    sta M1L
    rts

; FAC1 = FAC1 * FAC2
fmul:
    lda E1
    beq zero
    lda E2
    beq zero
    lda S1
    eor S2
    sta S1
    clc
    lda E1
    adc E2
    bcs fm_hi
    sec             ; sum < $100: exponent = sum - $80, must stay above 0
    sbc #$80
    beq zero
    bcc zero
    sta E1
    jmp fm_go
fm_hi:
    cmp #$80        ; sum >= $100: exponent = sum - $80 = A + $80
    bcs overflow
    ora #$80
    sta E1
fm_go:
    lda #$00
    sta PR
    sta PR+1
    sta PR+2
    lda M2H
    sta PR+3
    lda M2M
    sta PR+4
    lda M2L
    sta PR+5
    ldx #$18        ; 24 multiplier bits, LSB first
fm_loop:
    clc
    lda PR+5
    and #$01
    beq fm_shift
    clc
    lda PR+2
    adc M1L
    sta PR+2
    lda PR+1
    adc M1M
    sta PR+1
    lda PR
    adc M1H
    sta PR
fm_shift:
    ror PR
    ror PR+1
    ror PR+2
    ror PR+3
    ror PR+4
    ror PR+5
    dex
    bne fm_loop
    lda PR          ; product in [1/4, 1): at most one normalising shift
    bmi fm_keep
    asl PR+3
    rol PR+2
    rol PR+1
    rol PR
    dec E1
    bne fm_keep
    jmp zero
fm_keep:
    lda PR
    sta M1H
    lda PR+1
    sta M1M
    lda PR+2
    sta M1L
    rts

; FAC1 = FAC1 + FAC2
fadd:
    lda E2
    beq fa_rts
    lda E1
    bne fa_both
    ldx #$04        ; FAC1 is zero: result is FAC2
fa_copy:
    lda E2,x
    sta E1,x
    dex
    bpl fa_copy
fa_rts:
    rts
fa_both:
    cmp E2
    bcs fa_order
    ldx #$04        ; make FAC1 the larger exponent
fa_swap:
    lda E1,x
    ldy E2,x
    sta E2,x
    sty E1,x
    dex
    bpl fa_swap
fa_order:
    sec
    lda E1
    sbc E2
    cmp #$18
    bcs fa_rts      ; FAC2 is below the last mantissa bit
    tax
    beq fa_aligned
fa_shr:
    lsr M2H
    ror M2M
    ror M2L
    dex
    bne fa_shr
fa_aligned:
    lda S1
    cmp S2
    bne fa_sub
    clc
    lda M1L
    adc M2L
    sta M1L
    lda M1M
    adc M2M
    sta M1M
    lda M1H
    adc M2H
    sta M1H
    bcc fa_rts
    ror M1H         ; carry out: shift it back in, exponent + 1
    ror M1M
    ror M1L
    inc E1
    bne fa_rts
    jmp overflow
fa_sub:
    sec
    lda M1L
    sbc M2L
    sta M1L
    lda M1M
    sbc M2M
    sta M1M
    lda M1H
    sbc M2H
    sta M1H
    bcs fa_norm
    lda #$00        ; FAC2 was larger: negate, take its sign
    sec
    sbc M1L
    sta M1L
    lda #$00
    sbc M1M
    sta M1M
    lda #$00
    sbc M1H
    sta M1H
    lda S2
    sta S1
fa_norm:
    lda M1H
    ora M1M
    ora M1L
    bne fa_nloop
    jmp zero
fa_nloop:
    lda M1H
    bmi fa_done
    asl M1L
    rol M1M
    rol M1H
    dec E1
    bne fa_nloop
    jmp zero
fa_done:
    rts

optable:
    .byte "+", $81, $40, $00, $00, $82, $10, $00, $00   ; 1.5 + 2.25 = 3.75
    .byte "*", $82, $40, $00, $00, $80, $00, $00, $00   ; 3 * 0.5 = 1.5
    .byte "+", $82, $a0, $00, $00, $81, $00, $00, $00   ; -2.5 + 1 = -1.5
    .byte "*", $84, $20, $00, $00, $84, $20, $00, $00   ; 10 * 10 = 100
    .byte "+", $81, $00, $00, $00, $81, $80, $00, $00   ; 1 + -1 = 0
    .byte "*", $83, $c0, $00, $00, $82, $40, $00, $00   ; -6 * 3 = -18
    .byte "+", $00, $00, $00, $00, $83, $60, $00, $00   ; 0 + 7 = 7
    .byte "*", $f0, $00, $00, $00, $f0, $00, $00, $00   ; overflow saturates
    .byte 0
