//! Generic SBC boards from text: parsing, validation, memory map, devices, and the
//! Apple I-compatible preset against the dedicated Apple-1 machine.

use mos6502::bus::{IoDevice, MemoryBus};
use mos6502::systems::apple1::Apple1;
use mos6502::systems::sbc::{BoardConfig, DeviceKind, Handle, Sbc, APPLE1_CONFIG};
use mos6502::CpuVariant;

fn err_line(text: &str) -> (usize, String) {
    let e = BoardConfig::parse(text).unwrap_err();
    (e.line, e.message)
}

#[test]
fn apple1_preset_parses_to_the_documented_layout() {
    let c = BoardConfig::parse(APPLE1_CONFIG).unwrap();
    assert_eq!(c.name, "Apple I compatible");
    assert_eq!(c.cpu, CpuVariant::NMOS);
    assert_eq!(c.ram, [(0x0000, 0x0FFF)]);
    assert_eq!(c.devices.len(), 1);
    assert_eq!(
        (c.devices[0].kind, c.devices[0].start, c.devices[0].end),
        (DeviceKind::Apple1Pia, 0xD010, 0xD013)
    );
    assert_eq!((c.roms[0].start, c.roms[0].bytes.len()), (0xFE00, 512));
    let map: Vec<_> = c
        .memory_map()
        .into_iter()
        .map(|m| (m.start, m.end, m.what))
        .collect();
    assert_eq!(
        map,
        [
            (0x0000, 0x0FFF, "ram".to_string()),
            (0x1000, 0xD00F, "open".into()),
            (0xD010, 0xD013, "kbd".into()),
            (0xD014, 0xFDFF, "open".into()),
            (0xFE00, 0xFFFF, "rom".into()),
        ]
    );
}

#[test]
fn apple1_preset_runs_the_monitor_like_the_apple1_machine() {
    let mut sbc = Sbc::from_text(APPLE1_CONFIG).unwrap();
    let mut a1 = Apple1::new();
    let Some(Handle::Apple1(kbd)) = sbc.device("kbd").cloned() else {
        panic!("kbd device");
    };
    for line in ["FF00.FF0F", "0300: A9 42 85 10", "0300.0303", "FFFC.FFFD"] {
        let want = a1.enter(line, 200_000).unwrap();
        kbd.borrow_mut().queue_input(line);
        kbd.borrow_mut().queue_input("\n");
        let mut budget = 0;
        while !kbd.borrow().input_buffer.is_empty()
            || IoDevice::peek(&kbd.borrow().pia, 1) & 0x80 != 0
        {
            sbc.run_cycles(1000).unwrap();
            budget += 1;
            assert!(budget < 2000);
        }
        sbc.run_cycles(200_000).unwrap();
        let got: String = std::mem::take(&mut kbd.borrow_mut().output_buffer)
            .iter()
            .map(|&b| if b == 0x0D { '\n' } else { b as char })
            .collect();
        assert_eq!(got, want, "{line}");
    }
    // Writes beyond the 4 KiB fall on open bus here.
    sbc.cpu.bus.write(0x2000, 0x12);
    assert_eq!(sbc.cpu.bus.read(0x2000), 0xFF);
    assert_eq!(sbc.cpu.bus.read(0xFFFC), a1.cpu.bus.read(0xFFFC));
}

#[test]
fn value_syntax() {
    let c = BoardConfig::parse(
        "name = \"x\" # comment\ncpu = \"65C02\"\n[[rom]]\nstart = $F000\nbytes = [0xA9, 1, $FF]\n[[rom]]\nstart = 61440\nfill=0xEA\nsize = 4\n",
    );
    // 61440 = $F000: overlap with the first ROM.
    let e = c.unwrap_err();
    assert_eq!(e.line, 6);
    assert!(e.message.contains("overlaps"), "{}", e.message);
    let c = BoardConfig::parse("cpu = \"65c02\"\n[[rom]]\nstart = $F000\nbytes = [0xA9, 1, $FF]\n[[rom]]\nstart = 0xF003\nfill = 0xEA\nsize = 4\n").unwrap();
    assert_eq!(c.cpu, CpuVariant::CMOS);
    assert_eq!(c.roms[0].bytes, [0xA9, 1, 0xFF]);
    assert_eq!(c.roms[1].bytes, [0xEA; 4]);
    assert_eq!(c.name, "SBC");
}

