//! Famicom Disk System: `.fds` parsing, the disk bit stream (gaps, `$80` marks,
//! CRC-16/KERMIT), the RAM adapter's timer IRQ, drive transfers in both
//! directions with CRC checking and end-of-head, the wavetable/modulation audio
//! unit, and booting a machine from a (synthetic) BIOS plus disk.

use mos6502::ines::{parse_fds, FdsError, FDS_MAGIC, FDS_SIDE};
use mos6502::mapper::{
    fds_crc, fds_raw_side, Cartridge, Fds, FdsAudio, FDS_BLOCK_GAP, FDS_BYTE_CYCLES, FDS_LEAD_GAP,
    FDS_MOD_STEPS,
};
use mos6502::ppu::Mirroring;
use mos6502::systems::nes2a03::Nes2A03;

fn side(files: &[&[u8]]) -> Vec<u8> {
    let mut s = vec![1u8];
    s.extend_from_slice(b"*NINTENDO-HVC*");
    s.extend((0..41).map(|i| i as u8 ^ 0x5A));
    s.extend([2, files.len() as u8]);
    for (n, f) in files.iter().enumerate() {
        let mut h = vec![3, n as u8, n as u8];
        h.extend_from_slice(b"FILENAME");
        h.extend(0x6000u16.to_le_bytes());
        h.extend((f.len() as u16).to_le_bytes());
        h.push(0);
        s.extend(h);
        s.push(4);
        s.extend_from_slice(f);
    }
    s.resize(FDS_SIDE, 0);
    s
}

#[test]
fn crc_is_kermit() {
    assert_eq!(fds_crc(b"123456789"), 0x2189);
    assert_eq!(fds_crc(&[]), 0);
    let mut m = b"\x80hello".to_vec();
    let c = fds_crc(&m);
    m.extend(c.to_le_bytes());
    assert_eq!(fds_crc(&m), 0, "appending the CRC leaves residue 0");
}

#[test]
fn raw_side_layout() {
    let files: [&[u8]; 2] = [&[1, 2, 3, 4, 5], &[9; 300]];
    let s = side(&files);
    let raw = fds_raw_side(&s);
    assert!(raw[..FDS_LEAD_GAP].iter().all(|&b| b == 0));
    let mut at = FDS_LEAD_GAP;
    let mut src = 0usize;
    for len in [56usize, 2, 16, 6, 16, 301] {
        assert_eq!(raw[at], 0x80, "gap terminator");
        let block = &raw[at + 1..at + 1 + len];
        assert_eq!(block, &s[src..src + len]);
        let crc = u16::from_le_bytes([raw[at + 1 + len], raw[at + 2 + len]]);
        assert_eq!(crc, fds_crc(&raw[at..at + 1 + len]));
        assert!(raw[at + 3 + len..at + 3 + len + FDS_BLOCK_GAP]
            .iter()
            .all(|&b| b == 0));
        at += 3 + len + FDS_BLOCK_GAP;
        src += len;
    }
    assert_eq!(raw.len(), at, "stops at the padding");
    assert_eq!((FDS_LEAD_GAP, FDS_BLOCK_GAP), (28300 / 8, 976 / 8));
}

#[test]
fn fds_file_parsing() {
    let a = side(&[&[7]]);
    let b = side(&[&[8, 8]]);
    let mut raw = a.clone();
    raw.extend(&b);
    assert_eq!(parse_fds(&raw).unwrap(), vec![a.clone(), b.clone()]);
    let mut hdr = FDS_MAGIC.to_vec();
    hdr.push(2);
    hdr.extend([0; 11]);
    hdr.extend(&raw);
    assert_eq!(parse_fds(&hdr).unwrap().len(), 2);
    hdr[4] = 3;
    assert_eq!(
        parse_fds(&hdr),
        Err(FdsError::SideCount { header: 3, data: 2 })
    );
    assert_eq!(parse_fds(&raw[..100]), Err(FdsError::BadSize(100)));
    assert_eq!(parse_fds(&[]), Err(FdsError::BadSize(0)));
    let mut bad = raw.clone();
    bad[FDS_SIDE + 3] = b'X';
    assert_eq!(parse_fds(&bad), Err(FdsError::NotADisk(1)));
}

