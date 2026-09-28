; Original: add two bytes and keep the carry-out. Not a copied monitor.
.section code
.org $8000
.global start
start:
  clc
  lda lo
  adc hi
  sta sum
  lda #$00
  adc #$00
  sta carry
  brk
.section data
.org $0020
.global lo
.global hi
.global sum
.global carry
lo:
  .byte $F0
hi:
  .byte $20
sum:
  .byte $00
carry:
  .byte $00
