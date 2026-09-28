; Original monitor for the Apple-1 model (written for this project, not WozMon).
; Commands, hex, separated by spaces:  AAAA  (show)  AAAA.BBBB  (show range)
; AAAA: BB BB ...  (store)  AAAA R  (run).  Prompt is "\".
KBD    = $D010
KBDCR  = $D011
DSP    = $D012
DSPCR  = $D013
IN     = $0200

; Zero page: $24/$25 address, $26/$27 range end, $28/$29 number, $2A mode,
; $2B line length, $2C digit count.
        .org $FE00
reset:  cld
        ldy #$7F
        sty DSP          ; DDRB: PB0-6 out, PB7 (busy) in
        lda #$A7
        sta KBDCR
        sta DSPCR
        lda #$DC         ; "\"
        jsr echo
getln:  lda #$8D
        jsr echo
        ldy #0
nextch: lda KBDCR
        bpl nextch
        lda KBD
        jsr echo
        cmp #$8D
        beq parse
        sta IN,y
        iny
        bpl nextch
        bmi getln        ; line too long
eol:    lda $2A
        cmp #3
        bne getln+5      ; back to "ldy #0" (CR already echoed)
        jsr single
        jmp getln+5
run:    jmp ($24)
parse:  sty $2B
        ldx #$FF
        lda #0
setm:   sta $2A
token:  inx
        cpx $2B
        bcs eol
        lda IN,x
        cmp #$A0
        beq token
        cmp #$D2
        beq run
        ldy #1
        cmp #$BA
        beq sety
        iny
        cmp #$AE
        beq sety
        lda #0
        sta $28
        sta $29
        sta $2C
num:    lda IN,x
        eor #$B0
        cmp #$0A
        bcc dig
        adc #$88
        cmp #$FA
        bcc endnum
dig:    asl
        asl
        asl
        asl
        ldy #4
shift:  asl
        rol $28
        rol $29
        dey
        bne shift
        inc $2C
        inx
        cpx $2B
        bcc num
endnum: dex
        lda $2C
        beq token        ; not hex: skip the character
        lda $2A
        cmp #1
        beq store
        cmp #2
        beq range
        cmp #3
        bne newadr
        jsr single
newadr: lda $28
        sta $24
        lda $29
        sta $25
        lda #3
        bne setm
sety:   tya
        bne setm
store:  ldy #0
        lda $28
        sta ($24),y
        inc $24
        bne st2
        inc $25
st2:    jmp token
range:  lda $28
        sta $26
        lda $29
        sta $27
        jsr dump
        lda #0
        jmp setm
single: lda $24
        sta $26
        lda $25
        sta $27
dump:   lda $25
        jsr prbyte
        lda $24
        jsr prbyte
        lda #$BA
        jsr echo
dbyte:  lda #$A0
        jsr echo
        ldy #0
        lda ($24),y
        jsr prbyte
        lda $24
        cmp $26
        lda $25
        sbc $27
        bcs ddone
        inc $24
        bne dnext
        inc $25
dnext:  lda $24
        and #7
        bne dbyte
        lda #$8D
        jsr echo
        jmp dump
ddone:  lda #$8D
        jmp echo
prbyte: pha
        lsr
        lsr
        lsr
        lsr
        jsr prhex
        pla
prhex:  and #$0F
        ora #$B0
        cmp #$BA
        bcc echo
        adc #6
echo:   bit DSP
        bmi echo
        sta DSP
        rts

        .org $FFFA
        .word reset, reset, reset
