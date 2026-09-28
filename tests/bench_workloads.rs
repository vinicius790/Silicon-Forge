//! Benchmark workloads have exact, reproducible cycle counts.

use mos6502::bench::{run_benchmark, run_timed, run_workload, Workload};

#[test]
fn cycle_counts_are_exact_and_both_steppers_agree() {
    // One NMOS pass over X = 0..255: 255 iterations of 2+2+3+3+2+3, one with BNE not
    // taken (2) then JMP (3): 6 instructions per iteration, plus the JMP.
    let per_pass_insns = 256 * 6 + 1;
    let per_pass_cycles = 256 * 15 - 1 + 3;
    assert_eq!(
        run_workload(Workload::Alu, per_pass_insns).unwrap(),
        per_pass_cycles
    );
    assert_eq!(
        run_workload(Workload::Alu, 3 * per_pass_insns).unwrap(),
        3 * per_pass_cycles
    );
    for n in [0u64, 1, 7, 1000, 4096] {
        assert_eq!(
            run_workload(Workload::Alu, n).unwrap(),
            run_workload(Workload::CycleStepped, n).unwrap(),
            "{n}"
        );
    }
    // 65C02: INC A 2, STA zp 3, INX 2, BRA 3 (no page cross).
    assert_eq!(run_workload(Workload::Cmos, 4 * 100).unwrap(), 10 * 100);
}

#[test]
fn timed_runs_reject_bad_durations_and_report() {
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(run_benchmark(bad).is_err());
    }
    for w in Workload::ALL {
        let r = run_timed(w, 0.01).unwrap();
        assert!(r.instructions >= 10_000 && r.cycles >= 2 * r.instructions);
        assert!(r.report().contains(&format!("{w:?}")));
    }
}
