//! v0.5 coverage: bus-cycle subset, assembler macros/locals/include.
//! ACIA, JSON images, PCM, and the debug listener have their own module tests.

use std::collections::HashMap;

use mos6502::bus::{FlatMemory, MemoryBus};
use mos6502::cycles::is_bus_cycle_stepped;
use mos6502::flags::bits;
use mos6502::{assemble, assemble_with_files, Cpu};

fn cpu_at(bytes: &[u8], pc: u16) -> Cpu<FlatMemory> {
    let mut mem = FlatMemory::new();
    mem.load_at(pc, bytes);
    let mut cpu = Cpu::new(mem);
    cpu.pc = pc;
    cpu.cycles = 0;
    cpu
}

fn run_until_complete(cpu: &mut Cpu<FlatMemory>) -> Vec<mos6502::BusCycle> {
    let mut trace = Vec::new();
    for _ in 0..8 {
        let cycle = cpu.step_cycle().unwrap();
        let done = cycle.complete;
        trace.push(cycle);
        if done {
            break;
        }
    }
    trace
}

#[test]
fn immediate_zp_abs_and_abs_x_loads_and_stores() {
    let mut cpu = cpu_at(&[0xA9, 0x42], 0x8000);
    let trace = run_until_complete(&mut cpu);
    assert_eq!(trace.len(), 2);
    assert_eq!(cpu.a, 0x42);
    assert!(trace[1].complete);
    assert!(!trace.iter().any(|c| c.write));

    let mut cpu = cpu_at(&[0xA5, 0x10], 0x8000);
    cpu.bus.write(0x10, 0x99);
    let trace = run_until_complete(&mut cpu);
    assert_eq!(trace.len(), 3);
    assert_eq!(trace[2].address, 0x0010);
    assert_eq!(cpu.a, 0x99);

    let mut cpu = cpu_at(&[0xAD, 0x00, 0x40], 0x8000);
    cpu.bus.write(0x4000, 0x0A);
    let trace = run_until_complete(&mut cpu);
    assert_eq!(trace.len(), 4);
    assert_eq!(cpu.a, 0x0A);

    let mut cpu = cpu_at(&[0xA2, 0x01, 0xBD, 0x00, 0x20], 0x8000);
    cpu.step().unwrap(); // LDX is itself cycle-stepped; use step() for it
                         // LDX immediate is cycle-stepped, but step() must still work on a boundary.
    assert_eq!(cpu.x, 1);
    cpu.bus.write(0x2001, 0x77);
    let trace = run_until_complete(&mut cpu);
    assert_eq!(trace.len(), 4, "no page cross");
    assert!(trace.iter().all(|c| !c.page_cross_penalty));
    assert_eq!(cpu.a, 0x77);

    let mut cpu = cpu_at(&[0xBD, 0xFF, 0x20], 0x8000);
    cpu.x = 1;
    cpu.bus.write(0x2100, 0x5A);
    cpu.bus.write(0x2000, 0x11);
    let trace = run_until_complete(&mut cpu);
    assert_eq!(trace.len(), 5);
    assert!(trace
        .iter()
        .any(|c| c.page_cross_penalty && c.address == 0x2000 && !c.write));
    assert_eq!(trace.last().unwrap().address, 0x2100);
    assert_eq!(cpu.a, 0x5A);

    let mut cpu = cpu_at(&[0x85, 0x10], 0x8000);
    cpu.a = 0x3C;
    let trace = run_until_complete(&mut cpu);
    assert_eq!(trace.len(), 3);
    assert!(trace[2].write && trace[2].data == 0x3C);
    assert_eq!(cpu.bus.peek(0x10), 0x3C);

    let mut cpu = cpu_at(&[0x8D, 0x34, 0x12], 0x8000);
    cpu.a = 0x01;
    let trace = run_until_complete(&mut cpu);
    assert_eq!(trace.len(), 4);
    assert_eq!(cpu.bus.peek(0x1234), 0x01);

    let mut cpu = cpu_at(&[0x9D, 0x00, 0x30], 0x8000);
    cpu.a = 0xAB;
    cpu.x = 2;
    let trace = run_until_complete(&mut cpu);
    assert_eq!(
        trace.len(),
        5,
        "STA abs,X is 5 cycles even without a page cross"
    );
    assert!(trace.iter().all(|c| !c.page_cross_penalty));
    assert!(trace.last().unwrap().write);
    assert_eq!(cpu.bus.peek(0x3002), 0xAB);

    let mut cpu = cpu_at(&[0x9D, 0xFF, 0x30], 0x8000);
    cpu.a = 0xCD;
    cpu.x = 1;
    let trace = run_until_complete(&mut cpu);
    assert_eq!(trace.len(), 5);
    assert!(trace.iter().any(|c| c.page_cross_penalty));
    assert_eq!(cpu.bus.peek(0x3100), 0xCD);
}

