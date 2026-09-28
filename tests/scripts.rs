//! The bash tooling in scripts/ runs against this checkout and fails when it should.

use std::path::Path;
use std::process::Command;

fn script(name: &str, envs: &[(&str, &str)], args: &[&str]) -> (bool, String) {
    let root = env!("CARGO_MANIFEST_DIR");
    let out = Command::new("bash")
        .arg(Path::new(root).join("scripts").join(name))
        .args(args)
        .envs(envs.iter().copied())
        .output()
        .expect("bash");
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.success(), text)
}

const BIN: &str = env!("CARGO_BIN_EXE_mos6502");

#[test]
fn every_example_produces_its_expected_memory() {
    let (ok, text) = script("examples.sh", &[("BIN", BIN)], &[]);
    assert!(ok, "{text}");
    assert!(text.contains(", 0 falhas"), "{text}");
}

#[test]
fn a_wrong_expectation_fails_the_examples_script() {
    let dir = std::env::temp_dir().join(format!("mos6502-manifest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = dir.join("bad.tsv");
    std::fs::write(
        &manifest,
        "examples/hello_lda.bin\tbin\tnmos\t20\t0x10:1\tMEM[0010]: 43\n",
    )
    .unwrap();
    let (ok, text) = script(
        "examples.sh",
        &[("BIN", BIN), ("MANIFEST", manifest.to_str().unwrap())],
        &[],
    );
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(!ok);
    assert!(text.contains("obtido 'MEM[0010]: 42'"), "{text}");
}

#[test]
fn versions_agree_and_the_report_counts() {
    let (ok, text) = script("check_versions.sh", &[("BIN", BIN)], &[]);
    assert!(ok, "{text}");
    assert!(text.contains(env!("CARGO_PKG_VERSION")));
    let (ok, text) = script("report.sh", &[], &[]);
    assert!(ok, "{text}");
    assert!(
        text.contains("src/cpu.rs") && text.contains("funções de teste: "),
        "{text}"
    );
}

#[test]
fn package_excludes_target_and_git() {
    let dir = std::env::temp_dir().join(format!("mos6502-pkg-{}", std::process::id()));
    let (ok, text) = script("package.sh", &[], &[dir.to_str().unwrap()]);
    assert!(ok, "{text}");
    let archive = text.trim().lines().last().unwrap().to_string();
    assert!(archive.ends_with(&format!("mos6502-{}.tgz", env!("CARGO_PKG_VERSION"))));
    let list = Command::new("tar")
        .args(["-tzf", &archive])
        .output()
        .unwrap();
    let list = String::from_utf8_lossy(&list.stdout);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(list.lines().any(|l| l.ends_with("/src/apu.rs")));
    assert!(!list
        .lines()
        .any(|l| l.contains("/target/") || l.contains("/.git/")));
}

#[test]
fn powershell_scripts_are_utf8_with_bom_and_have_a_bash_twin_or_reason() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts");
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "ps1") {
            let bytes = std::fs::read(&path).unwrap();
            assert!(
                bytes.starts_with(&[0xEF, 0xBB, 0xBF]),
                "{path:?} lacks the BOM"
            );
            assert!(
                std::str::from_utf8(&bytes[3..]).is_ok(),
                "{path:?} is not UTF-8"
            );
            let stem = path.file_stem().unwrap().to_str().unwrap();
            assert!(
                stem == "mirror" || dir.join(format!("{stem}.sh")).exists(),
                "{stem}"
            );
            count += 1;
        }
    }
    assert_eq!(count, 6);
}