fn fds(files: &[&[u8]]) -> Fds {
    let mut bios = vec![0xEAu8; 0x2000];
    bios[0x1FFC..].copy_from_slice(&[0x00, 0xE0, 0x00, 0xE0]);
    bios[..3].copy_from_slice(&[0x4C, 0x00, 0xE0]);
    Fds::new(bios, &[side(files)]).unwrap()
}

#[test]
fn timer_irq_reload_repeat_and_acks() {
    for reload in [0u16, 1, 2, 5, 300, 0xFFFF] {
        for repeat in [false, true] {
            let mut c = fds(&[]);
            c.write(0x4023, 1);
            c.write(0x4020, reload as u8);
            c.write(0x4021, (reload >> 8) as u8);
            c.write(0x4022, 2 | u8::from(repeat));
            let first = (1..=0x10002u32)
                .find(|_| {
                    c.clock();
                    c.irq()
                })
                .unwrap();
            assert_eq!(first, u32::from(reload) + 1, "reload {reload}");
            assert_eq!(c.read(0x4030) & 1, 1);
            assert!(!c.irq(), "$4030 read acknowledges");
            if reload < 10 {
                let next = (1..=40u32).find(|_| {
                    c.clock();
                    c.irq()
                });
                assert_eq!(
                    next,
                    repeat.then_some(u32::from(reload) + 1),
                    "repeat {repeat}"
                );
            }
        }
    }
    let mut c = fds(&[]);
    c.write(0x4022, 2);
    c.clock();
    assert!(!c.irq(), "$4022 ignored with disk registers off");
    c.write(0x4023, 1);
    c.write(0x4022, 2); // reload 0: immediate
    c.clock();
    assert!(c.irq());
    c.write(0x4022, 0);
    assert!(!c.irq(), "$4022 bit 1 clear acknowledges");
    c.write(0x4022, 3);
    c.clock();
    assert!(c.irq());
    c.write(0x4023, 0);
    assert!(!c.irq(), "$4023 bit 0 clear acknowledges and stops");
    c.write(0x4023, 1);
    for _ in 0..10 {
        c.clock();
    }
    assert!(!c.irq());
    // $4020/$4021 work even with disk registers off.
    c.write(0x4023, 0);
    c.write(0x4020, 0x34);
    c.write(0x4021, 0x12);
    assert_eq!(c.reload, 0x1234);
}

/// Read bytes from the drive: returns (bytes, cycles between transfers).
fn read_bytes(c: &mut Fds, n: usize) -> (Vec<u8>, Vec<u32>) {
    let (mut out, mut gaps, mut t) = (vec![], vec![], 0u32);
    while out.len() < n {
        c.clock();
        t += 1;
        assert!(t < 2_000_000, "stalled");
        if c.peek(0x4030) & 0x80 != 0 {
            assert!(c.irq(), "byte IRQ enabled");
            out.push(c.read(0x4031));
            assert!(!c.irq(), "$4031 acknowledges");
            gaps.push(t);
            t = 0;
        }
    }
    (out, gaps)
}

