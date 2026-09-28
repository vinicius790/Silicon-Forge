//! Golden traces / asserts por opcode — estilo nestest (funcional, não cycle-perfect).
//!
//! Cada teste monta um snippet em $8000, define reset vector, roda até BRK
//! (ou N instruções) e valida registradores / memória.

use mos6502::{bits, Cpu, FlatMemory, MemoryBus, Status};

fn cpu_with(program: &[u8]) -> Cpu<FlatMemory> {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, program);
    mem.set_reset_vector(0x8000);
    mem.set_irq_vector(0x0000);
    let mut cpu = Cpu::new(mem);
    cpu.halt_on_brk = true;
    cpu.reset();
    cpu
}

fn run_until_brk(cpu: &mut Cpu<FlatMemory>, max_insns: u64) {
    for _ in 0..max_insns {
        if cpu.is_halted() {
            break;
        }
        cpu.step().expect("step");
    }
    assert!(cpu.is_halted(), "esperado BRK (halt)");
}

// 1
#[test]
fn lda_immediate_sets_a_and_zn() {
    let mut cpu = cpu_with(&[0xA9, 0x00, 0x00]); // LDA #$00 / BRK
    run_until_brk(&mut cpu, 8);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.p.get(bits::Z));
    assert!(!cpu.p.get(bits::N));
}

// 2
#[test]
fn lda_immediate_negative() {
    let mut cpu = cpu_with(&[0xA9, 0x80, 0x00]);
    run_until_brk(&mut cpu, 8);
    assert_eq!(cpu.a, 0x80);
    assert!(cpu.p.get(bits::N));
    assert!(!cpu.p.get(bits::Z));
}

// 3
#[test]
fn sta_zero_page() {
    let mut cpu = cpu_with(&[0xA9, 0x42, 0x85, 0x10, 0x00]); // LDA #$42 / STA $10 / BRK
    run_until_brk(&mut cpu, 8);
    assert_eq!(cpu.bus.read(0x0010), 0x42);
}

// 4
#[test]
fn adc_immediate_carry_and_overflow() {
    // 0x50 + 0x50 = 0xA0, V=1, C=0, N=1
    let mut cpu = cpu_with(&[0xA9, 0x50, 0x18, 0x69, 0x50, 0x00]);
    run_until_brk(&mut cpu, 16);
    assert_eq!(cpu.a, 0xA0);
    assert!(cpu.p.get(bits::V));
    assert!(!cpu.p.get(bits::C));
    assert!(cpu.p.get(bits::N));
}

// 5
#[test]
fn adc_sets_carry() {
    let mut cpu = cpu_with(&[0xA9, 0xFF, 0x18, 0x69, 0x01, 0x00]);
    run_until_brk(&mut cpu, 16);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.p.get(bits::C));
    assert!(cpu.p.get(bits::Z));
}

// 6
#[test]
fn sbc_basic() {
    // A=0x50, SEC, SBC #$20 → 0x30
    let mut cpu = cpu_with(&[0xA9, 0x50, 0x38, 0xE9, 0x20, 0x00]);
    run_until_brk(&mut cpu, 16);
    assert_eq!(cpu.a, 0x30);
    assert!(cpu.p.get(bits::C)); // sem borrow
}

// 7
#[test]
fn sbc_borrow() {
    let mut cpu = cpu_with(&[0xA9, 0x20, 0x38, 0xE9, 0x30, 0x00]);
    run_until_brk(&mut cpu, 16);
    assert_eq!(cpu.a, 0xF0);
    assert!(!cpu.p.get(bits::C)); // borrow
    assert!(cpu.p.get(bits::N));
}

// 8
#[test]
fn jmp_absolute() {
    // JMP $8005; A9 99 (skipped); A9 77; BRK  — @8005 = LDA #$77
    let prog = [
        0x4C, 0x05, 0x80, // JMP $8005
        0xA9, 0x99, // skipped
        0xA9, 0x77, // LDA #$77
        0x00,
    ];
    let mut cpu = cpu_with(&prog);
    run_until_brk(&mut cpu, 16);
    assert_eq!(cpu.a, 0x77);
}

