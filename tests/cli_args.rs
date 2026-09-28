//! CLI argument validation: every malformed invocation exits 2 with a message, and
//! the valid forms still run.

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

fn program(tag: &str) -> String {
    let path = std::env::temp_dir().join(format!("cli_args_{tag}_{}.bin", std::process::id()));
    // LDA #$42 / STA $10 / BRK
    std::fs::write(&path, [0xA9, 0x42, 0x85, 0x10, 0x00]).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn malformed_invocations_exit_2() {
    let bin = program("bad");
    let cases: &[(&[&str], &str)] = &[
        (&["--bin"], "--bin exige um valor"),
        (
            &["--bin", &bin, "--frobnicate"],
            "argumento desconhecido: --frobnicate",
        ),
        (&["--bin", &bin, "stray"], "argumento desconhecido: stray"),
        (&["--bin", &bin, "-n"], "-n exige um valor"),
        (&["--bin", &bin, "-n", "-3"], "inteiro não negativo"),
        (&["--bin", &bin, "-n", "lots"], "inteiro não negativo"),
        (&["--bin", &bin, "--mode", "fast"], "insns ou cycles"),
        (&["--bin", &bin, "--load", "zz"], "Endereço inválido"),
        (&["--bin", &bin, "--load"], "--load exige um valor"),
        (&["--bin", &bin, "--dump-mem", "10"], "ADDR:LEN"),
        (&["--bin", &bin, "--dump-mem", "10:qq"], "Endereço inválido"),
        (&["--bin", &bin, "--variant"], "--variant exige"),
        (&["disasm", &bin, "--count", "x"], "inteiro não negativo"),
        (&["disasm", &bin, "--start"], "--start exige"),
        (
            &["disasm", &bin, "--what"],
            "argumento desconhecido: --what",
        ),
        (
            &["asm", "examples/fibonacci.asm", "-o"],
            "-o exige um valor",
        ),
    ];
    for (args, needle) in cases {
        let (code, _, err) = run(args);
        assert_eq!(code, 2, "{args:?}: {err}");
        assert!(err.contains(needle), "{args:?}: {err}");
    }
    std::fs::remove_file(bin).unwrap();
}

#[test]
fn valid_forms_run() {
    let bin = program("ok");
    let (code, out, err) = run(&[
        "--bin",
        &bin,
        "--mode",
        "insns",
        "-n",
        "2",
        "--dump-mem",
        "$10:1",
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("MEM[0010]: 42"), "{out}");
    let (code, out, _) = run(&["--bin", &bin, "--mode", "cycles", "-n", "0"]);
    assert_eq!(code, 0);
    assert!(!out.is_empty());
    let (code, out, _) = run(&["disasm", &bin, "--count", "2", "--start", "$0400"]);
    assert_eq!(code, 0);
    assert_eq!(out.lines().count(), 2, "{out}");
    assert!(out.starts_with("$0400:"), "{out}");
    std::fs::remove_file(bin).unwrap();
}

#[test]
fn ines_inspects_a_built_header() {
    let path = std::env::temp_dir().join(format!("cli_ines_{}.nes", std::process::id()));
    let mut d = b"NES\x1A\x02\x01\x21\x08\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
    d.extend(vec![0; 0x8000 + 0x2000]);
    std::fs::write(&path, &d).unwrap();
    let p = path.to_string_lossy().into_owned();
    let (code, out, err) = run(&["ines", &p]);
    assert_eq!(code, 0, "{err}");
    for needle in [
        "NES 2.0",
        "mapper: 2 ",
        "PRG: 32768",
        "CHR: 8192",
        "Vertical",
        "mapper suportado",
    ] {
        assert!(out.contains(needle), "{needle}: {out}");
    }
    std::fs::write(&path, &d[..100]).unwrap();
    let (code, _, err) = run(&["ines", &p]);
    assert_eq!(code, 1);
    assert!(err.contains("truncado"), "{err}");
    assert_eq!(run(&["ines"]).0, 2);
    assert_eq!(run(&["ines", &p, "x"]).0, 2);
    std::fs::remove_file(path).unwrap();
}