#[test]
fn branches_jsr_rts_brk_irq_and_nmi() {
    // BNE not taken (Z=1): 2 cycles, PC stays after the branch.
    let mut cpu = cpu_at(&[0xD0, 0x10], 0x80FE);
    cpu.p.set(bits::Z, true);
    let trace = run_until_complete(&mut cpu);
    assert_eq!(trace.len(), 2);
    assert_eq!(cpu.pc, 0x8100);

    // BNE taken, offset 0, no page cross: 3 cycles.
    let mut cpu = cpu_at(&[0xD0, 0x00], 0x80FE);
    cpu.p.set(bits::Z, false);
    let trace = run_until_complete(&mut cpu);
    assert_eq!(trace.len(), 3);
    assert!(trace.iter().all(|c| !c.page_cross_penalty));
    assert_eq!(cpu.pc, 0x8100);

    // BNE taken, offset -1, crosses back to $80FF: 4 cycles.
    let mut cpu = cpu_at(&[0xD0, 0xFF], 0x80FE);
    cpu.p.set(bits::Z, false);
    let trace = run_until_complete(&mut cpu);
    assert_eq!(trace.len(), 4);
    assert!(trace.last().unwrap().page_cross_penalty);
    assert_eq!(cpu.pc, 0x80FF);

    // JSR $8006 / LDA #$11 / RTS
    let mut cpu = cpu_at(&[0x20, 0x06, 0x80, 0xA9, 0x11, 0x00, 0x60], 0x8000);
    let jsr = run_until_complete(&mut cpu);
    assert_eq!(jsr.len(), 6);
    assert_eq!(cpu.pc, 0x8006);
    assert_eq!(cpu.sp, 0xFB);
    assert_eq!(cpu.bus.peek(0x01FD), 0x80);
    assert_eq!(cpu.bus.peek(0x01FC), 0x02);
    let rts = run_until_complete(&mut cpu);
    assert_eq!(rts.len(), 6);
    assert_eq!(cpu.pc, 0x8003);
    let lda = run_until_complete(&mut cpu);
    assert_eq!(lda.len(), 2);
    assert_eq!(cpu.a, 0x11);

    let mut cpu = cpu_at(&[0x00, 0xEA], 0x8000);
    cpu.bus.write(0xFFFE, 0x00);
    cpu.bus.write(0xFFFF, 0x90);
    let trace = run_until_complete(&mut cpu);
    assert_eq!(trace.len(), 7);
    assert_eq!(cpu.pc, 0x9000);
    assert!(cpu.p.get(bits::I));
    assert_eq!(cpu.bus.peek(0x01FD), 0x80);
    assert_eq!(cpu.bus.peek(0x01FC), 0x02);
    let pushed = cpu.bus.peek(0x01FB);
    assert_ne!(pushed & bits::B, 0, "BRK pushes B");

    let mut via_step = cpu_at(&[0x00, 0xEA], 0x8000);
    via_step.bus.write(0xFFFE, 0x00);
    via_step.bus.write(0xFFFF, 0x90);
    via_step.sp = cpu.sp; // both start at default; compare after their own sequences
    let mut fresh = cpu_at(&[0x00, 0xEA], 0x8000);
    fresh.bus.write(0xFFFE, 0x00);
    fresh.bus.write(0xFFFF, 0x90);
    fresh.step().unwrap();
    assert_eq!(fresh.pc, cpu.pc);
    assert_eq!(fresh.sp, cpu.sp);
    assert_eq!(fresh.bus.peek(0x01FB) & bits::B, bits::B);

    let mut cpu = cpu_at(&[0xEA], 0x4000);
    cpu.p.set(bits::I, false);
    cpu.set_irq_line(true);
    cpu.bus.write(0xFFFE, 0x34);
    cpu.bus.write(0xFFFF, 0x12);
    // Raised before the first cycle: the NOP polls it and the IRQ follows the NOP.
    assert_eq!(run_until_complete(&mut cpu).len(), 2);
    let trace = run_until_complete(&mut cpu);
    assert_eq!(trace.len(), 7);
    assert_eq!(cpu.pc, 0x1234);
    assert_eq!(cpu.bus.peek(0x01FD), 0x40);
    assert_eq!(
        cpu.bus.peek(0x01FC),
        0x01,
        "IRQ pushes the PC after the NOP"
    );
    assert_eq!(cpu.bus.peek(0x01FB) & bits::B, 0);

    let mut cpu = cpu_at(&[0xEA], 0x2222);
    cpu.p.set(bits::I, true);
    cpu.pulse_nmi();
    cpu.bus.write(0xFFFA, 0x00);
    cpu.bus.write(0xFFFB, 0x30);
    assert_eq!(run_until_complete(&mut cpu).len(), 2);
    let trace = run_until_complete(&mut cpu);
    assert_eq!(trace.len(), 7);
    assert_eq!(cpu.pc, 0x3000);
    assert!(!cpu.nmi_pending());
}

