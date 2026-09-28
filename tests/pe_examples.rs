//! Runs the peripheral examples in `examples/` on small boards built here, and the
//! pure-CPU examples through the library, checking their documented results.

use mos6502::acia::Acia6551;
use mos6502::asm::assemble;
use mos6502::bus::{FlatMemory, MemoryBus};
use mos6502::cpu::Cpu;
use mos6502::via::Via6522;

/// 64 KiB RAM with a VIA at `$C000-$C00F` and an ACIA at `$D000-$D003`.
struct Board {
    ram: FlatMemory,
    via: Via6522,
    acia: Acia6551,
}

impl MemoryBus for Board {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0xC000..=0xC00F => self.via.read((addr & 0x0F) as u8),
            0xD000..=0xD003 => mos6502::bus::IoDevice::read(&mut self.acia, addr & 3),
            _ => self.ram.read(addr),
        }
    }
    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0xC000..=0xC00F => self.via.write((addr & 0x0F) as u8, value),
            0xD000..=0xD003 => mos6502::bus::IoDevice::write(&mut self.acia, addr & 3, value),
            _ => self.ram.write(addr, value),
        }
    }
    fn peek(&self, addr: u16) -> u8 {
        self.ram.peek(addr)
    }
}

fn board(src: &str) -> Cpu<Board> {
    let program = assemble(src, 0x8000).expect("example assembles");
    let mut ram = FlatMemory::new();
    ram.load_at(0x8000, &program);
    ram.set_reset_vector(0x8000);
    ram.set_irq_vector(0x8003);
    let mut cpu = Cpu::new(Board {
        ram,
        via: Via6522::new(),
        acia: Acia6551::new(),
    });
    cpu.reset();
    cpu
}

/// Instruction-level run: every CPU cycle ticks both devices once, and a device
/// IRQ is offered to the CPU between instructions (the CPU ignores it under I).
/// Returns the cycles actually run.
fn run(cpu: &mut Cpu<Board>, cycles: u64, mut each: impl FnMut(&mut Cpu<Board>)) -> u64 {
    let mut done = 0;
    while done < cycles {
        let used = u64::from(cpu.step().expect("step"));
        for _ in 0..used {
            cpu.bus.via.tick();
            cpu.bus.acia.tick();
            each(cpu);
        }
        done += used;
        if cpu.bus.via.irq_asserted() || cpu.bus.acia.irq_asserted() {
            cpu.irq();
        }
    }
    done
}

#[test]
fn via_clock_counts_ticks_and_seconds_from_timer1_interrupts() {
    let mut cpu = board(include_str!("../examples/via_clock.asm"));
    let mut pb7_edges = 0u32;
    let mut last = None;
    run(&mut cpu, 100_500, |c| {
        let now = c.bus.via.pb7_output();
        if last.is_some() && now != last {
            pb7_edges += 1;
        }
        last = now;
    });
    let ram = |c: &Cpu<Board>, a| c.bus.ram.peek(a);
    // 100 interrupts at 1000-cycle spacing: 10 seconds, ticks back to 0.
    assert_eq!(ram(&cpu, 0x01), 10, "seconds");
    assert_eq!(ram(&cpu, 0x00), 0, "ticks");
    assert_eq!(cpu.bus.via.orb_latch(), 10, "ORB mirrors seconds");
    assert_eq!(cpu.bus.via.port_b_pins() & 0x7F, 10);
    // PB7 inverts on every timeout once the T1C-H write drove it low.
    assert!((99..=101).contains(&pb7_edges), "PB7 edges {pb7_edges}");
}

#[test]
fn via_clock_interrupt_period_is_n_plus_two() {
    let mut cpu = board(include_str!("../examples/via_clock.asm"));
    let mut irq_at = Vec::new();
    let mut cycle = 0u64;
    run(&mut cpu, 6_000, |c| {
        cycle += 1;
        let far = irq_at.last().is_none_or(|&last| cycle - last > 100);
        if c.bus.via.t1_flag() && far {
            irq_at.push(cycle);
        }
    });
    assert!(irq_at.len() >= 5);
    for pair in irq_at.windows(2) {
        assert_eq!(pair[1] - pair[0], 1000);
    }
}

#[test]
fn acia_irq_echo_buffers_and_echoes_every_byte() {
    let mut cpu = board(include_str!("../examples/acia_irq_echo.asm"));
    run(&mut cpu, 200, |_| {});
    let message = b"Hello, 6551!\r";
    let mut echoed = Vec::new();
    for &b in message {
        cpu.bus.acia.push_rx(b);
        run(&mut cpu, 200, |c| {
            if let Some(x) = c.bus.acia.pop_tx() {
                echoed.push(x);
            }
        });
    }
    assert_eq!(echoed, message);
    assert_eq!(cpu.bus.ram.peek(0x10) as usize, message.len());
    let buffer: Vec<u8> = (0..message.len() as u16)
        .map(|i| cpu.bus.ram.peek(0x0200 + i))
        .collect();
    assert_eq!(buffer, message);
    assert!(!cpu.bus.acia.irq_asserted(), "status read cleared the IRQ");
}

