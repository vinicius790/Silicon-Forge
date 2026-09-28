; ==============================================================================
; Apple-1 PIA 6820 Terminal Output Demo
; Demonstrates interacting with the Apple-1 hardware interface.
; ==============================================================================

KBD   = $D010       ; Keyboard data
KBDCR = $D011       ; Keyboard control
DSP   = $D012       ; Display output
DSPCR = $D013       ; Display control

.org $0200

start:
    LDX #$00
print_str:
    LDA message,X
    BEQ done

wait_dsp:
    BIT DSP         ; Bit 7 indicates if display is ready
    BMI wait_dsp    ; Wait until bit 7 is clear

    ORA #$80        ; Apple-1 uses high-bit set ASCII
    STA DSP         ; Send character to terminal screen
    INX
    BNE print_str

done:
    BRK

message:
    .asciiz "HELLO FROM APPLE-1 COMPUTER (1976)!"
