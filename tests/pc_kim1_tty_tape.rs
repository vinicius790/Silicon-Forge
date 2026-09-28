//! KIM-1 TTY routines (bit-level serial on PB0/PA7), PB7 timer IRQ jumpers and the
//! documented audio tape format.

use mos6502::bus::MemoryBus;
use mos6502::systems::kim1::{entry, map, tape, Kim1, MONITOR_SRC};

fn kim() -> Kim1 {
    let mut k = Kim1::new();
    k.run_instructions(200).unwrap();
    k
}

/// Run a program at $0200 ending in `JMP *` (spin at `spin`).
fn run(k: &mut Kim1, prog: &[u8], spin: u16, max: u64) {
    k.cpu.bus.ram[0x200..0x200 + prog.len()].copy_from_slice(prog);
    k.cpu.pc = 0x200;
    let mut n = 0;
    while k.cpu.pc != spin {
        k.run_instructions(1).unwrap();
        n += 1;
        assert!(n < max, "program did not finish");
    }
}

#[test]
fn tty_entry_points_are_documented_addresses() {
    let out = mos6502::asm::assemble_with_symbols(MONITOR_SRC, 0x1C00).unwrap();
    for (n, a) in [
        ("crlf", entry::CRLF),
        ("prtbyt", entry::PRTBYT),
        ("getch", entry::GETCH),
        ("outsp", entry::OUTSP),
        ("outch", entry::OUTCH),
    ] {
        assert_eq!(out.symbols[n], a, "{n}");
    }
}

#[test]
fn outch_sends_every_byte_as_an_8n2_frame_with_constant_bit_time() {
    for cnt in [0x10u8, 0x40, 0x00] {
        let mut k = kim();
        k.cpu.bus.write(map::CNTL30, cnt);
        let bit = k.tty_bit_cycles();
        // LDX #0 ; loop: TXA ; JSR OUTCH ; INX ; BNE loop ; JMP *
        let prog = [
            0xA2, 0x00, 0x8A, 0x20, 0xA0, 0x1E, 0xE8, 0xD0, 0xF9, 0x4C, 0x09, 0x02,
        ];
        if cnt == 0 {
            // 256 frames at the slowest rate: just the first 8.
            let p8 = [
                0xA2, 0x00, 0x8A, 0x20, 0xA0, 0x1E, 0xE8, 0xE0, 0x08, 0xD0, 0xF7, 0x4C, 0x0B, 0x02,
            ];
            run(&mut k, &p8, 0x20B, 10_000_000);
            assert_eq!(k.cpu.bus.tty_decode(bit), (0..8).collect::<Vec<u8>>());
        } else {
            run(&mut k, &prog, 0x209, 50_000_000);
            assert_eq!(k.cpu.bus.tty_decode(bit), (0..=255).collect::<Vec<u8>>());
        }
        // Every edge falls on the bit grid of its frame; the line starts with a start bit.
        let edges = &k.cpu.bus.tty_out;
        assert!(!edges[0].1);
        let mut frame_start = edges[0].0;
        for w in edges.windows(2) {
            let gap = w[1].0 - w[0].0;
            let _ = gap;
            if w[1].0 - frame_start >= 10 * bit {
                frame_start = w[1].0;
                continue;
            }
            assert_eq!(
                (w[1].0 - frame_start) % bit,
                0,
                "edge off the grid, CNTL30 {cnt:02X}"
            );
        }
        assert_eq!(k.cpu.a, if cnt == 0 { 7 } else { 255 }, "OUTCH keeps A");
    }
}

#[test]
fn prtbyt_crlf_outsp_text() {
    let mut k = kim();
    let bit = k.tty_bit_cycles();
    // LDA #$3C ; JSR PRTBYT ; JSR OUTSP ; LDA #$A5 ; JSR PRTBYT ; JSR CRLF ; JMP *
    let prog = [
        0xA9, 0x3C, 0x20, 0x3B, 0x1E, 0x20, 0x9E, 0x1E, 0xA9, 0xA5, 0x20, 0x3B, 0x1E, 0x20, 0x2F,
        0x1E, 0x4C, 0x10, 0x02,
    ];
    run(&mut k, &prog, 0x210, 5_000_000);
    assert_eq!(k.cpu.bus.tty_decode(bit), b"3C A5\r\n");
    assert_eq!(k.cpu.a, 0x0A);
}