// 9
#[test]
fn jsr_rts_roundtrip() {
    // JSR $8006; BRK;  (pad) ; @8006: LDA #$55; RTS
    let prog = [
        0x20, 0x06, 0x80, // JSR $8006
        0x00, // BRK after return
        0xEA, 0xEA, // pad so $8006 aligns
        0xA9, 0x55, // LDA #$55
        0x60, // RTS
    ];
    let mut cpu = cpu_with(&prog);
    run_until_brk(&mut cpu, 32);
    assert_eq!(cpu.a, 0x55);
}

// 10
#[test]
fn bne_taken_and_not_taken() {
    // LDA #$01; BNE +2; LDA #$AA; LDA #$BB; BRK
    // path: A=1 → branch skips LDA #$AA → A=BB
    let prog = [
        0xA9, 0x01, // LDA #$01
        0xD0, 0x02, // BNE +2
        0xA9, 0xAA, // skipped
        0xA9, 0xBB, // LDA #$BB
        0x00,
    ];
    let mut cpu = cpu_with(&prog);
    run_until_brk(&mut cpu, 16);
    assert_eq!(cpu.a, 0xBB);
}

// 11
#[test]
fn beq_when_zero() {
    let prog = [
        0xA9, 0x00, // LDA #$00
        0xF0, 0x02, // BEQ +2
        0xA9, 0x11, // skipped
        0xA9, 0x22, 0x00,
    ];
    let mut cpu = cpu_with(&prog);
    run_until_brk(&mut cpu, 16);
    assert_eq!(cpu.a, 0x22);
}

// 12
#[test]
fn and_ora_eor_logic() {
    let prog = [
        0xA9, 0xF0, // LDA #$F0
        0x29, 0x0F, // AND #$0F → 00
        0x09, 0x0A, // ORA #$0A → 0A
        0x49, 0xFF, // EOR #$FF → F5
        0x00,
    ];
    let mut cpu = cpu_with(&prog);
    run_until_brk(&mut cpu, 16);
    assert_eq!(cpu.a, 0xF5);
}

// 13
#[test]
fn ldx_stx_transfer() {
    let prog = [
        0xA2, 0x33, // LDX #$33
        0x8E, 0x00, 0x02, // STX $0200
        0xA9, 0x00, // LDA #$00
        0x8A, // TXA
        0x00,
    ];
    let mut cpu = cpu_with(&prog);
    run_until_brk(&mut cpu, 16);
    assert_eq!(cpu.bus.read(0x0200), 0x33);
    assert_eq!(cpu.a, 0x33);
}

// 14
#[test]
fn stack_pha_pla() {
    let prog = [
        0xA9, 0xDE, // LDA #$DE
        0x48, // PHA
        0xA9, 0x00, // LDA #$00
        0x68, // PLA
        0x00,
    ];
    let mut cpu = cpu_with(&prog);
    run_until_brk(&mut cpu, 16);
    assert_eq!(cpu.a, 0xDE);
}

// 15
#[test]
fn cmp_sets_flags() {
    // A=0x40 CMP #$40 → Z=1 C=1; CMP #$41 → Z=0 C=0
    let mut cpu = cpu_with(&[0xA9, 0x40, 0xC9, 0x40, 0x00]);
    run_until_brk(&mut cpu, 8);
    assert!(cpu.p.get(bits::Z));
    assert!(cpu.p.get(bits::C));

    let mut cpu2 = cpu_with(&[0xA9, 0x40, 0xC9, 0x41, 0x00]);
    run_until_brk(&mut cpu2, 8);
    assert!(!cpu2.p.get(bits::Z));
    assert!(!cpu2.p.get(bits::C));
}

// 16
#[test]
fn inx_dex_wrap() {
    let prog = [0xA2, 0xFF, 0xE8, 0x00]; // LDX #$FF / INX / BRK
    let mut cpu = cpu_with(&prog);
    run_until_brk(&mut cpu, 8);
    assert_eq!(cpu.x, 0x00);
    assert!(cpu.p.get(bits::Z));

    let prog2 = [0xA2, 0x00, 0xCA, 0x00]; // DEX → FF
    let mut cpu2 = cpu_with(&prog2);
    run_until_brk(&mut cpu2, 8);
    assert_eq!(cpu2.x, 0xFF);
    assert!(cpu2.p.get(bits::N));
}

