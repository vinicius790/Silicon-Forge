; Interrupt-driven echo on a 6551 ACIA at $D000. Runs on the CLI `board` command
; (RAM + 6522 at $C000 + 6551 at $D000) and in tests/pe_examples.rs.
; Command $09: DTR on, receiver IRQ enabled (bit 1 clear), RTS low with the
; transmit IRQ off. Control $10: 8N1, baud nibble 0.
; The IRQ handler reads status (clearing the IRQ), and when RDRF is set takes
; the byte, appends it to the buffer at $0200 (count in $10) and echoes it.
; Layout: $8000 jmp reset, $8003 jmp irq.

DATA    = $D000
STATUS  = $D001
COMMAND = $D002
CONTROL = $D003

.org $8000
    jmp reset
    jmp irq
reset:
    sei
    ldx #$ff
    txs
    lda #0
    sta $10
    sta STATUS      ; programmed reset
    lda #$10
    sta CONTROL
    lda #$09
    sta COMMAND
    cli
idle:
    jmp idle
irq:
    pha
    txa
    pha
    lda STATUS
    and #$08        ; RDRF
    beq done
    lda DATA
    ldx $10
    sta $0200,x
    inc $10
    sta DATA        ; echo
done:
    pla
    tax
    pla
    rti
