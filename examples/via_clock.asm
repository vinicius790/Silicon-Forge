; Interrupt-driven clock on a 6522 VIA mapped at $C000. Runs on the CLI `board`
; command (RAM + 6522 at $C000 + 6551 at $D000) and in tests/pe_examples.rs.
; Timer 1 free-runs with latch 998, so it interrupts every N+2 = 1000 PHI2
; cycles and, with ACR bit 7 set, PB7 outputs a 2000-cycle square wave.
; Every 10 interrupts ("ticks", $00) the seconds counter $01 increments and is
; copied to ORB (PB0..PB6 visible; PB7 belongs to timer 1).
; Layout: $8000 jmp reset, $8003 jmp irq; the host sets the vectors.

ORB  = $C000
DDRB = $C002
T1CL = $C004
T1CH = $C005
ACR  = $C00B
IER  = $C00E

.org $8000
    jmp reset
    jmp irq
reset:
    sei
    ldx #$ff
    txs
    lda #0
    sta $00
    sta $01
    lda #$ff
    sta DDRB
    lda #$c0        ; T1 free-run, PB7 output
    sta ACR
    lda #$e6        ; 998 = $03E6
    sta T1CL
    lda #$03
    sta T1CH        ; starts the timer
    lda #$c0        ; IER: set bit 6 (timer 1)
    sta IER
    cli
idle:
    jmp idle
irq:
    pha
    lda T1CL        ; reading T1C-L clears IFR bit 6
    inc $00
    lda $00
    cmp #$0a        ; 10 ticks
    bne out
    lda #0
    sta $00
    inc $01
    lda $01
    sta ORB
out:
    pla
    rti
