//! JAM PC on halt-run, framing recovery, org-into-last-byte, object version.

use mos6502::acia::{programmed_frame, Acia6551, RDRF};
use mos6502::asm::assemble;
use mos6502::bus::{FlatMemory, MemoryBus};
use mos6502::debugger::{Debugger, StopReason};
use mos6502::link::{assemble_object, read_object, write_object};
use mos6502::{Cpu, IoDevice};

#[test]
fn run_until_halt_on_jam_keeps_pc_on_the_jam() {
    let mut mem = FlatMemory::new();
    mem.write(0x8000, 0xA9);
    mem.write(0x8001, 0x11);
    mem.write(0x8002, 0x02);
    mem.write(0x8003, 0xA9);
    mem.write(0x8004, 0x22);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let stop = Debugger::default().run_until_halt(&mut cpu, 30).unwrap();
    assert_eq!(stop.reason, StopReason::Halted);
    assert!(cpu.is_halted());
    assert_eq!(cpu.pc, 0x8002, "PC must stay on the JAM");
    assert_eq!(cpu.a, 0x11, "the byte after JAM must not run");
    assert_eq!(stop.instructions, 2);
}

#[test]
fn good_frame_after_framing_error_clears_the_bit_and_delivers() {
    let mut acia = Acia6551::new();
    acia.set_loopback(true);
    acia.write(2, 0x2B);
    acia.write(3, 0x00);
    let good = programmed_frame(0x5A, 0x2B, 0x00);
    acia.loopback_frame_with_stop(0xA5, good.parity, false);
    assert_eq!(acia.peek(1) & 2, 2);
    assert_eq!(acia.peek(1) & RDRF, 0);

    acia.loopback_frame_with_stop(0x5A, good.parity, true);
    assert_eq!(acia.peek(1) & 2, 0, "framing bit clears on a good frame");
    assert_eq!(acia.peek(1) & RDRF, RDRF);
    assert_eq!(acia.read(0), 0x5A);
}

#[test]
fn org_back_one_into_the_last_emitted_byte_is_rejected() {
    let err = assemble(
        ".org $4000\n.byte $AA\n.byte $BB\n.org $4001\n.byte $CC\n",
        0,
    )
    .unwrap_err();
    assert!(err.to_string().contains("org para trás"), "{err}");
}

#[test]
fn unknown_object_format_version_is_rejected_and_named() {
    let obj = assemble_object("m", ".section code\n nop\n").unwrap();
    let mut bytes = write_object(&obj).unwrap();
    assert_eq!(&bytes[..4], b"M65O");
    bytes[4] = 99;
    let err = read_object(&bytes).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("versão"), "{text}");
    assert!(text.contains("99"), "{text}");
}
