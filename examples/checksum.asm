; Soma três bytes. Programa original deste crate. Não é ROM de monitor.
.section code
.global start
start:
  clc
  lda n0
  adc n1
  adc n2
  sta total
  brk
.section data
.global n0
.global n1
.global n2
.global total
n0:
  .byte $0A
n1:
  .byte $14
n2:
  .byte $1E
total:
  .byte $00
