; ==============================================================================
; MOS 6502 Demoscene Color Plasma / Palette Cycle Effect
; Fills the 32x32 retro display with animated diagonal color patterns.
; ==============================================================================

.org $0600

start:
    LDX #$00        ; Shift de fase
frame_loop:
    LDY #$00        ; Índice de pixel (0..1023)
draw_loop:
    TYA
    CLC
    ADC $00         ; Adiciona offset de tempo
    AND #$0F        ; Cicla pelas 16 cores da paleta
    STA $0200,Y
    STA $0300,Y
    STA $0400,Y
    STA $0500,Y

    INY
    BNE draw_loop

    INC $00         ; Avança tempo
    INX
    BNE frame_loop

    BRK
