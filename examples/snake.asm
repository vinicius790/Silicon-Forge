; ==============================================================================
; MOS 6502 Retro Snake Game
;
; Rendered on the 32x32 Fantasy Console Framebuffer ($0200..$05FF).
; Controls: W (Up), A (Left), S (Down), D (Right) mapped at $00FF.
; Random apple generator mapped at $00FE.
; ==============================================================================

KEYBOARD = $00FF
RANDOM   = $00FE

SNAKE_HEAD_X = $00
SNAKE_HEAD_Y = $01
APPLE_X      = $02
APPLE_Y      = $03
DIRECTION    = $04      ; 1: Up, 2: Right, 3: Down, 4: Left
LENGTH       = $05

.org $0600

init:
    ; Posição inicial da cabeça (16, 16)
    LDA #16
    STA SNAKE_HEAD_X
    STA SNAKE_HEAD_Y

    ; Direção inicial = Direita (2)
    LDA #$02
    STA DIRECTION

    ; Tamanho inicial
    LDA #$03
    STA LENGTH

    ; Gera primeira maçã
    JSR spawn_apple

game_loop:
    ; Lê teclado
    LDA KEYBOARD
    BEQ no_input
    CMP #'w'
    BNE check_s
    LDA #$01
    STA DIRECTION
    JMP no_input
check_s:
    CMP #'s'
    BNE check_a
    LDA #$03
    STA DIRECTION
    JMP no_input
check_a:
    CMP #'a'
    BNE check_d
    LDA #$04
    STA DIRECTION
    JMP no_input
check_d:
    CMP #'d'
    BNE no_input
    LDA #$02
    STA DIRECTION

no_input:
    ; Move cabeça
    LDA DIRECTION
    CMP #$01
    BNE not_up
    DEC SNAKE_HEAD_Y
    JMP check_bounds
not_up:
    CMP #$03
    BNE not_down
    INC SNAKE_HEAD_Y
    JMP check_bounds
not_down:
    CMP #$04
    BNE move_right
    DEC SNAKE_HEAD_X
    JMP check_bounds
move_right:
    INC SNAKE_HEAD_X

check_bounds:
    ; Limites de tela (0..31)
    LDA SNAKE_HEAD_X
    AND #$1F
    STA SNAKE_HEAD_X

    LDA SNAKE_HEAD_Y
    AND #$1F
    STA SNAKE_HEAD_Y

    ; Desenha cabeça no framebuffer ($0200 + Y*32 + X)
    ; Cor da cobra = 5 (Verde)
    LDX SNAKE_HEAD_X
    LDY SNAKE_HEAD_Y
    LDA #$05
    JSR draw_pixel

    ; Testa se comeu maçã
    LDA SNAKE_HEAD_X
    CMP APPLE_X
    BNE draw_apple
    LDA SNAKE_HEAD_Y
    CMP APPLE_Y
    BNE draw_apple

    ; Comeu! Aumenta tamanho e cria nova maçã
    INC LENGTH
    JSR spawn_apple

draw_apple:
    ; Desenha maçã: Cor = 2 (Vermelho)
    LDX APPLE_X
    LDY APPLE_Y
    LDA #$02
    JSR draw_pixel

    ; Atraso
    LDX #$FF
delay:
    DEX
    BNE delay

    JMP game_loop

spawn_apple:
    LDA RANDOM
    AND #$1F
    STA APPLE_X
    LDA RANDOM
    AND #$1F
    STA APPLE_Y
    RTS

; Sub-rotina: Desenha pixel (X, Y) com cor A
draw_pixel:
    PHA
    TYA
    LSR A
    LSR A
    LSR A
    CLC
    ADC #$02        ; High byte do endereço ($02..$05)
    STA $11

    TYA
    ASL A
    ASL A
    ASL A
    ASL A
    ASL A
    CLC
    TXA
    STA $10         ; Low byte

    PLA
    LDY #$00
    STA ($10),Y
    RTS
