; Sieve of Eratosthenes over 0..255 with one flag byte per number at $0200.
; After the sieve $0200+n is 1 when n is prime. $00 receives the number of
; primes below 256, which is 54 ($36). Ends on BRK.

FLAGS = $0200
STEP  = $10

.org $8000
    ldx #0
    lda #1
init:
    sta FLAGS,x
    inx
    bne init
    lda #0
    sta FLAGS       ; 0 and 1 are not prime
    sta FLAGS+1
    ldx #2
outer:
    lda FLAGS,x
    beq nexto       ; already crossed out
    stx STEP
    txa
mark:
    clc
    adc STEP        ; next multiple
    bcs nexto       ; ran past 255
    tay
    lda #0
    sta FLAGS,y
    tya
    jmp mark
nexto:
    inx
    cpx #$10        ; every composite below 256 has a factor below 16
    bne outer
    ldx #0
    ldy #0
count:
    lda FLAGS,x
    beq skip
    iny
skip:
    inx
    bne count
    sty $00
    brk
