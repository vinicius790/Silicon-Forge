//! Ben Eater board: the shared 6522 drives an HD44780 through its port pins.

use mos6502::assemble;
use mos6502::bus::IoDevice;
use mos6502::systems::beneater::{Board, Lcd};
use mos6502::BenEaterComputer;

#[test]
fn lcd_command_set_over_every_command_byte() {
    for cmd in 0..=255u8 {
        let mut lcd = Lcd::default();
        lcd.data(b'X');
        lcd.command(cmd);
        match cmd {
            0x80..=0xFF => assert_eq!(lcd.address, cmd & 0x7F),
            0x04..=0x07 => assert_eq!((lcd.address, lcd.increment), (1, cmd & 2 != 0)),
            0x02 | 0x03 => assert_eq!((lcd.address, lcd.ddram[0]), (0, b'X')),
            0x01 => assert_eq!((lcd.address, lcd.ddram[0]), (0, b' ')),
            _ => assert_eq!(lcd.address, 1, "{cmd:02X} ignored"),
        }
    }
    let mut lcd = Lcd::default();
    lcd.command(0xC0);
    lcd.data(b'B');
    lcd.command(0x04); // decrement
    lcd.command(0x81);
    lcd.data(b'2');
    lcd.data(b'1');
    assert_eq!(
        lcd.lines(),
        [
            "12              ".to_string(),
            "B               ".to_string()
        ]
    );
    lcd.command(0x80);
    lcd.data(b'w');
    assert_eq!(lcd.address, 0x7F, "decrement wraps in 7 bits");
}

#[test]
fn writes_latch_only_on_e_falling_with_rw_low_and_outputs_enabled() {
    let mut board = Board::default();
    board.write(0x2, 0xFF); // DDRB out
    board.write(0x3, 0xE0); // E, RW, RS out
    let strobe = |board: &mut Board, rs_rw: u8, data: u8| {
        board.write(0x0, data);
        board.write(0x1, rs_rw | 0x80);
        board.write(0x1, rs_rw);
    };
    strobe(&mut board, 0x20, b'H');
    strobe(&mut board, 0x60, b'Q'); // R/W high: not a write
    strobe(&mut board, 0x20, b'i');
    assert_eq!(&board.lcd.lines()[0][..3], "Hi ");
    board.write(0x3, 0x00); // port A to inputs: pins float high, no edges
    board.write(0x0, b'Z');
    board.write(0x1, 0x00);
    assert_eq!(&board.lcd.lines()[0][..3], "Hi ");
}

#[test]
fn a_program_prints_on_the_lcd() {
    let src = "
        .org $8000
        lda #$ff
        sta $6002
        lda #$e0
        sta $6003
        ldx #0
loop:   lda msg,x
        beq done
        sta $6000
        lda #$a0
        sta $6001
        lda #$20
        sta $6001
        inx
        jmp loop
done:   jmp done
msg:    .byte \"OK\",0
    ";
    let rom = assemble(src, 0x8000).unwrap();
    let mut pc = BenEaterComputer::new(&rom);
    pc.cpu.run_instructions(200).unwrap();
    let rendered = pc.board().render_lcd();
    assert!(rendered.contains("|OK              |"), "{rendered}");
}
