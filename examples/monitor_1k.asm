; Monitor original de 1 KiB deste crate. Não é o WozMon e não copia ROM alheia.
; A imagem ocupa $FC00..=$FFFF (1024 bytes). A 6551 está em $D000..=$D003.
; O laço lê um byte (RDRF, bit 3 do status) e escreve o mesmo byte de volta.

.org $FC00

reset:
    lda $D001
    and #$08
    beq reset
    lda $D000
    sta $D000
    jmp reset

.org $FFFC
    .word reset
    .word reset
