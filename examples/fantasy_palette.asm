.org $8000

    LDA #$00
    STA $00
    LDA #$02
    STA $01
    LDA #$00
    STA $02
    LDY #$00

fill_screen:
    LDA $02
    AND #$0F
    STA ($00),Y
    INC $02
    INY
    BNE fill_screen

    INC $01
    LDA $01
    CMP #$06
    BNE fill_screen
    BRK
