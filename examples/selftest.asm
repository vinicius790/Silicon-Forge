; Self-test in the project's own assembler (Klaus Dormann's idea, original code).
; Each test stores its number in $0200 before it runs. A failure branches to that
; test's trap, a `jmp` to itself, with the number still in $0200. Success clears $0200
; and loops at `done`. NMOS 6502: the LAX check needs the unofficial opcode.
TEST = $0200
ptr  = $10
tbl  = $0300

.org $8000
reset:
    cld
    ldx #$FF
    txs

    lda #1
    sta TEST
    lda #$00
    bne trap1
    bmi trap1
    lda #$80
    bpl trap1
    beq trap1

    jmp t1_ok
trap1:
    jmp trap1
t1_ok:
    lda #2
    sta TEST
    clc
    lda #$7F
    adc #$01
    bvc trap2
    bcs trap2
    bpl trap2
    cmp #$80
    bne trap2

    jmp t2_ok
trap2:
    jmp trap2
t2_ok:
    lda #3
    sta TEST
    clc
    lda #$FF
    adc #$01
    bcc trap3
    bne trap3

    jmp t3_ok
trap3:
    jmp trap3
t3_ok:
    lda #4
    sta TEST
    sec
    lda #$00
    sbc #$01
    bcs trap4
    bvs trap4
    cmp #$FF
    bne trap4
    sec
    lda #$80
    sbc #$01
    bvc trap4
    cmp #$7F
    bne trap4

    jmp t4_ok
trap4:
    jmp trap4
t4_ok:
    lda #5
    sta TEST
    sed
    clc
    lda #$19
    adc #$01
    cmp #$20
    bne fail_d
    sec
    lda #$20
    sbc #$01
    cmp #$19
    bne fail_d
    clc
    lda #$99
    adc #$01
    bcc fail_d
    cld
    jmp t5_end
fail_d:
    cld
    jmp trap5
t5_end:
    jmp t5_ok
trap5:
    jmp trap5
t5_ok:
    lda #6
    sta TEST
    lda #$81
    asl a
    bcc trap6
    cmp #$02
    bne trap6
    lda #$01
    lsr a
    bcc trap6
    bne trap6
    sec
    lda #$00
    ror a
    bcs trap6
    cmp #$80
    bne trap6

    jmp t6_ok
trap6:
    jmp trap6
t6_ok:
    lda #7
    sta TEST
    lda #$42
    pha
    lda #$00
    pla
    cmp #$42
    bne trap7
    sec
    php
    clc
    plp
    bcc trap7
    jmp t7_ok
trap7:
    jmp trap7
t7_ok:
    lda #8
    sta TEST
    lda #$00
    jsr setA
    cmp #$33
    bne trap8

    jmp t8_ok
trap8:
    jmp trap8
t8_ok:
    lda #9
    sta TEST
    lda #$11
    sta tbl
    lda #$22
    sta tbl+1
    lda #$33
    sta tbl+2
    ldx #$02
    lda tbl,x
    cmp #$33
    bne trap9
    lda #<tbl
    sta ptr
    lda #>tbl
    sta ptr+1
    ldy #$01
    lda (ptr),y
    cmp #$22
    bne trap9
    ldx #$00
    lda (ptr,x)
    cmp #$11
    bne trap9

    jmp t9_ok
trap9:
    jmp trap9
t9_ok:
    lda #10
    sta TEST
    lda #$40
    cmp #$40
    bne trap10
    bcc trap10
    cmp #$41
    bcs trap10
    bpl trap10

    jmp t10_ok
trap10:
    jmp trap10
t10_ok:
    lda #11
    sta TEST
    lda #$C0
    sta $20
    lda #$01
    bit $20
    bpl trap11
    bvc trap11
    bne trap11

    jmp t11_ok
trap11:
    jmp trap11
t11_ok:
    lda #12
    sta TEST
    lda #$FF
    sta $21
    inc $21
    bne trap12
    dec $21
    bpl trap12

    jmp t12_ok
trap12:
    jmp trap12
t12_ok:
    lda #13
    sta TEST
    lda #$5A
    sta $22
    lda #$00
    ldx #$00
    .byte $A7, $22       ; LAX $22
    cmp #$5A
    bne trap13
    cpx #$5A
    bne trap13

    jmp t13_ok
trap13:
    jmp trap13
t13_ok:
    lda #14
    sta TEST
    lda #<brk_handler
    sta $FFFE
    lda #>brk_handler
    sta $FFFF
    brk
    .byte $EA            ; signature byte, skipped by RTI
    lda $23
    cmp #$01
    bne trap14

    jmp t14_ok
trap14:
    jmp trap14
t14_ok:
    lda #$00
    sta TEST
done:
    jmp done

setA:
    lda #$33
    rts

trap_brk:
    jmp trap_brk

brk_handler:
    tsx
    lda $0101,x          ; status pushed by BRK
    and #$10
    beq trap_brk
    inc $23
    rti

.org $FFFC
    .word reset
    .word $0000
