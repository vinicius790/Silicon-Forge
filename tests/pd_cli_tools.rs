//! Toolchain subcommands: outputs and exit codes (0 ok, 1 bad input, 2 bad usage).

use std::path::PathBuf;
use std::process::Command;

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_mos6502"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pd_cli_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn s(p: &std::path::Path) -> &str {
    p.to_str().unwrap()
}

const PROG: &str =
    "start:\n  LDX #3\nloop:\n  JSR sub\n  DEX\n  BNE loop\n  BRK\nsub:\n  LDA #1\n  RTS\n";

fn source(tag: &str) -> PathBuf {
    let p = tmp(&format!("{tag}.asm"));
    std::fs::write(&p, PROG).unwrap();
    p
}

#[test]
fn assemble_in_every_format() {
    let src = source("fmt");
    let bin = tmp("fmt.bin");
    let (code, out, _) = run(&["assemble", s(&src), "-o", s(&bin)]);
    assert_eq!(code, 0);
    assert!(out.contains("12 bytes em $8000"));
    let bytes = std::fs::read(&bin).unwrap();
    assert_eq!(&bytes[..3], &[0xA2, 0x03, 0x20]);
    for (fmt, lead) in [("ihex", ':'), ("srec", 'S'), ("mos", ';')] {
        let out = tmp(&format!("fmt.{fmt}"));
        let (code, _, _) = run(&["assemble", s(&src), "-o", s(&out), "--format", fmt]);
        assert_eq!(code, 0);
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.starts_with(lead), "{fmt}");
        let img = mos6502::hexfmt::read_any(&text).unwrap();
        assert_eq!((img.origin, img.bytes), (0x8000, bytes.clone()));
    }
    let sym = tmp("fmt.sym");
    assert_eq!(
        run(&["assemble", s(&src), "-o", s(&bin), "--sym", s(&sym)]).0,
        0
    );
    assert!(std::fs::read_to_string(&sym).unwrap().contains("sub"));
}

#[test]
fn assemble_errors_have_exit_codes() {
    let bad = tmp("bad.asm");
    std::fs::write(&bad, "  LDA #\n").unwrap();
    let (code, _, err) = run(&["assemble", s(&bad), "-o", s(&tmp("bad.bin"))]);
    assert_eq!(code, 1);
    assert!(err.starts_with("mos6502 assemble: "));
    let (code, _, err) = run(&["assemble", "/nonexistent/x.asm", "-o", "y"]);
    assert_eq!(code, 1);
    assert!(err.contains("não foi possível ler"));
    assert_eq!(run(&["assemble", s(&source("e"))]).0, 2);
    let (code, _, err) = run(&["assemble", "a", "b", "-o", "c"]);
    assert_eq!(code, 2);
    assert!(err.contains("argumento inesperado: b"));
    let (code, _, err) = run(&["assemble", "a", "--wat"]);
    assert_eq!(code, 2);
    assert!(err.contains("opção desconhecida: --wat"));
    let (code, _, _) = run(&["assemble", "a", "-o", "b", "--format", "elf"]);
    assert_eq!(code, 2);
    let (code, _, err) = run(&["assemble", "a", "-o"]);
    assert_eq!(code, 2);
    assert!(err.contains("-o exige um valor"));
}

#[test]
fn run_stops_at_brk_and_reports_registers() {
    let src = source("run");
    let hex = tmp("run.mos");
    run(&["assemble", s(&src), "-o", s(&hex), "--format", "mos"]);
    let (code, out, _) = run(&["run", s(&hex)]);
    assert_eq!(code, 0);
    assert!(out.contains("16 instruções; PC=$8008 A=$01 X=$00"), "{out}");
    assert!(out.contains("ciclos=58"));
    let (code, _, err) = run(&["run", s(&hex), "-n", "3"]);
    assert_eq!(code, 1);
    assert!(err.contains("limite de 3 instruções"));
    assert_eq!(run(&["run", s(&hex), "-n", "x"]).0, 2);
    assert_eq!(run(&["run", s(&hex), "--variant", "z80"]).0, 2);
    assert_eq!(run(&["run", s(&hex), "--folded", "f"]).0, 2);
    assert_eq!(run(&["run", s(&hex), "--load", "$1G"]).0, 2);
}

#[test]
fn run_raw_binary_at_load_address() {
    let bin = tmp("raw.bin");
    // LDA #$42 / STA $10 / BRK
    std::fs::write(&bin, [0xA9, 0x42, 0x85, 0x10, 0x00]).unwrap();
    let (code, out, _) = run(&["run", s(&bin), "--load", "0x0600"]);
    assert_eq!(code, 0);
    assert!(out.contains("PC=$0604 A=$42"), "{out}");
    let big = tmp("big.bin");
    std::fs::write(&big, vec![0xEA; 0x100]).unwrap();
    let (code, _, err) = run(&["run", s(&big), "--load", "$FFF0"]);
    assert_eq!(code, 1);
    assert!(err.contains("passam de $FFFF"));
}

#[test]
fn trace_prints_one_line_per_instruction() {
    let src = source("trace");
    let bin = tmp("trace.ihex");
    run(&["assemble", s(&src), "-o", s(&bin), "--format", "ihex"]);
    let (code, out, _) = run(&["trace", s(&bin)]);
    assert_eq!(code, 0);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 17);
    assert!(lines[0].starts_with("8000  A2 03"));
    assert!(lines[0].contains("LDX #$03"));
    assert!(lines[1].contains("X:03"));
    assert!(lines[1].contains("CYC:2"));
}

