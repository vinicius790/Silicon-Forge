use mos6502::asm::assemble;
use mos6502::disasm;
use mos6502::{bits, Cpu, FlatMemory, MemoryBus};

fn cpu_with(program: &[u8]) -> Cpu<FlatMemory> {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, program);
    mem.set_reset_vector(0x8000);
    mem.set_irq_vector(0x9000);
    mem.set_nmi_vector(0x9100);
    let mut cpu = Cpu::new(mem);
    cpu.halt_on_brk = true;
    cpu.reset();
    cpu
}

#[test]
fn bcd_adc_09_plus_01() {
    // SED CLC LDA #$09 ADC #$01 CLD BRK
    let mut cpu = cpu_with(&[0xF8, 0x18, 0xA9, 0x09, 0x69, 0x01, 0xD8, 0x00]);
    for _ in 0..16 {
        if cpu.is_halted() {
            break;
        }
        cpu.step().unwrap();
    }
    assert_eq!(cpu.a, 0x10);
}

#[test]
fn assembler_lda_sta_roundtrip() {
    let src = r#"
        lda #$42
        sta $10
        brk
    "#;
    let bin = assemble(src, 0x8000).unwrap();
    let mut cpu = cpu_with(&bin);
    for _ in 0..16 {
        if cpu.is_halted() {
            break;
        }
        cpu.step().unwrap();
    }
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cpu.bus.peek(0x0010), 0x42);
}

#[test]
fn disasm_lda_imm() {
    let l = disasm::disasm(0xA9, 0x10, 0x00);
    assert_eq!(l.size, 2);
    assert!(l.text.contains("LDA"));
}

#[test]
fn irq_jumps_to_vector_when_i_clear() {
    let mut cpu = cpu_with(&[0x58, 0xEA, 0x00]); // CLI NOP BRK
    cpu.step().unwrap(); // CLI
    cpu.bus.data[0x9000] = 0xEA;
    cpu.bus.data[0x9001] = 0x40; // NOP RTI at IRQ
    cpu.irq();
    assert_eq!(cpu.pc, 0x9000);
    assert!(cpu.p.get(bits::I));
}
