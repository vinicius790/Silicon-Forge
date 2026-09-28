use mos6502::bus::{NamedMapError, NamedMemory};
use mos6502::debugger::{Debugger, FlagEdge};
use mos6502::disasm::{disasm, disasm_at, official_layout};
use mos6502::link::{assemble_object, link};
use mos6502::opcodes::documented_opcodes;
use mos6502::{assemble, bits, Cpu, FlatMemory, StopReason};

#[test]
fn official_nmos_disassembly_round_trips_mnemonics() {
    for op in documented_opcodes() {
        let (name, mode, len) = official_layout(op.code).expect(op.mnemonic);
        assert_eq!(name, op.mnemonic);
        assert_eq!(mode, op.mode);
        assert_eq!(len, op.mode.size());
        let line = disasm(op.code, 0x10, 0x20);
        assert_eq!(line.size, len, "{}", op.mnemonic);
        assert!(
            line.text.starts_with(op.mnemonic),
            "{} -> {}",
            op.mnemonic,
            line.text
        );
    }
    assert!(
        official_layout(0xA7).is_none(),
        "SLO is not an official NMOS opcode"
    );

    let bytes = assemble(
        "lda #$2A\nsta $80\ninx\nbne done\nnop\ndone:\n  rts\n",
        0x8000,
    )
    .unwrap();
    let mut mem = vec![0u8; 65536];
    mem[0x8000..0x8000 + bytes.len()].copy_from_slice(&bytes);
    let mut pc = 0x8000u16;
    for mnemonic in ["LDA", "STA", "INX", "BNE", "NOP", "RTS"] {
        let line = disasm_at(&mem, pc);
        assert!(line.text.starts_with(mnemonic), "{} at {pc:04X}", line.text);
        let (_, _, len) = official_layout(mem[pc as usize]).unwrap();
        assert_eq!(line.size, len);
        pc = pc.wrapping_add(u16::from(line.size));
    }
    assert_eq!(pc, 0x8000 + bytes.len() as u16);
}

#[test]
fn flag_breakpoint_fires_only_when_n_or_z_becomes_set() {
    let mut dbg = Debugger::default();
    dbg.break_when_flag_becomes_set(FlagEdge::Zero);
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xA9, 0x01, 0xA9, 0x00, 0x00]);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let stop = dbg.run(&mut cpu, 8).unwrap();
    assert_eq!(stop.reason, StopReason::FlagSet);
    assert_eq!(stop.registers.a, 0);
    assert_eq!(stop.registers.pc, 0x8004);
    assert_eq!(stop.instructions, 2);

    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xA9, 0x00, 0xEA]);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    cpu.p.set(bits::Z, true);
    let stop = dbg.run(&mut cpu, 2).unwrap();
    assert_eq!(stop.reason, StopReason::InstructionLimit);
    assert_eq!(stop.instructions, 2);
    assert!(cpu.p.get(bits::Z));

    dbg.break_when_flag_becomes_set(FlagEdge::Negative);
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xA9, 0x80, 0xEA]);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let stop = dbg.run(&mut cpu, 4).unwrap();
    assert_eq!(stop.reason, StopReason::FlagSet);
    assert_eq!(stop.registers.a, 0x80);
    assert_eq!(stop.registers.pc, 0x8002);
    assert_eq!(stop.instructions, 1);

    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xA9, 0x00, 0xEA]);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let stop = dbg.run(&mut cpu, 2).unwrap();
    assert_eq!(stop.reason, StopReason::InstructionLimit);
    assert!(!cpu.p.get(bits::N));
    assert!(cpu.p.get(bits::Z));
}

#[test]
fn named_map_rejects_overlap_and_load_past_end() {
    let mut map = NamedMemory::new();
    map.region("zp", 0x0000, 0x00FF).unwrap();
    let err = map.region("via", 0x0080, 0x008F).unwrap_err();
    assert_eq!(
        err,
        NamedMapError::Overlap {
            name: "via".into(),
            other: "zp".into(),
        }
    );
    map.region("code", 0x8000, 0x8001).unwrap();
    let err = map.load("code", &[0xA9, 0x01, 0x00]).unwrap_err();
    assert_eq!(
        err,
        NamedMapError::PastEnd {
            name: "code".into(),
            length: 3,
            room: 2,
        }
    );
    assert_eq!(map.get(0x8000), 0);
    assert_eq!(map.get(0x8001), 0);
    map.load("code", &[0xA9, 0x01]).unwrap();
    assert_eq!(map.get(0x8000), 0xA9);
    assert_eq!(map.get(0x8001), 0x01);
}

#[test]
fn add_carry_program_links_and_runs() {
    let source = include_str!("../examples/add_carry.asm");
    let module = assemble_object("add", source).unwrap();
    let linked = link(&[module], 0).unwrap();
    assert_eq!(linked.symbols["start"], 0x8000);
    assert_eq!(linked.symbols["lo"], 0x0020);
    assert_eq!(linked.symbols["sum"], 0x0022);
    let mut mem = FlatMemory::new();
    linked.store(&mut mem.data);
    let mut cpu = Cpu::new(mem);
    cpu.pc = linked.symbols["start"];
    cpu.halt_on_brk = true;
    for _ in 0..16 {
        if cpu.is_halted() {
            break;
        }
        cpu.step().unwrap();
    }
    assert!(cpu.is_halted());
    assert_eq!(cpu.bus.data[0x0022], 0x10);
    assert_eq!(cpu.bus.data[0x0023], 1);
    assert_eq!(cpu.a, 1);
}