// 17
#[test]
fn asl_a_and_carry() {
    let prog = [0xA9, 0x80, 0x0A, 0x00]; // LDA #$80 / ASL A
    let mut cpu = cpu_with(&prog);
    run_until_brk(&mut cpu, 8);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.p.get(bits::C));
    assert!(cpu.p.get(bits::Z));
}

// 18
#[test]
fn rol_ror_with_carry() {
    // SEC; LDA #$00; ROL A → 01; ROR A → 00 C=1
    let prog = [0x38, 0xA9, 0x00, 0x2A, 0x6A, 0x00];
    let mut cpu = cpu_with(&prog);
    run_until_brk(&mut cpu, 16);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.p.get(bits::C));
}

// 19
#[test]
fn bit_zp_flags() {
    // mem[$10]=0xC0; LDA #$0F; BIT $10 → Z=1 (A&M=0), N=1, V=1
    let mut cpu = cpu_with(&[0xA9, 0x0F, 0x24, 0x10, 0x00]);
    cpu.bus.write(0x0010, 0xC0);
    run_until_brk(&mut cpu, 8);
    assert!(cpu.p.get(bits::Z));
    assert!(cpu.p.get(bits::N));
    assert!(cpu.p.get(bits::V));
    assert_eq!(cpu.a, 0x0F); // A inalterado
}

// 20
#[test]
fn bcc_bcs_carry() {
    let prog = [
        0x18, // CLC
        0x90, 0x02, // BCC +2
        0xA9, 0x11, // skipped
        0xA9, 0x22, // LDA #$22
        0x38, // SEC
        0xB0, 0x02, // BCS +2
        0xA9, 0x33, // skipped
        0xA9, 0x44, 0x00,
    ];
    let mut cpu = cpu_with(&prog);
    run_until_brk(&mut cpu, 32);
    assert_eq!(cpu.a, 0x44);
}

// 21
#[test]
fn lda_absolute_indexed_page_cross_cycles() {
    // LDA $80FF,X com X=1 → lê $8100, page cross → 5 ciclos
    let mut mem = FlatMemory::new();
    mem.data[0x8100] = 0xAB;
    mem.load_at(0x8000, &[0xA2, 0x01, 0xBD, 0xFF, 0x80, 0x00]); // LDX #1 / LDA $80FF,X / BRK
    mem.set_reset_vector(0x8000);
    mem.set_irq_vector(0x0000);
    let mut cpu = Cpu::new(mem);
    cpu.halt_on_brk = true;
    cpu.reset();
    let c0 = cpu.cycles;
    cpu.step().unwrap(); // LDX = 2
    let c1 = cpu.cycles;
    assert_eq!(c1 - c0, 2);
    cpu.step().unwrap(); // LDA abs,X page cross = 5
    assert_eq!(cpu.cycles - c1, 5);
    assert_eq!(cpu.a, 0xAB);
    run_until_brk(&mut cpu, 4);
}

// 22
#[test]
fn indirect_indexed_indy() {
    // ($10),Y: ptr $10/$11 = $0200; Y=3 → $0203
    let mut cpu = cpu_with(&[
        0xA0, 0x03, // LDY #3
        0xB1, 0x10, // LDA ($10),Y
        0x00,
    ]);
    cpu.bus.write(0x0010, 0x00);
    cpu.bus.write(0x0011, 0x02);
    cpu.bus.write(0x0203, 0x5A);
    run_until_brk(&mut cpu, 8);
    assert_eq!(cpu.a, 0x5A);
}

// 23
#[test]
fn indexed_indirect_indx() {
    // ($20,X) com X=4 → ptr em $24/$25 = $0300
    let mut cpu = cpu_with(&[
        0xA2, 0x04, // LDX #4
        0xA1, 0x20, // LDA ($20,X)
        0x00,
    ]);
    cpu.bus.write(0x0024, 0x00);
    cpu.bus.write(0x0025, 0x03);
    cpu.bus.write(0x0300, 0x7E);
    run_until_brk(&mut cpu, 8);
    assert_eq!(cpu.a, 0x7E);
}

