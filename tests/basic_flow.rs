//! Mini BASIC control flow and comparisons.

use mos6502::systems::basic::BasicSystem;

fn run(lines: &[&str]) -> Result<Vec<String>, String> {
    let mut b = BasicSystem::new();
    for l in lines {
        b.execute_line(l).unwrap();
    }
    b.execute_line("RUN")
}

#[test]
fn every_comparison_over_a_grid_of_values() {
    let mut b = BasicSystem::new();
    for x in -3..=3i32 {
        for y in -3..=3i32 {
            for (op, want) in [
                ("=", x == y),
                ("<>", x != y),
                ("<", x < y),
                (">", x > y),
                ("<=", x <= y),
                (">=", x >= y),
            ] {
                let out = b.execute_line(&format!("PRINT {x} {op} {y}")).unwrap();
                assert_eq!(out, [if want { "1" } else { "0" }], "{x} {op} {y}");
            }
        }
    }
}

#[test]
fn for_next_goto_and_if_then() {
    let sum = run(&[
        "10 S=0",
        "20 FOR I=1 TO 100",
        "30 S=S+I",
        "40 NEXT I",
        "50 PRINT S",
    ])
    .unwrap();
    assert_eq!(sum, ["5050"]);
    let down = run(&["10 FOR I=10 TO 1 STEP -3", "20 PRINT I", "30 NEXT"]).unwrap();
    assert_eq!(down, ["10", "7", "4", "1"]);
    let nested = run(&[
        "10 FOR A=1 TO 3",
        "20 FOR B=1 TO A",
        "30 PRINT A*10+B",
        "40 NEXT B",
        "50 NEXT A",
    ])
    .unwrap();
    assert_eq!(nested, ["11", "21", "22", "31", "32", "33"]);
    let fizz = run(&[
        "10 N=1",
        "20 IF N>5 THEN 60",
        "30 IF N=3 THEN PRINT \"tres\"",
        "40 N=N+1",
        "50 GOTO 20",
        "60 END",
        "70 PRINT \"nunca\"",
    ])
    .unwrap();
    assert_eq!(fizz, ["tres"]);
}

#[test]
fn runtime_errors_name_the_line_and_infinite_loops_stop() {
    assert_eq!(
        run(&["10 GOTO 99"]).unwrap_err(),
        "Linha 10: Linha 99 não existe."
    );
    assert_eq!(run(&["10 NEXT"]).unwrap_err(), "Linha 10: NEXT sem FOR.");
    assert_eq!(
        run(&["10 IF 1 PRINT 2"]).unwrap_err(),
        "Linha 10: IF sem THEN."
    );
    assert_eq!(
        run(&["10 FOR I=1 TO 2 STEP 0"]).unwrap_err(),
        "Linha 10: STEP 0 não termina."
    );
    assert!(run(&["10 GOTO 10"])
        .unwrap_err()
        .contains("Limite de 100000"));
}