#[test]
fn getch_reads_every_byte_from_pa7() {
    let mut k = kim();
    let bit = k.tty_bit_cycles();
    // loop: JSR GETCH ; STA $0300,X ; INX ; BNE loop ; JMP *
    let prog = [
        0x20, 0x5A, 0x1E, 0x9D, 0x00, 0x03, 0xE8, 0xD0, 0xF7, 0x4C, 0x09, 0x02,
    ];
    k.cpu.bus.ram[0x200..0x200 + prog.len()].copy_from_slice(&prog);
    k.cpu.pc = 0x200;
    k.cpu.x = 0;
    for b in 0..=255u8 {
        k.cpu
            .bus
            .tty_queue_byte(b.rotate_left(3) ^ 0x5A, bit, 3 * bit);
    }
    let mut n = 0u64;
    while k.cpu.pc != 0x209 {
        k.run_instructions(1).unwrap();
        n += 1;
        assert!(n < 50_000_000);
    }
    let _ = (0x0300u16..0x0400)
        .map(|a| k.cpu.bus.peek(a))
        .collect::<Vec<_>>();
    for b in 0..=255u8 {
        // RAM is 1 KiB: $0300 page holds the bytes.
        assert_eq!(
            k.cpu.bus.ram[0x300 + usize::from(b)],
            b.rotate_left(3) ^ 0x5A
        );
    }
}

#[test]
fn getch_tolerates_three_percent_line_speed_error() {
    for pct in [-3i64, 3] {
        let mut k = kim();
        let bit = (k.tty_bit_cycles() as i64 * (100 + pct) / 100) as u64;
        let prog = [0x20, 0x5A, 0x1E, 0x85, 0x10, 0x4C, 0x05, 0x02];
        for b in [0x00u8, 0xFF, 0x55, 0xAA, 0x81] {
            k.cpu.bus.tty_queue_byte(b, bit, bit);
            run(&mut k, &prog, 0x205, 1_000_000);
            assert_eq!(k.cpu.bus.ram[0x10], b, "{pct}% {b:02X}");
        }
    }
}

#[test]
fn pb7_timer_irq_needs_the_jumper_and_reads_low_on_pb7() {
    for (chip, base) in [(0usize, 0x1740u16), (1, 0x1700)] {
        for jumper in [false, true] {
            let mut k = kim();
            k.cpu.bus.pb7_irq_jumper[chip] = jumper;
            k.cpu.bus.write(map::IRQV, 0x00);
            k.cpu.bus.write(map::IRQV + 1, 0x03);
            k.cpu.bus.ram[0x300] = 0xE6; // INC $20
            k.cpu.bus.ram[0x301] = 0x20;
            k.cpu.bus.ram[0x302] = 0xAD; // LDA timer (A3 = 0: clears the flag and disables)
            k.cpu.bus.ram[0x303] = (base as u8) | 0x06;
            k.cpu.bus.ram[0x304] = 0x17;
            k.cpu.bus.ram[0x305] = 0x40; // RTI
                                         // CLI ; LDA #$10 ; STA base+$0D (/8, IRQ on) ; JMP *
            let lo = (base as u8) | 0x0D;
            let prog = [0x58, 0xA9, 0x10, 0x8D, lo, 0x17, 0x4C, 0x06, 0x02];
            k.cpu.bus.ram[0x200..0x209].copy_from_slice(&prog);
            k.cpu.pc = 0x200;
            k.run_instructions(3).unwrap();
            let pb = base + 2;
            assert_eq!(k.cpu.bus.read(pb) & 0x80, 0x80);
            k.run_instructions(60).unwrap(); // 17 * 8 cycles pass
            if jumper {
                assert_eq!(k.cpu.bus.ram[0x20], 1, "handler ran once");
                assert_eq!(k.cpu.bus.read(pb) & 0x80, 0x80, "flag cleared by the read");
            } else {
                assert_eq!(k.cpu.bus.ram[0x20], 0);
                assert_eq!(k.cpu.bus.read(pb) & 0x80, 0, "PB7 pulled low by the timer");
                assert!(!k.cpu.bus.irq());
            }
        }
    }
}

#[test]
fn tape_record_layout_and_checksum() {
    let data = [0xA9, 0x01, 0x00, 0xFF];
    let s = tape::encode(0x42, 0x0200, &data);
    assert!(s[..100].iter().all(|&c| c == 0x16));
    let tail = &s[100..];
    let text = String::from_utf8(tail.to_vec()).unwrap();
    // ID 42, SA 0200 (low first), data, '/', checksum = 00+02+A9+01+00+FF = $01AB.
    assert_eq!(text, "*420002A90100FF/AB01\u{4}\u{4}");
    assert_eq!(tape::checksum(0x0200, &data), 0x01AB);
}

