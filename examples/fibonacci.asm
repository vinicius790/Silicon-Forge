; ==============================================================================
; MOS 6502 Fibonacci Sequence Generator
; Computes the first 12 numbers in the Fibonacci sequence and stores them
; sequentially in the Zero Page at $00..$0B.
; ==============================================================================

.org $8000

    LDA #$00        ; F(0) = 0
    STA $00
    LDA #$01        ; F(1) = 1
    STA $01

    LDX #$02        ; Index = 2
fib_loop:
    TXA
    SEC
    SBC #$01
    TAY
    LDA $00,Y       ; A = F(n-1)

    DEY
    CLC
    ADC $00,Y       ; A = F(n-1) + F(n-2)
    STA $00,X       ; Store F(n)

    INX
    CPX #$0C        ; Up to 12 numbers (0, 1, 1, 2, 3, 5, 8, 13, 21, 34, 55, 89)
    BNE fib_loop

    BRK             ; Stop execution (halt)
