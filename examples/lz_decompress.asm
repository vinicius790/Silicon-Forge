; LZSS decompressor (original code for this project). Unpacks the stream at
; `packed` to $0400 and executes BRK.
;
; Stream format:
;   2 bytes   uncompressed length, low byte first
;   groups    a flag byte, then up to 8 items, flag bit 0 first:
;             bit 1  literal: one byte copied to the output
;             bit 0  match: two bytes a, b; distance = ((b & $F0) << 4 | a) + 1
;                    (1..4096 bytes back), length = (b & $0F) + 3 (3..18).
;                    Copying runs byte by byte, so a match may overlap its own
;                    output (distance 1 repeats the last byte).
; Decoding stops as soon as the declared length has been written.

SRC  = $00
DST  = $02
LEFT = $04
FLAGS = $06
BITS = $07
CP   = $08
LEN  = $0a

.org $8000
reset:
    ldx #$ff
    txs
    cld
    lda #<packed
    sta SRC
    lda #>packed
    sta SRC+1
    lda #$00
    sta DST
    lda #$04
    sta DST+1
    jsr lzdec
    brk

lzdec:
    jsr getsrc
    sta LEFT
    jsr getsrc
    sta LEFT+1
group:
    lda LEFT
    ora LEFT+1
    beq lz_done
    jsr getsrc
    sta FLAGS
    lda #$08
    sta BITS
item:
    lda LEFT
    ora LEFT+1
    beq lz_done
    lsr FLAGS
    bcc match
    jsr getsrc
    jsr put
    jmp nexti
match:
    jsr getsrc
    sta CP
    jsr getsrc
    pha
    and #$0f
    clc
    adc #$03
    sta LEN
    pla
    lsr a
    lsr a
    lsr a
    lsr a
    sta CP+1        ; CP = distance - 1
    sec             ; CP = DST - (distance - 1) - 1
    lda DST
    sbc CP
    sta CP
    lda DST+1
    sbc CP+1
    sta CP+1
    lda CP
    bne m1
    dec CP+1
m1:
    dec CP
copy:
    ldy #$00
    lda (CP),y
    jsr put
    inc CP
    bne c1
    inc CP+1
c1:
    lda LEFT
    ora LEFT+1
    beq lz_done
    dec LEN
    bne copy
nexti:
    dec BITS
    bne item
    jmp group
lz_done:
    rts

; A -> (DST)+, LEFT -= 1
put:
    ldy #$00
    sta (DST),y
    inc DST
    bne put1
    inc DST+1
put1:
    lda LEFT
    bne put2
    dec LEFT+1
put2:
    dec LEFT
    rts

getsrc:
    ldy #$00
    lda (SRC),y
    inc SRC
    bne gs1
    inc SRC+1
gs1:
    rts

; "TO BE OR NOT TO BE, THAT IS THE QUESTION. TO BE OR NOT TO BE, THAT IS THE
; ANSWER. ABABABABABABABABABABABAB!" (107 bytes in 67)
packed:
    .byte $6b, $00, $ff, $54, $4f, $20, $42, $45, $20, $4f, $52, $df
    .byte $20, $4e, $4f, $54, $20, $0c, $02, $2c, $20, $7f, $54, $48
    .byte $41, $54, $20, $49, $53, $07, $00, $ff, $45, $20, $51, $55
    .byte $45, $53, $54, $49, $c7, $4f, $4e, $2e, $1c, $03, $29, $0f
    .byte $29, $06, $41, $4e, $ff, $53, $57, $45, $52, $2e, $20, $41
    .byte $42, $04, $01, $0f, $01, $01, $21