#[test]
fn every_error_names_its_line() {
    let cases: &[(&str, usize, &str)] = &[
        ("cpu = \"z80\"", 1, "unknown cpu"),
        ("\n\nfoo = 1", 3, "unknown key"),
        ("[[ram]]\nstart = 0\nend = 0x10000", 3, "beyond"),
        ("[[ram]]\nstart = 5\nend = 4", 1, "above end"),
        ("[[ram]]\nend = 4", 1, "needs start"),
        ("[[ram]]\nstart = 0\nstart = 1", 3, "duplicate key"),
        ("[[disk]]", 1, "unknown table"),
        ("[[ram]\n", 1, "bad table header"),
        ("x", 1, "expected key"),
        ("name = \"abc", 1, "unterminated string"),
        ("[[rom]]\nstart = 0\nbytes = [1, 256]", 3, "not a byte"),
        ("[[rom]]\nstart = 0\nbytes = [1", 3, "unterminated list"),
        ("[[rom]]\nstart = 0xFFFF\nbytes = [1, 2]", 1, "past $FFFF"),
        ("[[rom]]\nstart = 0\nbytes = []", 1, "empty ROM"),
        ("[[rom]]\nstart = 0", 1, "needs one of"),
        ("[[rom]]\nstart = 0\nimage = \"wozmon\"", 3, "unknown image"),
        ("[[rom]]\nstart = 0\nfill = 300\nsize = 1", 3, "fill must be a byte"),
        ("[[device]]\nstart = 0\nend = 3", 1, "needs kind"),
        ("[[device]]\nkind = \"tia\"\nstart = 0\nend = 3", 2, "unknown device kind"),
        ("[[device]]\nkind = \"via6522\"\nstart = 0\nend = 3", 1, "at least 16"),
        ("[[device]]\nkind = \"riot6532\"\nstart = 0\nend = 0x7F", 1, "at least 256"),
        ("[[device]]\nkind = \"pia6821\"\nstart = 0\nend = 3\nirq = 1", 5, "irq must be"),
        (
            "[[device]]\nkind = \"pia6821\"\nname = \"a\"\nstart = 0\nend = 3\n[[device]]\nkind = \"pia6821\"\nname = \"a\"\nstart = 4\nend = 7",
            6,
            "duplicate device name",
        ),
        ("[[ram]]\nstart = 0\nend = 0x0FFF\n[[device]]\nkind = \"pia6821\"\nstart = 0x0FFC\nend = 0x0FFF", 4, "overlaps"),
        ("bad key = 1", 1, "bad key"),
        ("x = 0xZZ", 1, "bad value"),
    ];
    for &(text, line, msg) in cases {
        let (l, m) = err_line(text);
        assert_eq!(l, line, "{text:?}: {m}");
        assert!(m.contains(msg), "{text:?}: {m}");
    }
}

const VIA_BOARD: &str = r#"
name = "VIA timer board"
[[ram]]
start = 0
end = 0x7FFF
[[device]]
kind = "via6522"
name = "via"
start = 0x8000
end = 0x800F
irq = true
[[device]]
kind = "riot6532"
name = "riot"
start = 0x8100
end = 0x81FF
[[device]]
kind = "acia6551"
name = "acia"
start = 0x8200
end = 0x8203
[[device]]
kind = "pia6821"
name = "pia"
start = 0x8300
end = 0x8303
[[rom]]
start = 0xFF00
# IRQ handler at $FF00: INC $10 ; LDA $8004 (clears T1) ; RTI
# reset at $FF10: CLI ; LDA #$C0 ; STA $800E (enable T1) ; LDA #$40 ; STA $800B (free run)
#                 LDA #$FF ; STA $8004 ; LDA #$00 ; STA $8005 ; JMP *
bytes = [0xE6, 0x10, 0xAD, 0x04, 0x80, 0x40, 0,0,0,0,0,0,0,0,0,0, 0x58, 0xA9, 0xC0, 0x8D, 0x0E, 0x80, 0xA9, 0x40, 0x8D, 0x0B, 0x80, 0xA9, 0xFF, 0x8D, 0x04, 0x80, 0xA9, 0x00, 0x8D, 0x05, 0x80, 0x4C, 0x25, 0xFF]
[[rom]]
start = 0xFFFA
bytes = [0x00, 0xFF, 0x10, 0xFF, 0x00, 0xFF]
"#;

