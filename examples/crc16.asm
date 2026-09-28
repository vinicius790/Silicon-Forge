; CRC-16/CCITT-FALSE (poly $1021, init $FFFF, no reflection, no final XOR)
; over the ASCII string "123456789". The standard check value is $29B1.
; Result: $00 = CRC low byte, $01 = CRC high byte. Ends on BRK.

CRCLO = $00
CRCHI = $01

.org $8000
    lda #$ff
    sta CRCLO
    sta CRCHI
    ldy #0
next:
    lda msg,y
    beq done        ; zero terminator
    eor CRCHI       ; feed the byte into the top of the register
    sta CRCHI
    ldx #$08
bit:
    asl CRCLO       ; shift the 16-bit register left one bit
    rol CRCHI
    bcc nox         ; bit 15 was clear: no polynomial
    lda CRCHI
    eor #$10
    sta CRCHI
    lda CRCLO
    eor #$21
    sta CRCLO
nox:
    dex
    bne bit
    iny
    bne next
done:
    brk
msg:
    .byte "123456789", 0