#[test]
fn granular_opcodes_do_not_move_and_step_still_runs_them() {
    // Unstable ANE is traced: two reads, then A = (A | ANE_MAGIC) & X & imm.
    let mut cpu = cpu_at(&[0x8B, 0x0F], 0x8000);
    cpu.a = 0x11;
    cpu.x = 0x3C;
    let trace = cpu.trace_bus_instruction().unwrap();
    assert_eq!(trace.len(), 2);
    assert_eq!(cpu.a, (0x11 | mos6502::ANE_MAGIC) & 0x3C & 0x0F);
    assert!(is_bus_cycle_stepped(0x8B));
    assert!(is_bus_cycle_stepped(0x02));
    assert!(is_bus_cycle_stepped(0x07));
    assert!(is_bus_cycle_stepped(0xA9));
    assert!(is_bus_cycle_stepped(0xAA));

    let mut stepped = cpu_at(&[0x07, 0x10], 0x8000);
    stepped.a = 0x0F;
    stepped.bus.write(0x10, 0x01);
    let mut cycled = cpu_at(&[0x07, 0x10], 0x8000);
    cycled.a = 0x0F;
    cycled.bus.write(0x10, 0x01);
    let cycles = stepped.step().unwrap();
    let trace = run_until_complete(&mut cycled);
    assert_eq!(trace.len() as u32, cycles);
    assert_eq!(cycled.bus.peek(0x10), 0x02);
    assert_eq!(cycled.a, stepped.a);
    assert_eq!(cycled.p.raw(), stepped.p.raw());
}

#[test]
fn macros_locals_and_includes() {
    let src = "\
.macro store val, addr
    lda #\\val
    sta \\addr
.endmacro
    store $42, $10
1:
    dex
    bne 1b
start:
@loop:
    dey
    bne @loop
";
    let bytes = assemble(src, 0x8000).unwrap();
    assert_eq!(&bytes[..4], &[0xA9, 0x42, 0x85, 0x10]);
    // dex; bne 1b  -> CA D0 FD
    assert_eq!(&bytes[4..7], &[0xCA, 0xD0, 0xFD]);
    assert_eq!(&bytes[7..], &[0x88, 0xD0, 0xFD]);

    // One call deep is expanded. A three-deep chain expands. `.macro` inside `.macro` is an error.
    let one_level = "\
.macro leaf val
    lda #\\val
.endmacro
.macro wrap val
    leaf \\val
.endmacro
    wrap $42
";
    assert_eq!(assemble(one_level, 0x8000).unwrap(), vec![0xA9, 0x42]);

    let two_levels = "\
.macro leaf
    nop
.endmacro
.macro mid
    leaf
.endmacro
.macro top
    mid
.endmacro
    top
";
    assert_eq!(assemble(two_levels, 0).unwrap(), vec![0xEA]);

    let nested = ".macro outer\n.macro inner\n    nop\n.endmacro\n.endmacro\n";
    assert!(assemble(nested, 0)
        .unwrap_err()
        .to_string()
        .contains("aninhada"));

    let mut files = HashMap::new();
    files.insert("inc.asm".into(), "    lda #$07\n".into());
    let included = assemble_with_files(".include \"inc.asm\"\n    brk\n", 0x8000, &files).unwrap();
    assert_eq!(included.bytes, vec![0xA9, 0x07, 0x00]);

    files.insert("self.asm".into(), ".include \"self.asm\"\n".into());
    assert!(assemble_with_files(".include \"self.asm\"\n", 0, &files).is_err());
}