#[test]
fn tape_round_trips_and_rejects_damage() {
    for len in [0usize, 1, 2, 255, 1000] {
        let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
        let s = tape::encode(7, 0x1234, &data);
        let recs = tape::decode_all(&s).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(
            (recs[0].id, recs[0].start, &recs[0].data),
            (7, 0x1234, &data)
        );
        if len > 0 {
            for pos in [103, 106, 107 + len] {
                let mut bad = s.clone();
                bad[pos] = if bad[pos] == b'0' { b'1' } else { b'0' };
                assert!(tape::decode_all(&bad).is_err(), "len {len} pos {pos}");
            }
            // The ID is outside the checksum: a damaged ID reads as another record.
            let mut other = s.clone();
            other[102] = b'8';
            assert_eq!(tape::decode_all(&other).unwrap()[0].id, 8);
            let mut bad = s.clone();
            bad[108] = b'G';
            assert!(tape::decode_all(&bad).is_err());
        }
    }
    let mut two = tape::encode(1, 0x0010, &[1, 2]);
    two.extend(tape::encode(2, 0x0020, &[3]));
    let recs = tape::decode_all(&two).unwrap();
    assert_eq!(recs.iter().map(|r| r.id).collect::<Vec<_>>(), [1, 2]);
}

#[test]
fn tape_audio_tones_and_round_trip() {
    let rate = 100_000;
    let s = tape::encode(0x10, 0x0300, &[0x00, 0xFF, 0x5A]);
    let audio = tape::to_audio(&s, rate);
    let tone = (0.002484f64 * f64::from(rate)).round() as usize;
    assert_eq!(audio.len(), s.len() * 8 * 3 * tone);
    // 134.2 bit/s.
    let bps = f64::from(rate) / (3 * tone) as f64;
    assert!((bps - 134.2).abs() < 0.3, "{bps}");
    assert_eq!(tape::from_audio(&audio, rate), s);
    // SYN = $16 = 0b0001_0110 LSB first: bit 0 is 0 → middle tone high frequency.
    let crossings = |seg: &[i16]| seg.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
    let mid0 = &audio[tone..2 * tone];
    let mid1 = &audio[3 * tone + tone..3 * tone + 2 * tone];
    assert!(crossings(mid0) > crossings(mid1));
}

#[test]
fn save_and_load_through_the_board_with_id_rules() {
    let mut k = kim();
    for i in 0..16u16 {
        k.cpu.bus.write(0x0200 + i, i as u8 * 3);
    }
    let rec = k.save_tape(0x05, 0x0200, 0x0210);
    let mut stream = tape::encode(0x04, 0x0100, &[9, 9]);
    stream.extend(&rec);
    // Matching ID: lands at the tape address, end exclusive in EAL/EAH.
    let mut b = kim();
    b.cpu.bus.write(map::ID, 0x05);
    let r = b.load_tape(&stream).unwrap();
    assert_eq!(r.id, 5);
    assert_eq!(
        (0..16)
            .map(|i| b.cpu.bus.peek(0x200 + i))
            .collect::<Vec<_>>(),
        (0..16).map(|i| i as u8 * 3).collect::<Vec<_>>()
    );
    assert_eq!(
        (b.cpu.bus.peek(map::EAL), b.cpu.bus.peek(map::EAH)),
        (0x10, 0x02)
    );
    // ID $00: first record, at its own address.
    let mut b = kim();
    b.cpu.bus.write(map::ID, 0x00);
    assert_eq!(b.load_tape(&stream).unwrap().id, 4);
    assert_eq!(b.cpu.bus.peek(0x100), 9);
    // ID $FF: first record, at SAL/SAH.
    let mut b = kim();
    b.cpu.bus.write(map::ID, 0xFF);
    b.cpu.bus.write(map::SAL, 0x80);
    b.cpu.bus.write(map::SAH, 0x03);
    b.load_tape(&stream).unwrap();
    assert_eq!((b.cpu.bus.peek(0x380), b.cpu.bus.peek(0x381)), (9, 9));
    assert_eq!(b.cpu.bus.peek(0x100), 0);
    // Unknown ID.
    let mut b = kim();
    b.cpu.bus.write(map::ID, 0x77);
    assert!(b.load_tape(&stream).is_err());
}
