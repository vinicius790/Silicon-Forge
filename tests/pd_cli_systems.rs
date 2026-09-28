//! `nes`, `kim1`, `board`, `bench-suite`, load formats and include-aware coverage.

use std::path::{Path, PathBuf};
use std::process::Command;

use mos6502::{audio, graphics};

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
    let dir = std::env::temp_dir().join(format!("pd_sys_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// NROM with battery RAM: increments $6000 once at reset, writes a red backdrop,
/// starts a pulse tone, then loops.
fn rom(tag: &str) -> PathBuf {
    let code: &[u8] = &[
        0xAD, 0x00, 0x60, 0x18, 0x69, 0x01, 0x8D, 0x00, 0x60, // inc $6000
        0xA9, 0x3F, 0x8D, 0x06, 0x20, 0xA9, 0x00, 0x8D, 0x06, 0x20, // $2006 = $3F00
        0xA9, 0x16, 0x8D, 0x07, 0x20, // backdrop $16
        0xA9, 0x01, 0x8D, 0x15, 0x40, 0xA9, 0xBF, 0x8D, 0x00, 0x40, // pulse 1 on
        0xA9, 0xFD, 0x8D, 0x02, 0x40, 0xA9, 0x00, 0x8D, 0x03, 0x40, //
        0x4C, 0x2C, 0xC0, // loop
    ];
    let mut prg = vec![0u8; 0x4000];
    prg[..code.len()].copy_from_slice(code);
    prg[0x3FFA..].copy_from_slice(&[0x2C, 0xC0, 0x00, 0xC0, 0x2C, 0xC0]);
    let mut file = b"NES\x1a\x01\x01\x02\x00".to_vec();
    file.extend_from_slice(&[0; 8]);
    file.extend(prg);
    file.extend(vec![0u8; 0x2000]);
    let p = tmp(&format!("{tag}.nes"));
    std::fs::write(&p, file).unwrap();
    p
}

#[test]
fn nes_png_matches_ppm_and_wav_has_the_tone() {
    let r = rom("frame");
    let (png, ppm, wav) = (tmp("f.png"), tmp("f.ppm"), tmp("f.wav"));
    let (code, out, err) = run(&[
        "nes",
        s(&r),
        "--frames",
        "3",
        "--png",
        s(&png),
        "--ppm",
        s(&ppm),
        "--wav",
        s(&wav),
        "--rate",
        "22050",
        "--bits",
        "8",
        "--channels",
        "2",
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("3 quadros"));
    let ppm = std::fs::read(&ppm).unwrap();
    assert!(ppm.starts_with(b"P6\n256 240\n255\n"));
    let (w, h, rgb) = graphics::decode_png_rgb(&std::fs::read(&png).unwrap()).unwrap();
    assert_eq!((w, h), (256, 240));
    assert_eq!(rgb, ppm[15..]);
    let (fmt, samples) = audio::parse_wav_any(&std::fs::read(&wav).unwrap()).unwrap();
    assert_eq!((fmt.sample_rate, fmt.channels, fmt.bits), (22050, 2, 8));
    // 3 NTSC frames = 89342 CPU cycles ~ 1100 frames of audio, stereo.
    let frames = samples.len() / 2;
    assert!((1090..=1110).contains(&frames), "{frames}");
    assert!(
        samples.chunks(2).all(|c| c[0] == c[1]),
        "both channels equal"
    );
    let distinct: std::collections::BTreeSet<i16> = samples.iter().copied().collect();
    assert!(distinct.len() >= 2, "pulse channel audible");
}

#[test]
fn nes_sav_persists_across_runs() {
    let r = rom("sav");
    let sav = tmp("game.sav");
    let _ = std::fs::remove_file(&sav);
    let (code, out, _) = run(&["nes", s(&r), "--frames", "1", "--sav", s(&sav)]);
    assert_eq!(code, 0);
    assert!(out.contains("bateria salva"));
    assert!(!out.contains("restaurada"));
    assert_eq!(std::fs::read(&sav).unwrap()[0], 1);
    let (_, out, _) = run(&["nes", s(&r), "--frames", "1", "--sav", s(&sav)]);
    assert!(out.contains("bateria restaurada"));
    let data = std::fs::read(&sav).unwrap();
    assert_eq!(data.len(), 0x2000);
    assert_eq!(data[0], 2, "incremented from the restored value");
    std::fs::write(&sav, [0u8; 3]).unwrap();
    let (code, _, err) = run(&["nes", s(&r), "--frames", "1", "--sav", s(&sav)]);
    assert_eq!(code, 1, "wrong-size save");
    assert!(err.contains(s(&sav)));
}

#[test]
fn nes_regions_and_errors() {
    let r = rom("reg");
    let (_, ntsc, _) = run(&["nes", s(&r), "--frames", "2"]);
    let (_, pal, _) = run(&["nes", s(&r), "--frames", "2", "--region", "pal"]);
    let cycles = |o: &str| -> u64 {
        o.split(", ")
            .nth(1)
            .unwrap()
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap()
    };
    // PAL frames are longer in CPU cycles (312 lines at 3.2 dots per cycle).
    assert!(cycles(&pal) > cycles(&ntsc) + 3000, "{ntsc} {pal}");
    assert_eq!(run(&["nes", s(&r), "--region", "secam"]).0, 2);
    assert_eq!(run(&["nes", s(&r), "--frames", "x"]).0, 2);
    let wav = tmp("bad.wav");
    assert_eq!(run(&["nes", s(&r), "--wav", s(&wav), "--bits", "12"]).0, 2);
    assert_eq!(
        run(&["nes", s(&r), "--wav", s(&wav), "--channels", "6"]).0,
        2
    );
    let junk = tmp("junk.nes");
    std::fs::write(&junk, b"not a rom").unwrap();
    assert_eq!(run(&["nes", s(&junk)]).0, 1);
    assert_eq!(run(&["nes"]).0, 2);
}

#[test]
fn kim1_keys_load_and_go() {
    let (code, out, _) = run(&["kim1", "--keys", "AD 0200 DA A9", "-n", "5000"]);
    assert_eq!(code, 0);
    assert!(out.contains("display: [0200A9]"), "{out}");
    let prog = tmp("k.bin");
    // LDA #$42 / STA $10 / JMP $0204
    std::fs::write(&prog, [0xA9, 0x42, 0x85, 0x10, 0x4C, 0x04, 0x02]).unwrap();
    let (code, out, _) = run(&["kim1", "--load", s(&prog), "--go", "$0200", "-n", "100"]);
    assert_eq!(code, 0);
    assert!(out.contains("7 bytes carregados em $0200"));
    assert!(out.contains("PC=$0204 A=$42"), "{out}");
    let (code, _, err) = run(&["kim1", "--load", s(&prog), "--at", "$1C00"]);
    assert_eq!(code, 1);
    assert!(err.contains("não é RAM"));
    assert_eq!(run(&["kim1", "--keys", "AD XYZ"]).0, 2);
    assert_eq!(run(&["kim1", "--at", "$0200"]).0, 2);
    assert_eq!(run(&["kim1", "extra"]).0, 2);
}

fn example(name: &str) -> String {
    format!("{}/examples/{name}", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn board_runs_via_clock_example() {
    let (code, out, err) = run(&[
        "board",
        &example("via_clock.asm"),
        "--cycles",
        "100500",
        "--dump",
        "0:2",
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("VIA: ORB=$0A"), "{out}");
    assert!(out.contains("0000: 00 0A"), "ten seconds, ticks back at 0");
}

#[test]
fn board_runs_acia_echo_example() {
    let (code, out, _) = run(&[
        "board",
        &example("acia_irq_echo.asm"),
        "--cycles",
        "5000",
        "--input",
        "Hi!",
        "--dump",
        "$0200:3",
    ]);
    assert_eq!(code, 0);
    assert!(out.contains("ACIA tx: \"Hi!\""), "{out}");
    assert!(out.contains("0200: 48 69 21"));
    let (code, _, err) = run(&[
        "board",
        &example("acia_irq_echo.asm"),
        "--cycles",
        "300",
        "--input",
        "long text",
    ]);
    assert_eq!(code, 1);
    assert!(err.contains("não couberam"));
    assert_eq!(
        run(&["board", &example("via_clock.asm"), "--dump", "12"]).0,
        2
    );
    assert_eq!(run(&["board"]).0, 2);
}

#[test]
fn run_and_hex_handle_prg_and_xex() {
    let src = tmp("p.asm");
    std::fs::write(&src, "  .org $0900\n  LDA #7\n  BRK\n").unwrap();
    let prg = tmp("p.prg");
    let xex = tmp("p.xex");
    assert_eq!(
        run(&["assemble", s(&src), "-o", s(&prg), "--format", "prg"]).0,
        0
    );
    assert_eq!(std::fs::read(&prg).unwrap(), [0x00, 0x09, 0xA9, 0x07, 0x00]);
    assert_eq!(
        run(&["assemble", s(&src), "-o", s(&xex), "--format", "atari"]).0,
        0
    );
    let x = mos6502::hexfmt::from_xex(&std::fs::read(&xex).unwrap()).unwrap();
    assert_eq!(x.run, Some(0x0900));
    let (code, out, _) = run(&["run", s(&prg)]);
    assert_eq!(code, 0);
    assert!(out.contains("PC=$0902 A=$07"), "{out}");
    let (code, out, _) = run(&["run", s(&xex)]);
    assert_eq!(code, 0);
    assert!(out.contains("PC=$0902 A=$07"), "{out}");
    // Explicit --from on a file without the extension.
    let raw = tmp("p.data");
    std::fs::copy(&prg, &raw).unwrap();
    let (_, out, _) = run(&["run", s(&raw), "--from", "prg"]);
    assert!(out.contains("PC=$0902"), "{out}");
    let back = tmp("p.mos");
    assert_eq!(run(&["hex", s(&xex), "-o", s(&back), "--to", "mos"]).0, 0);
    assert!(std::fs::read_to_string(&back).unwrap().starts_with(';'));
    assert_eq!(
        run(&["hex", s(&prg), "-o", s(&back), "--from", "bogus"]).0,
        2
    );
    assert_eq!(
        run(&["hex", s(&raw), "-o", s(&back), "--from", "ihex"]).0,
        1
    );
}

#[test]
fn coverage_follows_includes_from_disk_and_macros() {
    let dir = tmp("covdir");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("lib.inc"),
        "helper:\n  LDA #2\n  RTS\nunused:\n  RTS\n",
    )
    .unwrap();
    let main = dir.join("main.asm");
    std::fs::write(
        &main,
        ".macro two\n  INX\n  INX\n.endmacro\n  two\n  JSR helper\n  BRK\n.include \"lib.inc\"\n",
    )
    .unwrap();
    let lcov = dir.join("cov.info");
    let (code, out, err) = run(&["coverage", s(&main), "--lcov", s(&lcov)]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("4/6 linhas"), "{out}");
    assert!(out.contains("não executada: lib.inc:5"));
    let info = std::fs::read_to_string(&lcov).unwrap();
    assert!(info.contains(&format!("SF:{}\nDA:5,1\nDA:6,1\nDA:7,0\n", s(&main))));
    assert!(info.contains("SF:lib.inc\nDA:2,1\nDA:3,1\nDA:5,0\nLF:3\nLH:2\n"));
    std::fs::write(&main, ".include \"missing.inc\"\n").unwrap();
    assert_eq!(run(&["coverage", s(&main)]).0, 1);
}

#[test]
fn bench_suite_baseline_and_regressions() {
    let base = tmp("base.txt");
    let (code, out, _) = run(&["bench-suite", "--seconds", "0.02", "--write", s(&base)]);
    assert_eq!(code, 0);
    assert!(out.contains("alu"));
    let text = std::fs::read_to_string(&base).unwrap();
    // Same build, generous tolerance: passes.
    let (code, out, _) = run(&[
        "bench-suite",
        "--seconds",
        "0.02",
        "--baseline",
        s(&base),
        "--tolerance",
        "95",
    ]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("sem regressões"));
    // A baseline claiming a different cycle fingerprint is a regression.
    let tampered = tmp("tampered.txt");
    let changed: String = text
        .lines()
        .map(|l| {
            if l.starts_with("cmos ") {
                "cmos 0.001 1\n".to_string()
            } else {
                format!("{l}\n")
            }
        })
        .collect();
    std::fs::write(&tampered, changed).unwrap();
    let (code, _, err) = run(&[
        "bench-suite",
        "--seconds",
        "0.02",
        "--baseline",
        s(&tampered),
    ]);
    assert_eq!(code, 1);
    assert!(err.contains("regressão: cmos"), "{err}");
    // Impossible speed in the baseline: slower than it by far.
    let fast = text
        .lines()
        .map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            if f[0] == "alu" {
                format!("alu 1000000 {}\n", f[2])
            } else {
                format!("{l}\n")
            }
        })
        .collect::<String>();
    std::fs::write(&tampered, fast).unwrap();
    let (code, _, err) = run(&[
        "bench-suite",
        "--seconds",
        "0.02",
        "--baseline",
        s(&tampered),
    ]);
    assert_eq!(code, 1);
    assert!(err.contains("alu:") && err.contains("queda"), "{err}");
    assert_eq!(run(&["bench-suite", "--seconds", "-1"]).0, 2);
    assert_eq!(run(&["bench-suite", "x"]).0, 2);
    std::fs::write(&tampered, "junk\n").unwrap();
    assert_eq!(run(&["bench-suite", "--baseline", s(&tampered)]).0, 1);
}