// 24
#[test]
fn php_plp_roundtrip_forces_u() {
    let mut cpu = cpu_with(&[0x08, 0x28, 0x00]); // PHP / PLP / BRK
    cpu.p.set(bits::C, true);
    cpu.p.set(bits::N, true);
    run_until_brk(&mut cpu, 8);
    assert!(cpu.p.get(bits::C));
    assert!(cpu.p.get(bits::N));
    // bit U sempre 1 após PLP
    assert_eq!(cpu.p.raw() & bits::U, bits::U);
}

// 25
#[test]
fn golden_trace_multi_insn_registers() {
    // Programa: carrega, soma, guarda, compara
    // LDA #$10; ADC #$05; STA $00; CMP #$15; BRK
    let mut cpu = cpu_with(&[0xA9, 0x10, 0x18, 0x69, 0x05, 0x85, 0x00, 0xC9, 0x15, 0x00]);
    let mut trace = Vec::new();
    for _ in 0..10 {
        if cpu.is_halted() {
            break;
        }
        let before = format!("PC={:04X} A={:02X} P={:02X}", cpu.pc, cpu.a, cpu.p.raw());
        cpu.step().unwrap();
        trace.push(before);
    }
    assert_eq!(cpu.a, 0x15);
    assert_eq!(cpu.bus.read(0x0000), 0x15);
    assert!(cpu.p.get(bits::Z));
    assert!(cpu.p.get(bits::C));
    // Trace mínimo: pelo menos 5 snapshots
    assert!(trace.len() >= 5, "trace curto: {trace:?}");
    // Snapshot inicial após reset aponta para $8000
    assert!(trace[0].starts_with("PC=8000"), "{}", trace[0]);
}

// 26
#[test]
fn status_display_and_from_stack() {
    let mut s = Status::new();
    s.set(bits::C, true);
    s.set(bits::Z, true);
    let stacked = s.to_stack_php();
    assert_eq!(stacked & bits::B, bits::B);
    assert_eq!(stacked & bits::U, bits::U);
    let restored = Status::from_stack(stacked);
    assert!(restored.get(bits::C));
    assert!(restored.get(bits::Z));
    assert!(!restored.get(bits::B));
}

// 27
#[test]
fn run_cycles_budget() {
    // NOP loop: EA EA EA ... cada NOP = 2 ciclos
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xEA, 0xEA, 0xEA, 0xEA, 0xEA, 0x00]);
    mem.set_reset_vector(0x8000);
    mem.set_irq_vector(0x0000);
    let mut cpu = Cpu::new(mem);
    cpu.halt_on_brk = true;
    cpu.reset();
    let after_reset = cpu.cycles; // 7
    let ran = cpu.run_cycles(6).unwrap(); // 3 NOPs
    assert_eq!(ran, 6);
    assert_eq!(cpu.cycles, after_reset + 6);
    assert_eq!(cpu.pc, 0x8003);
}

// 28
#[test]
fn jmp_indirect_page_bug() {
    // JMP ($10FF): lo em $10FF, hi em $1000 (bug 6502), não $1100
    let mut mem = FlatMemory::new();
    mem.data[0x10FF] = 0x34;
    mem.data[0x1000] = 0x12; // hi bug
    mem.data[0x1100] = 0x99; // NÃO deve ser usado
                             // destino $1234: LDA #$EE / BRK
    mem.load_at(0x1234, &[0xA9, 0xEE, 0x00]);
    mem.load_at(0x8000, &[0x6C, 0xFF, 0x10]); // JMP ($10FF)
    mem.set_reset_vector(0x8000);
    mem.set_irq_vector(0x0000);
    let mut cpu = Cpu::new(mem);
    cpu.halt_on_brk = true;
    cpu.reset();
    run_until_brk(&mut cpu, 16);
    assert_eq!(cpu.a, 0xEE);
}