#[test]
fn devices_tick_and_wired_irq_reaches_the_cpu() {
    let mut sbc = Sbc::from_text(VIA_BOARD).unwrap();
    assert_eq!(sbc.cpu.pc, 0xFF10);
    sbc.run_cycles(50_000).unwrap();
    // T1 free-run period is $00FF + 2 cycles.
    let n = sbc.cpu.bus.read(0x10);
    let expect = 50_000 / 257;
    assert!((i64::from(n) - expect as i64).abs() <= 2, "{n} vs {expect}");
    // Devices reachable through their handles and the bus alike.
    let Some(Handle::Riot(riot)) = sbc.device("riot").cloned() else {
        panic!()
    };
    sbc.cpu.bus.write(0x8105, 0x77);
    assert_eq!(riot.borrow().ram[5], 0x77);
    assert!(matches!(sbc.device("acia"), Some(Handle::Acia(_))));
    assert!(matches!(sbc.device("pia"), Some(Handle::Pia(_))));
    assert!(sbc.device("none").is_none());
    // Every byte between devices is open.
    for a in [0x8010u16, 0x80FF, 0x8204, 0x8304, 0xFE00] {
        assert_eq!(sbc.cpu.bus.read(a), 0xFF, "{a:04X}");
    }
}

#[test]
fn unwired_device_does_not_interrupt() {
    let text = VIA_BOARD.replace("irq = true", "irq = false");
    let mut sbc = Sbc::from_text(&text).unwrap();
    sbc.run_cycles(20_000).unwrap();
    assert_eq!(sbc.cpu.bus.read(0x10), 0);
    assert!(!sbc.irq());
}

#[test]
fn memory_map_covers_every_address_once_for_random_layouts() {
    let mut seed = 0x1234_5678u32;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for _ in 0..300 {
        let mut text = String::new();
        let mut at = 0u32;
        let mut k = 0;
        while at < 0xF000 {
            at += rnd() % 0x800;
            let size = 16 + rnd() % 0x400;
            if at + size > 0xFFFF {
                break;
            }
            match rnd() % 3 {
                0 => text += &format!("[[ram]]\nstart = {at}\nend = {}\n", at + size - 1),
                1 => text += &format!("[[rom]]\nstart = {at}\nfill = 0xEA\nsize = {size}\n"),
                _ => {
                    text += &format!(
                        "[[device]]\nkind = \"via6522\"\nname = \"v{k}\"\nstart = {at}\nend = {}\n",
                        at + size - 1
                    )
                }
            }
            k += 1;
            at += size;
        }
        let c = BoardConfig::parse(&text).unwrap();
        let map = c.memory_map();
        assert_eq!(map[0].start, 0);
        assert_eq!(map.last().unwrap().end, 0xFFFF);
        for w in map.windows(2) {
            assert_eq!(u32::from(w[0].end) + 1, u32::from(w[1].start));
        }
        let sbc = c.build().unwrap();
        for m in &map {
            if m.what == "open" {
                assert_eq!(sbc.cpu.bus.peek(m.start), 0xFF);
                assert_eq!(sbc.cpu.bus.peek(m.end), 0xFF);
            } else if m.what == "rom" {
                assert_eq!(sbc.cpu.bus.peek(m.start), 0xEA);
            }
        }
    }
}