#[test]
fn drive_reads_blocks_with_crc_check() {
    let files: [&[u8]; 1] = [&[0xDE, 0xAD, 0xBE, 0xEF, 0x01]];
    let disk = side(&files);
    let mut c = fds(&files);
    assert_eq!(c.peek(0x4032) & 7, 0b010, "inserted, not scanning");
    c.write(0x4023, 0x83);
    c.write(0x4025, 0x2E); // reset, motor off
    c.write(0x4025, 0xAD); // scan, motor on, read, no CRC: nothing transfers
    for _ in 0..FDS_BYTE_CYCLES as u32 * 100 {
        c.clock();
        assert_eq!(c.peek(0x4030) & 0x80, 0);
    }
    assert_eq!(c.peek(0x4032) & 2, 0, "ready while scanning");
    assert_eq!(
        c.peek(0x4033) & 0x80,
        0x80,
        "drive power reported while the motor runs"
    );
    c.write(0x4025, 0xE5 | 8); // + CRC enable, IRQ, horizontal arrangement
    assert_eq!(c.mirroring(), Some(Mirroring::Horizontal));
    assert_eq!(c.peek(0x4030) & 8, 8);
    let (b, gaps) = read_bytes(&mut c, 56);
    assert_eq!(b, disk[..56]);
    assert!(gaps[1..].iter().all(|&g| g == u32::from(FDS_BYTE_CYCLES)));
    c.write(0x4025, 0xF5); // CRC transfer
    read_bytes(&mut c, 2);
    assert_eq!(c.peek(0x4030) & 0x10, 0, "CRC passes");
    // Next block after re-arming CRC (BIOS disables it between blocks).
    c.write(0x4025, 0xA5);
    c.write(0x4025, 0xE5);
    let (b, _) = read_bytes(&mut c, 2);
    assert_eq!(b, [2, 1]);
    c.write(0x4025, 0xF5);
    read_bytes(&mut c, 2);
    assert_eq!(c.peek(0x4030) & 0x10, 0);
    // A corrupted byte fails the CRC.
    let mut c = fds(&files);
    let pos = FDS_LEAD_GAP + 10;
    c.sides[0][pos] ^= 1;
    c.write(0x4023, 0x83);
    c.write(0x4025, 0xE5);
    read_bytes(&mut c, 56);
    c.write(0x4025, 0xF5);
    read_bytes(&mut c, 2);
    assert_eq!(c.peek(0x4030) & 0x10, 0x10, "CRC error");
    // Reaching the end of the side.
    let len = c.sides[0].len();
    c.write(0x4025, 0x25);
    for _ in 0..(len as u32 + 2) * u32::from(FDS_BYTE_CYCLES) {
        c.clock();
    }
    assert_eq!(c.peek(0x4030) & 0x40, 0x40, "end of head");
    assert_eq!(c.peek(0x4032) & 2, 2, "not ready");
    c.write(0x4025, 0x24);
    c.write(0x4025, 0x25);
    assert_eq!(
        (c.peek(0x4030) & 0x40, c.head),
        (0, 0),
        "scan reset rewinds"
    );
    c.insert(None);
    assert_eq!(c.peek(0x4032) & 7, 7);
    c.insert(Some(5));
    assert_eq!(c.inserted, None, "no such side");
}

#[test]
fn drive_writes_a_block_that_reads_back() {
    let mut c = fds(&[]);
    c.write(0x4023, 0x83);
    c.write(0x4025, 0x21); // scan, write mode, CRC off: gap zeros
    let start = c.head;
    let data = [0x80u8, 3, 0x11, 0x22, 0x33];
    let mut pending = data.iter();
    c.write(0x4024, *pending.next().unwrap());
    c.write(0x4025, 0xC1); // CRC on, IRQ
    let mut written = 0;
    let mut first = None;
    while written < data.len() {
        c.clock();
        if c.irq() {
            first.get_or_insert(c.head - 1);
            written += 1;
            match pending.next() {
                Some(&v) => c.write(0x4024, v),
                None => c.write(0x4025, 0xD1), // CRC transfer next
            }
        }
    }
    let at = first.unwrap();
    assert!(at >= start);
    for _ in 0..2 * u32::from(FDS_BYTE_CYCLES) {
        c.clock();
    }
    assert_eq!(&c.sides[0][at..at + 5], &data);
    let crc = fds_crc(&data).to_le_bytes();
    assert_eq!(&c.sides[0][at + 5..at + 7], &crc);
    // Read it back from the start with CRC checking.
    c.write(0x4025, 0x26);
    c.read(0x4030);
    c.write(0x4025, 0xE5);
    let (b, _) = read_bytes(&mut c, 4);
    assert_eq!(b, data[1..]);
    c.write(0x4025, 0xF5);
    read_bytes(&mut c, 2);
    assert_eq!(c.peek(0x4030) & 0x10, 0);
    // Write-protected sides are untouched.
    let mut c = fds(&[]);
    c.write_protected = true;
    assert_eq!(c.peek(0x4032) & 4, 4);
    let before = c.sides[0].clone();
    c.write(0x4023, 1);
    c.write(0x4024, 0x77);
    c.write(0x4025, 0xC1);
    for _ in 0..5000 {
        c.clock();
    }
    assert_eq!(c.sides[0], before);
}

