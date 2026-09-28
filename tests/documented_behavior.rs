use mos6502::{asm::assemble, bits, Cpu, FlatMemory};
use std::io::Write;
use std::process::{Command, Stdio};

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
fn readme_cli_hello_output_and_assembly_match() {
    let expected = include_bytes!("../examples/hello_lda.bin");
    assert_eq!(
        assemble(include_str!("../examples/hello_lda.asm.txt"), 0x8000).unwrap(),
        expected
    );
    let output = Command::new(env!("CARGO_BIN_EXE_mos6502"))
        .args([
            "--bin",
            "examples/hello_lda.bin",
            "-n",
            "20",
            "--mode",
            "cycles",
            "--dump-mem",
            "0x10:0x01",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        "A=42 X=00 Y=00 SP=FA PC=0000 P=24[nv-bdIzc] CYC=19\nMEM[0010]: 42\n"
    );
}

#[test]
fn cli_rejects_invalid_load_addresses_instead_of_using_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_mos6502"))
        .args([
            "--bin",
            "examples/hello_lda.bin",
            "--load",
            "not-an-address",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Endereço inválido"));
}

#[test]
fn cli_rejects_unknown_cpu_variants_instead_of_falling_back_to_nmos() {
    let output = Command::new(env!("CARGO_BIN_EXE_mos6502"))
        .args(["--bin", "examples/hello_lda.bin", "--variant", "65c99"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Variante inválida"));
}

#[test]
fn assembler_cli_returns_error_when_output_cannot_be_written() {
    let output = Command::new(env!("CARGO_BIN_EXE_mos6502"))
        .args(["asm", "examples/hello_lda.asm.txt", "-o", "."])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Erro ao salvar"));
}

#[test]
fn benchmark_cli_runs_and_rejects_invalid_durations() {
    let valid = Command::new(env!("CARGO_BIN_EXE_mos6502"))
        .args(["bench", "--seconds", "0.01"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(valid.status.success());
    assert!(String::from_utf8_lossy(&valid.stdout).contains("MIPS"));

    let invalid = Command::new(env!("CARGO_BIN_EXE_mos6502"))
        .args(["bench", "--seconds", "NaN"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
}

#[test]
fn mini_basic_cli_runs_numbered_programs() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mos6502"))
        .arg("basic")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"10 LET A=2+3*4\n20 PRINT A\nRUN\nEXIT\n")
        .unwrap();

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("14"));
}

#[test]
fn fantasy_cli_runs_a_binary_and_renders_the_screen() {
    let output = Command::new(env!("CARGO_BIN_EXE_mos6502"))
        .args(["fantasy", "examples/hello_lda.bin", "--cycles", "100"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("+--------------------------------+"));
    assert!(stdout.contains("A=42"));
}

#[test]
fn distributed_binary_examples_reach_documented_results() {
    let mut sum = cpu_with(include_bytes!("../examples/sum_loop.bin"));
    sum.run_cycles(200).unwrap();
    assert!(sum.is_halted());
    assert_eq!(sum.bus.data[0], 15);
    assert_eq!(sum.bus.data[1], 1);
    let mut jump = cpu_with(include_bytes!("../examples/jump_demo.bin"));
    jump.run_instructions(10).unwrap();
    assert!(jump.is_halted());
    assert_eq!(jump.a, 0x7f);
}

#[test]
fn brk_halts_after_stack_and_vector_effects() {
    let mut cpu = cpu_with(&[0x00]);
    assert_eq!(cpu.step().unwrap(), 7);
    assert_eq!(cpu.pc, 0x9000);
    assert_eq!(cpu.sp, 0xfa);
    assert_eq!(&cpu.bus.data[0x1fb..=0x1fd], &[0x34, 0x02, 0x80]);
    assert_eq!(cpu.step().unwrap(), 0);
    assert_eq!(cpu.cycles, 14);
}

#[test]
fn irq_mask_and_nmi_stack_return_are_explicit() {
    let mut cpu = cpu_with(&[0xea]);
    let before = cpu.registers();
    cpu.irq();
    assert_eq!(cpu.registers(), before);
    cpu.bus.data[0x9100] = 0x40; // RTI
    cpu.nmi();
    assert_eq!(cpu.pc, 0x9100);
    assert_eq!(cpu.cycles, 14);
    assert_eq!(&cpu.bus.data[0x1fb..=0x1fd], &[0x24, 0x00, 0x80]);
    cpu.step().unwrap();
    assert_eq!(cpu.pc, 0x8000);
    assert_eq!(cpu.sp, 0xfd);
    assert!(cpu.p.get(bits::I));
}

#[test]
fn nmos_decimal_zero_flag_follows_the_binary_sum() {
    // $99 + $01 in BCD is $00 with carry. NMOS Z follows the binary $9A (clear) and
    // N the intermediate $A0 (set).
    let mut cpu = cpu_with(&[0xf8, 0x18, 0xa9, 0x99, 0x69, 0x01]);
    cpu.run_instructions(4).unwrap();
    assert_eq!(cpu.a, 0);
    assert!(cpu.p.get(bits::C));
    assert!(!cpu.p.get(bits::Z));
    assert!(cpu.p.get(bits::N));
}

#[test]
fn labels_resolve_forward_backward_and_case_insensitively() {
    let src = "JMP start\nloop: LDA #$42\nSTA $10\nBRK\nstart: BNE LOOP";
    assert_eq!(
        assemble(src, 0x8000).unwrap(),
        [0x4c, 0x08, 0x80, 0xa9, 0x42, 0x85, 0x10, 0x00, 0xd0, 0xf9]
    );
}

#[test]
fn symbolic_load_store_sizes_are_stable_in_zero_page() {
    let src = "LDA target\nSTA target,X\ntarget: BRK";
    assert_eq!(
        assemble(src, 0x0010).unwrap(),
        [0xad, 0x16, 0x00, 0x9d, 0x16, 0x00, 0x00]
    );
    assert_eq!(
        assemble("LDA $10\nSTA $10,X", 0x8000).unwrap(),
        [0xa5, 0x10, 0x95, 0x10]
    );
}

#[test]
fn branches_use_actual_origin_and_wrapping_pc() {
    assert_eq!(assemble("BNE $8000", 0x8000).unwrap(), [0xd0, 0xfe]);
    assert_eq!(assemble("BNE $0000", 0xfffe).unwrap(), [0xd0, 0x00]);
    assert_eq!(assemble("BNE $8081", 0x8000).unwrap(), [0xd0, 0x7f]);
    assert_eq!(assemble("BNE $7F82", 0x8000).unwrap(), [0xd0, 0x80]);
    assert!(assemble("BNE $8082", 0x8000).is_err());
    assert!(assemble("BNE $7F81", 0x8000).is_err());
    assert!(assemble("BNE $8100", 0x8000).is_err());
    assert!(assemble("BNE missing", 0x8000).is_err());
}

#[test]
fn assembler_rejects_invalid_symbols_and_truncated_operands() {
    for src in [
        "same: NOP\nSAME: BRK",
        "1bad: BRK",
        "LDA missing",
        "LDA #$100",
        "ADC #$100",
        "BRK $10",
    ] {
        assert!(assemble(src, 0x8000).is_err(), "accepted {src}");
    }
}

#[test]
fn explicit_hex_literal_is_not_shadowed_by_label() {
    assert_eq!(
        assemble("face: LDA $FACE\nJMP face", 0x8000).unwrap(),
        [0xad, 0xce, 0xfa, 0x4c, 0x00, 0x80]
    );
}
