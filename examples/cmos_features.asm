; ==============================================================================
; CMOS 65C02 Instruction Extensions Demo
; Demonstrates BRA, STZ, PHX, PHY, PLX, PLY.
; ==============================================================================

.org $8000

    ; Store zero directly without modifying accumulator (STZ)
    STZ $00
    STZ $0200

    ; Push X and Y directly onto stack (PHX, PHY)
    LDX #$42
    LDY #$84
    PHX
    PHY

    ; Branch Always (BRA)
    BRA skip_dead_code

    ; Dead code (should be jumped over)
    LDA #$FF
    STA $00

skip_dead_code:
    ; Pull directly into different registers (PLX, PLY)
    PLX             ; X gets former Y ($84)
    PLY             ; Y gets former X ($42)

    BRK