#[test]
fn disk_irq_acknowledge_paths() {
    for ack in 0..4 {
        let mut c = fds(&[]);
        c.write(0x4023, 1);
        c.write(0x4025, 0xC1);
        while !c.irq() {
            c.clock();
        }
        match ack {
            0 => {
                c.read(0x4030);
            }
            1 => {
                c.read(0x4031);
            }
            2 => {
                c.read(0x4032);
            }
            _ => c.write(0x4024, 0),
        }
        assert!(!c.irq(), "ack {ack}");
    }
}

#[test]
fn memory_map() {
    let mut c = fds(&[]);
    for a in [0x6000u16, 0x7FFF, 0x8000, 0xDFFF] {
        c.write(a, (a >> 8) as u8);
        assert_eq!(c.peek(a), (a >> 8) as u8);
    }
    c.write(0xE000, 0x12);
    assert_eq!(c.peek(0xE000), 0x4C, "BIOS is ROM");
    assert_eq!(c.mirroring(), Some(Mirroring::Vertical));
    assert!(Fds::new(vec![0; 100], &[]).is_err());
}

fn reference_pitch(pitch: u32, counter: i32, gain: i32) -> u32 {
    let mut temp = counter * gain;
    if (temp & 0x0f) != 0 && (temp & 0x800) == 0 {
        temp += 0x20;
    }
    temp += 0x400;
    let temp = ((temp >> 4) & 0xff) as u32;
    (pitch * temp) & 0xFFFFF
}

#[test]
fn audio_pitch_formula_all_counters_and_gains() {
    for counter in -64..=63i8 {
        for gain in 0..=63u8 {
            for freq in [0u16, 1, 0x123, 0x800, 0xFFF] {
                // The reference works on the 12-bit result as the hardware does.
                let t = (i32::from(counter) * i32::from(gain)) & 0xFFF;
                let t = if t & 0x800 != 0 { t - 0x1000 } else { t };
                assert_eq!(
                    FdsAudio::wave_pitch(freq, counter, gain),
                    reference_pitch(u32::from(freq), t, 1) & 0xFFFFF,
                    "{counter} {gain} {freq}"
                );
            }
        }
    }
    assert_eq!(FdsAudio::wave_pitch(0x100, 0, 0), 0x100 * 64);
}

fn audio_on() -> FdsAudio {
    let mut a = FdsAudio::default();
    a.write(0x4089, 0x80);
    for i in 0..64u16 {
        a.write(0x4040 + i, i as u8);
    }
    a.write(0x4089, 0);
    a
}

#[test]
fn wave_steps_at_the_documented_rate() {
    for freq in [0x40u16, 0x100, 0x400, 0xFFF] {
        let mut a = audio_on();
        a.write(0x4087, 0x80); // mod off
        a.write(0x4082, freq as u8);
        a.write(0x4083, (freq >> 8) as u8);
        let cycles = 16 * 4096u64;
        for _ in 0..cycles {
            a.clock();
        }
        // f = n * p / 16 / 2^12 table steps per second -> p steps per 65536 cycles.
        let acc = u64::from(freq) * 64 * (cycles / 16);
        assert_eq!(u64::from(a.wave_acc), acc & 0xFFFFFF, "{freq:03X}");
    }
    let mut a = audio_on();
    a.write(0x4083, 0x80);
    assert_eq!(
        (a.wave_acc, a.position()),
        (0, 0),
        "halt resets the accumulator"
    );
}

