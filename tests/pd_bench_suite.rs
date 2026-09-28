//! Benchmark suite: deterministic fingerprints, baseline text and regressions.

use mos6502::bench::*;

fn entry(w: Workload, mips: f64, cycles: u64) -> BaselineEntry {
    BaselineEntry {
        workload: w,
        mips,
        cycles,
    }
}

#[test]
fn fingerprints_are_exact_cycle_counts() {
    // Alu loop: CLC 2, LDA# 2, ADC zp 3, STA zp 3, INX 2, BNE taken 3 = 6 insns/15 cycles
    // (JMP every 256 iterations). 10_000 insns: deterministic, checked twice.
    let a = run_workload(Workload::Alu, FINGERPRINT_INSTRUCTIONS).unwrap();
    assert_eq!(
        a,
        run_workload(Workload::Alu, FINGERPRINT_INSTRUCTIONS).unwrap()
    );
    assert_eq!(
        a,
        run_workload(Workload::CycleStepped, FINGERPRINT_INSTRUCTIONS).unwrap(),
        "instruction and cycle stepping agree"
    );
    // Cmos: INC A 2, STA zp 3, INX 2, BRA 3 = 10 cycles per 4 instructions.
    assert_eq!(run_workload(Workload::Cmos, 10_000).unwrap(), 25_000);
    assert_eq!(run_workload(Workload::Alu, 6).unwrap(), 15);
}

#[test]
fn names_round_trip() {
    for w in Workload::ALL {
        assert_eq!(Workload::from_name(w.name()), Some(w));
    }
    assert_eq!(Workload::from_name("z80"), None);
}

#[test]
fn baseline_text_round_trip() {
    let r = SuiteResult {
        entries: vec![
            entry(Workload::Alu, 12.5, 25_000),
            entry(Workload::Cmos, 30.0, 25_000),
        ],
    };
    let text = r.to_baseline();
    assert!(text.starts_with('#'));
    assert!(text.contains("alu 12.500 25000\n"));
    assert_eq!(SuiteResult::parse_baseline(&text).unwrap(), r);
}

#[test]
fn baseline_parse_errors_name_the_line() {
    for (text, msg) in [
        ("alu 1\n", "linha 1: esperado"),
        ("# x\nz80 1 2\n", "linha 2: carga desconhecida"),
        ("alu -3 2\n", "MIPS inválido"),
        ("alu nan 2\n", "MIPS inválido"),
        ("alu 1 x\n", "ciclos inválidos"),
        ("alu 1 2\nalu 1 2\n", "linha 2: carga repetida"),
    ] {
        let e = SuiteResult::parse_baseline(text).unwrap_err();
        assert!(e.contains(msg), "{text:?}: {e}");
    }
}

#[test]
fn compare_flags_slowdowns_cycle_changes_and_missing() {
    let base = SuiteResult {
        entries: vec![
            entry(Workload::Alu, 100.0, 25_000),
            entry(Workload::Cmos, 100.0, 25_000),
            entry(Workload::CycleStepped, 50.0, 25_000),
        ],
    };
    let now = SuiteResult {
        entries: vec![
            entry(Workload::Alu, 90.0, 25_000),   // 10% drop
            entry(Workload::Cmos, 120.0, 25_001), // faster but timing changed
        ],
    };
    assert_eq!(
        now.compare(&base, 15.0),
        vec![
            Regression::Cycles {
                workload: Workload::Cmos,
                baseline: 25_000,
                measured: 25_001
            },
            Regression::Missing(Workload::CycleStepped),
        ]
    );
    let strict = now.compare(&base, 5.0);
    assert!(matches!(
        strict[0],
        Regression::Slower { workload: Workload::Alu, drop_percent, .. } if (drop_percent - 10.0).abs() < 1e-9
    ));
    assert!(strict[0].to_string().contains("queda de 10.0%"));
    assert_eq!(now.compare(&now, 0.0), vec![]);
}

#[test]
fn run_suite_measures_all_workloads() {
    let r = run_suite(0.01).unwrap();
    assert_eq!(r.entries.len(), 3);
    for e in &r.entries {
        assert!(e.mips > 0.0);
        assert_eq!(
            e.cycles,
            run_workload(e.workload, FINGERPRINT_INSTRUCTIONS).unwrap()
        );
    }
    assert!(r.report().contains("cycle-stepped"));
    assert!(run_suite(0.0).is_err());
}
