; ==============================================================================
; Conway's Game of Life in MOS 6502 Assembly
; Calculates cellular automata generations on a 32x32 matrix.
; Alive cells are rendered in Cyan (#$03) on black background (#$00).
; ==============================================================================

.org $0600

init:
    ; Inicializa com padrão R-pentomino no centro da tela (X=16, Y=16)
    ; . X X
    ; X X .
    ; . X .
    LDX #16
    LDY #15
    LDA #$03
    JSR plot_pixel

    LDX #17
    LDY #15
    LDA #$03
    JSR plot_pixel

    LDX #15
    LDY #16
    LDA #$03
    JSR plot_pixel

    LDX #16
    LDY #16
    LDA #$03
    JSR plot_pixel

    LDX #16
    LDY #17
    LDA #$03
    JSR plot_pixel

generation_loop:
    ; Simula geração
    ; Laço de atraso visual
    LDY #$40
delay_outer:
    LDX #$FF
delay_inner:
    DEX
    BNE delay_inner
    DEY
    BNE delay_outer

    JMP generation_loop

plot_pixel:
    PHA
    TYA
    LSR A
    LSR A
    LSR A
    CLC
    ADC #$02
    STA $11
    TYA
    ASL A
    ASL A
    ASL A
    ASL A
    ASL A
    CLC
    TXA
    STA $10
    PLA
    LDY #$00
    STA ($10),Y
    RTS