#[test]
fn envelopes_tick_every_8_e1_m1_cycles() {
    for (e, m) in [(0u8, 1u8), (1, 1), (3, 2), (0, 0xE8)] {
        let mut a = audio_on();
        a.write(0x4083, 0x00);
        a.write(0x408A, m);
        a.write(0x4080, 0x40 | e); // envelope on, increase
        let period = FdsAudio::envelope_period(e, m);
        assert_eq!(period, 8 * (u32::from(e) + 1) * (u32::from(m) + 1));
        for k in 1..=40u32 {
            for _ in 0..period {
                a.clock();
            }
            assert_eq!(u32::from(a.vol_gain), k.min(32), "e {e} m {m}");
        }
        assert_eq!(a.read(0x4090), Some(0x40 | 32));
        // Decrease, from a gain set above 32.
        a.write(0x4080, 0x80 | 40);
        a.write(0x4080, e);
        for _ in 0..period {
            a.clock();
        }
        assert_eq!(a.vol_gain, 39, "can still decrease above 32");
        // Speed 0 disables envelopes.
        a.write(0x408A, 0);
        for _ in 0..10 * period {
            a.clock();
        }
        assert_eq!(a.vol_gain, 39);
    }
    let mut a = audio_on();
    a.write(0x4084, 0x40);
    a.write(0x408A, 0);
    a.write(0x408A, 1);
    a.write(0x4083, 0x40);
    for _ in 0..1000 {
        a.clock();
    }
    assert_eq!(a.mod_gain, 0, "$4083 bit 6 halts the envelopes");
}

#[test]
fn mod_table_writes_and_counter_steps() {
    let mut a = audio_on();
    a.write(0x4087, 0x80);
    for i in 0..32u8 {
        a.write(0x4088, (i % 8) | 0xF8);
    }
    assert_eq!((a.mod_acc >> 13) & 31, 0, "32 writes wrap the address");
    assert_eq!(
        a.mod_table.to_vec(),
        (0..32u8).map(|i| i % 8).collect::<Vec<_>>()
    );
    // All +1 entries, carry every second tick.
    let mut a = audio_on();
    a.write(0x4087, 0x80);
    for _ in 0..32 {
        a.write(0x4088, 1);
    }
    a.write(0x4085, 0x3E);
    assert_eq!(a.mod_counter, 62);
    a.write(0x4086, 0x00);
    a.write(0x4087, 0x08); // freq $800, running
    a.write(0x4088, 7);
    assert_eq!(a.mod_table[0], 1, "table locked while running");
    let mut seen = vec![];
    for _ in 0..4 * 32 {
        a.clock();
        if seen.last() != Some(&a.mod_counter) {
            seen.push(a.mod_counter);
        }
    }
    assert_eq!(seen, [62, 63, -64, -63, -62]);
    assert_eq!(a.read(0x4097), Some(((-62i8) as u8) & 0x7F));
    for (e, d) in FDS_MOD_STEPS.iter().enumerate() {
        let mut a = audio_on();
        a.write(0x4087, 0x80);
        for _ in 0..32 {
            a.write(0x4088, e as u8);
        }
        a.write(0x4085, 10);
        a.write(0x4087, 0x40); // force a carry every tick
        for _ in 0..16 {
            a.clock();
        }
        assert_eq!(a.mod_counter, d.map_or(0, |d| 10 + d), "entry {e}");
    }
    let mut a = audio_on();
    a.write(0x4087, 0x80);
    a.write(0x4085, 0x40);
    assert_eq!(a.mod_counter, -64, "$4085 is 7-bit signed");
}