#[test]
fn profile_writes_folded_and_dot() {
    let src = source("prof");
    let bin = tmp("prof.bin");
    let sym = tmp("prof.sym");
    run(&["assemble", s(&src), "-o", s(&bin), "--sym", s(&sym)]);
    let folded = tmp("prof.folded");
    let dot = tmp("prof.dot");
    let (code, out, err) = run(&[
        "profile",
        s(&bin),
        "--sym",
        s(&sym),
        "--folded",
        s(&folded),
        "--dot",
        s(&dot),
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("total 58 ciclos"));
    assert_eq!(
        std::fs::read_to_string(&folded).unwrap(),
        "top 34\ntop;sub 24\n"
    );
    assert!(std::fs::read_to_string(&dot)
        .unwrap()
        .contains("top -> r8009 [label=\"3x 24\"]"));
}

#[test]
fn coverage_reports_unexecuted_lines_and_lcov() {
    let src = tmp("cov.asm");
    std::fs::write(
        &src,
        "  LDA #0\n  BEQ done ; always\n  LDA #9\ndone:\n  BRK\n",
    )
    .unwrap();
    let lcov = tmp("cov.info");
    let (code, out, _) = run(&["coverage", s(&src), "--lcov", s(&lcov)]);
    assert_eq!(code, 0);
    assert!(out.contains("2/4 linhas executadas"), "{out}");
    assert!(out.contains(&format!("não executada: {}:3", s(&src))));
    let info = std::fs::read_to_string(&lcov).unwrap();
    assert!(info.contains("DA:3,0\n"));
    assert!(info.contains("DA:2,1\n"));
    assert!(info.contains("LF:4\nLH:2\n"));
}

#[test]
fn hex_converts_between_formats() {
    let raw = tmp("conv.bin");
    std::fs::write(&raw, [1, 2, 3, 4]).unwrap();
    let mos = tmp("conv.mos");
    let (code, _, _) = run(&[
        "hex",
        s(&raw),
        "-o",
        s(&mos),
        "--to",
        "mos",
        "--load",
        "$0200",
    ]);
    assert_eq!(code, 0);
    assert_eq!(
        std::fs::read_to_string(&mos).unwrap(),
        ";040200010203040010\n;0000010001\n"
    );
    let srec = tmp("conv.s19");
    assert_eq!(run(&["hex", s(&mos), "-o", s(&srec), "--to", "srec"]).0, 0);
    let back = tmp("conv.out");
    assert_eq!(run(&["hex", s(&srec), "-o", s(&back)]).0, 0);
    assert_eq!(std::fs::read(&back).unwrap(), vec![1, 2, 3, 4]);
    let broken = tmp("broken.mos");
    std::fs::write(&broken, ";0402000102030400FF\n").unwrap();
    let (code, _, err) = run(&["hex", s(&broken), "-o", s(&back)]);
    assert_eq!(code, 1);
    assert!(err.contains("linha 1"));
}

#[test]
fn link_two_modules() {
    let a = tmp("a.asm");
    let b = tmp("b.asm");
    std::fs::write(
        &a,
        ".section code\n.org $8000\n.global start\nstart:\n  JSR helper\n  BRK\n",
    )
    .unwrap();
    std::fs::write(
        &b,
        ".section lib\n.global helper\nhelper:\n  LDA #7\n  RTS\n",
    )
    .unwrap();
    let out = tmp("ab.bin");
    let map = tmp("ab.map");
    let (code, stdout, err) = run(&["link", s(&a), s(&b), "-o", s(&out), "--map", s(&map)]);
    assert_eq!(code, 0, "{err}");
    assert!(stdout.contains("2 objetos"));
    let img = std::fs::read(&out).unwrap();
    // JSR helper, BRK, then the relocated helper right after: $8004.
    assert_eq!(img, vec![0x20, 0x04, 0x80, 0x00, 0xA9, 0x07, 0x60]);
    assert!(std::fs::read_to_string(&map).unwrap().contains("helper"));
    assert_eq!(run(&["link", "-o", s(&out)]).0, 2);
    let (code, _, _) = run(&["link", s(&a), "-o", s(&out)]);
    assert_eq!(code, 1, "undefined helper");
}

#[test]
fn pack_unpack_round_trip_and_corruption() {
    let input = tmp("snap.json");
    std::fs::write(
        &input,
        "{\"ram\":\"".to_string() + &"00".repeat(4000) + "\"}",
    )
    .unwrap();
    let packed = tmp("snap.m65z");
    let restored = tmp("snap.out");
    assert_eq!(run(&["pack", s(&input), "-o", s(&packed)]).0, 0);
    assert!(std::fs::metadata(&packed).unwrap().len() < 1000);
    assert_eq!(run(&["unpack", s(&packed), "-o", s(&restored)]).0, 0);
    assert_eq!(
        std::fs::read(&input).unwrap(),
        std::fs::read(&restored).unwrap()
    );
    let mut bytes = std::fs::read(&packed).unwrap();
    bytes[11] ^= 0xFF;
    std::fs::write(&packed, bytes).unwrap();
    let (code, _, err) = run(&["unpack", s(&packed), "-o", s(&restored)]);
    assert_eq!(code, 1);
    assert!(err.contains("CRC-32"));
}

#[test]
fn help_lists_tools() {
    let (_, _, err) = run(&["--help"]);
    assert!(err.contains("coverage <SRC.asm>"));
    assert!(err.contains("profile <PROG>"));
}
