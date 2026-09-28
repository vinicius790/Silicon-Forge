; Terminal original deste crate: 32 KiB de RAM em $0000-$7FFF, ROM a partir de
; $E000, 6551 em $D000-$D003. Não é uma ROM de máquina histórica.
;
; Editor de linha:
;   $20..$7E e $80..$FF  guardados em $0200+len e ecoados (até 64 bytes)
;   BS ($08) ou DEL ($7F) apagam o último byte: ecoa BS, espaço, BS
;   CR ($0D)             ecoa CR LF e executa BRK; a linha fica em $0200, len em $10
;   qualquer outro byte, linha cheia ou apagar com linha vazia: ecoa BEL ($07)

.org $E000

reset:
    lda #$0b        ; 6551: DTR on (receiver enabled), IRQs off
    sta $d002
    ldx #0
    stx $10
loop:
    jsr getc
    cmp #$0d
    beq enter
    cmp #$08
    beq rubout
    cmp #$7f
    beq rubout
    cmp #$20
    bcc bell
    ldx $10
    cpx #$40
    bcs bell
    sta $0200,x
    inc $10
    jsr putc
    jmp loop
rubout:
    ldx $10
    beq bell
    dec $10
    lda #$08
    jsr putc
    lda #$20
    jsr putc
    lda #$08
    jsr putc
    jmp loop
bell:
    lda #$07
    jsr putc
    jmp loop
enter:
    jsr putc
    lda #$0a
    jsr putc
    brk

getc:
    lda $d001
    and #$08
    beq getc
    lda $d000
    rts

putc:
    pha
wait_tdre:
    lda $d001
    and #$10
    beq wait_tdre
    pla
    sta $d000
    rts

.org $FFFC
.word reset