#[test]
fn output_volume_latch_master_and_wave_ram() {
    let mut a = audio_on();
    a.write(0x4080, 0x80 | 20);
    a.write(0x4082, 0x00);
    a.write(0x4083, 0x08);
    a.write(0x4087, 0x80);
    for _ in 0..16 {
        a.clock();
    }
    assert_eq!(a.vol_out, 20, "taken at position 0");
    a.write(0x4080, 0x80 | 40);
    while a.position() == 0 {
        a.clock();
    }
    for _ in 0..16 {
        a.clock();
    }
    assert_eq!(a.vol_out, 20, "changes wait for position 0");
    while a.position() != 0 {
        a.clock();
    }
    assert_eq!(a.vol_out, 32, "clamped to 32");
    a.write(0x4080, 0x80);
    assert_eq!(a.vol_out, 0, "muting is immediate");
    a.write(0x4080, 0xA0);
    while a.position() != 0 {
        a.clock();
    }
    while a.position() != 40 {
        a.clock();
    }
    let full = a.level();
    assert_eq!(full, 40.0 * 32.0);
    for (m, num) in [(1u8, 2.0f32 / 3.0), (2, 0.5), (3, 0.4)] {
        a.write(0x4089, m);
        assert!((a.level() - full * num).abs() < 1e-3);
    }
    assert_eq!(
        a.read(0x4040 + 5),
        Some(0x40 | 40),
        "reads the current sample"
    );
    a.write(0x4089, 0x80);
    let held = a.level();
    for _ in 0..200 {
        a.clock();
    }
    assert_eq!(a.level(), held, "write enable holds the output");
    assert_eq!(a.read(0x4045), Some(0x45));
    a.write(0x4045, 0xFF);
    assert_eq!(a.wave[5], 0x3F);
}

#[test]
fn sound_registers_need_4023_bit_1() {
    let mut c = fds(&[]);
    c.write(0x4089, 0x80);
    c.write(0x4040, 9);
    assert_eq!(c.audio.wave[0], 0);
    c.write(0x4023, 2);
    c.write(0x4040, 9);
    assert!(!c.audio.wave_write);
    c.write(0x4089, 0x80);
    c.write(0x4040, 9);
    assert_eq!(c.audio.wave[0], 9);
    c.write(0x4089, 0);
    c.write(0x4080, 0xA0);
    c.write(0x4083, 0);
    for _ in 0..64 {
        c.clock();
    }
    assert!(c.audio() > 0.0, "same polarity as the APU");
    c.write(0x4023, 0);
    assert_eq!(c.audio(), 0.0);
}

#[test]
fn machine_boots_fds_and_round_trips_states() {
    let mut bios = vec![0xEAu8; 0x2000];
    // $E000: enable I/O, run the timer, write PRG RAM and CHR RAM, loop.
    let prog = [
        0xA9, 0x83, 0x8D, 0x23, 0x40, // lda #$83 sta $4023
        0xA9, 0x10, 0x8D, 0x20, 0x40, // lda #$10 sta $4020
        0xA9, 0x00, 0x8D, 0x21, 0x40, // lda #0 sta $4021
        0xA9, 0x03, 0x8D, 0x22, 0x40, // lda #3 sta $4022
        0xA9, 0x5A, 0x8D, 0x00, 0x60, // lda #$5A sta $6000
        0x4C, 0x19, 0xE0, // jmp *
    ];
    bios[..prog.len()].copy_from_slice(&prog);
    bios[0x1FFA..].copy_from_slice(&[0x00, 0xE0, 0x00, 0xE0, 0x00, 0xE0]);
    let files: [&[u8]; 1] = [&[1, 2, 3]];
    let disk = side(&files);
    let mut a = Nes2A03::from_fds(&bios, &disk).unwrap();
    a.run_cycles(10_000).unwrap();
    assert_eq!(a.cpu.bus.cart.as_ref().unwrap().peek(0x6000), 0x5A);
    assert!(a.cpu.bus.chr_writable && a.cpu.bus.chr.len() == 0x2000);
    let s = a.save_state();
    let mut b = Nes2A03::from_fds(&bios, &disk).unwrap();
    b.load_state(&s).unwrap();
    a.run_cycles(5_000).unwrap();
    b.run_cycles(5_000).unwrap();
    assert_eq!(a.save_state(), b.save_state());
    // A different BIOS is refused.
    let mut other = bios.clone();
    other[100] ^= 1;
    let mut c = Nes2A03::from_fds(&other, &disk).unwrap();
    assert!(c.load_state(&s).is_err());
    assert!(Nes2A03::from_fds(&bios, &disk[..10]).is_err());
    assert!(Nes2A03::from_fds(&bios[..10], &disk).is_err());
}