#[test]
fn acia_irq_echo_ignores_bytes_while_idle_until_one_arrives() {
    let mut cpu = board(include_str!("../examples/acia_irq_echo.asm"));
    run(&mut cpu, 5_000, |_| {});
    assert_eq!(cpu.bus.ram.peek(0x10), 0);
    assert!(cpu.bus.acia.pop_tx().is_none());
}

fn run_pure(src: &str, max_instr: u64) -> Cpu<FlatMemory> {
    let program = assemble(src, 0x8000).unwrap();
    let mut ram = FlatMemory::new();
    ram.load_at(0x8000, &program);
    ram.set_reset_vector(0x8000);
    let mut cpu = Cpu::new(ram);
    cpu.halt_on_brk = true;
    cpu.reset();
    cpu.run_instructions(max_instr).unwrap();
    assert!(cpu.is_halted(), "example reaches BRK");
    cpu
}

#[test]
fn crc16_example_yields_the_ccitt_false_check_value() {
    let cpu = run_pure(include_str!("../examples/crc16.asm"), 10_000);
    let crc = u16::from_le_bytes([cpu.bus.peek(0), cpu.bus.peek(1)]);
    // Reference computed bit-by-bit here.
    let mut r: u16 = 0xFFFF;
    for &b in b"123456789" {
        r ^= u16::from(b) << 8;
        for _ in 0..8 {
            r = if r & 0x8000 != 0 {
                (r << 1) ^ 0x1021
            } else {
                r << 1
            };
        }
    }
    assert_eq!(r, 0x29B1);
    assert_eq!(crc, r);
}

#[test]
fn muldiv_routines_match_rust_arithmetic_over_many_operands() {
    let src = include_str!("../examples/muldiv.asm");
    let cpu = run_pure(src, 10_000);
    assert_eq!(
        u16::from_le_bytes([cpu.bus.peek(0), cpu.bus.peek(1)]),
        200 * 123
    );
    assert_eq!(
        u16::from_le_bytes([cpu.bus.peek(2), cpu.bus.peek(3)]),
        50_000 / 7
    );
    assert_eq!(cpu.bus.peek(4), (50_000 % 7) as u8);
    // Drive the same subroutines with other operands by patching the immediates.
    let program = assemble(src, 0x8000).unwrap();
    let cases = [
        (0u8, 0u8, 0u16, 1u8),
        (255, 255, 65_535, 127),
        (17, 15, 1, 1),
        (128, 2, 12_345, 100),
    ];
    for (a, b, dividend, divisor) in cases {
        let mut p = program.clone();
        // lda #a / sta / lda #b / sta / jsr / lda #lo / sta / lda #hi / sta / lda #d
        p[1] = a;
        p[5] = b;
        p[12] = dividend as u8;
        p[16] = (dividend >> 8) as u8;
        p[20] = divisor;
        let mut ram = FlatMemory::new();
        ram.load_at(0x8000, &p);
        ram.set_reset_vector(0x8000);
        let mut cpu = Cpu::new(ram);
        cpu.halt_on_brk = true;
        cpu.reset();
        cpu.run_instructions(10_000).unwrap();
        let m = |i| cpu.bus.peek(i);
        assert_eq!(
            u16::from_le_bytes([m(0), m(1)]),
            u16::from(a) * u16::from(b),
            "{a}*{b}"
        );
        assert_eq!(
            u16::from_le_bytes([m(2), m(3)]),
            dividend / u16::from(divisor)
        );
        assert_eq!(u16::from(m(4)), dividend % u16::from(divisor));
    }
}

#[test]
fn sieve_256_flags_exactly_the_primes() {
    let cpu = run_pure(include_str!("../examples/sieve_256.asm"), 100_000);
    let is_prime = |n: u16| n >= 2 && (2..n).take_while(|d| d * d <= n).all(|d| n % d != 0);
    for n in 0..256u16 {
        assert_eq!(cpu.bus.peek(0x0200 + n) == 1, is_prime(n), "n = {n}");
    }
    assert_eq!(cpu.bus.peek(0), 54);
}

#[test]
fn every_new_example_is_listed_in_the_examples_manifest() {
    let manifest = include_str!("../scripts/examples.tsv");
    for f in ["crc16.asm", "muldiv.asm", "sieve_256.asm"] {
        assert!(manifest.contains(&format!("examples/{f}")), "{f}");
    }
}
