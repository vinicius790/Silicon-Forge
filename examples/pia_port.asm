; PIA 6821 at $C000: DDR A then ORA = $01
.org $8000
  lda #$ff
  sta $c000
  lda #$04
  sta $c001
  lda #$01
  sta $c000
  brk
