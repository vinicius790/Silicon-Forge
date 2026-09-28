//! Scripted TUI sessions and command parsing.

use mos6502::tui::{TuiCommand, TuiDashboard};
use mos6502::{Cpu, FlatMemory};

fn cpu() -> Cpu<FlatMemory> {
    let mut mem = FlatMemory::new();
    mem.load_at(0x0400, &[0xA9, 0x41, 0xE8, 0xE8, 0x02]); // LDA #$41, INX, INX, JAM
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x0400;
    cpu
}

#[test]
fn parse_every_command_and_reject_the_rest() {
    assert_eq!(TuiCommand::parse(""), Ok(TuiCommand::Step));
    assert_eq!(TuiCommand::parse(" S "), Ok(TuiCommand::Step));
    assert_eq!(TuiCommand::parse("c"), Ok(TuiCommand::Continue(100)));
    assert_eq!(TuiCommand::parse("c 7"), Ok(TuiCommand::Continue(7)));
    assert_eq!(TuiCommand::parse("m $C0de"), Ok(TuiCommand::Mem(0xC0DE)));
    assert_eq!(TuiCommand::parse("m 0x10"), Ok(TuiCommand::Mem(0x10)));
    for bad in ["c 0", "c x", "m 10000", "m", "s 1", "zz", "q now", "m 1 2"] {
        assert!(TuiCommand::parse(bad).is_err(), "{bad}");
    }
    for addr in (0..=0xFFFFu32).step_by(257) {
        assert_eq!(
            TuiCommand::parse(&format!("m {addr:x}")),
            Ok(TuiCommand::Mem(addr as u16))
        );
    }
}

#[test]
fn a_session_steps_shows_memory_and_reports_errors() {
    let mut cpu = cpu();
    let mut tui = TuiDashboard::new();
    let mut out = Vec::new();
    tui.run_with(&mut cpu, &b"s\nm 0400\nbogus\nc 10\nq\n"[..], &mut out)
        .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(cpu.a, 0x41);
    assert_eq!(cpu.x, 2, "c stops at the jam");
    assert!(text.contains("$0400: A9 41 E8 E8 02"), "dump row");
    assert!(
        text.contains("|.A...|") || text.contains("|.A..."),
        "ascii column"
    );
    assert!(text.contains("comando desconhecido: bogus"));
    assert_eq!(text.matches("(TUI)>").count(), 5);
    assert!(text.ends_with("Sessão TUI encerrada.\n"));
}

#[test]
fn stack_window_follows_sp_for_every_value() {
    let tui = TuiDashboard::new();
    for sp in 0..=255u8 {
        let mut cpu = cpu();
        cpu.sp = sp;
        let frame = tui.frame(&cpu);
        let first = sp.saturating_sub(8).min(0xF0);
        assert!(
            frame.contains(&format!("PILHA ($01{first:02X}..=$01{:02X}", first + 15)),
            "{sp}"
        );
        assert_eq!(
            frame.matches("\x1b[1;33m[").count(),
            1,
            "SP highlighted once ({sp})"
        );
    }
}
