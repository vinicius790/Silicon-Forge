; ==============================================================================
; MOS 6502 Sieve of Eratosthenes (Prime Numbers Generator)
; Finds prime numbers between 2 and 30 and marks non-primes in Zero Page.
; ==============================================================================

.org $8000

    ; Step 1: Initialize table at $00..$1E with 1 (prime candidate)
    LDX #$02
init_table:
    LDA #$01
    STA $00,X
    INX
    CPX #$1F
    BNE init_table

    ; Step 2: Sieve multiples
    LDX #$02
sieve_outer:
    LDA $00,X
    BEQ next_outer  ; If marked 0, already known composite

    ; Mark multiples: X + X, X + 2X, ... The step lives at $40, outside the table.
    STX $40
    TXA
    TAY
mark_inner:
    TYA
    CLC
    ADC $40         ; Add step (X)
    TAY
    CPY #$1F
    BCS next_outer
    LDA #$00
    STA $0000,Y     ; Mark composite
    JMP mark_inner

next_outer:
    INX
    CPX #$06        ; Check factors up to sqrt(30) ~ 5.4
    BNE sieve_outer

    BRK             ; Zero Page now contains primes marked with $01
