//! Profiler call graph and folded stacks; coverage LCOV export.

use mos6502::asm;
use mos6502::coverage::{source_lines, Coverage, SourceLine};
use mos6502::profiler::{EdgeStat, Profiler};
use mos6502::symbols::SymbolTable;
use mos6502::{Cpu, FlatMemory, MemoryBus};

const NESTED: &str = "\
main:
  JSR outer
  JSR leaf
  BRK
outer:
  JSR leaf
  JSR leaf
  RTS
leaf:
  NOP
  RTS
";

fn cpu_for(src: &str) -> (Cpu<FlatMemory>, asm::AsmOutput) {
    let o = asm::assemble_with_symbols(src, 0x8000).unwrap();
    let mut mem = FlatMemory::new();
    mem.load_at(o.origin, &o.bytes);
    let mut cpu = Cpu::new(mem);
    cpu.pc = o.origin;
    cpu.sp = 0xFF;
    (cpu, o)
}

fn profile(src: &str) -> (Profiler, asm::AsmOutput) {
    let (mut cpu, o) = cpu_for(src);
    let mut p = Profiler::new();
    while cpu.bus.peek(cpu.pc) != 0 {
        p.step(&mut cpu).unwrap();
    }
    (p, o)
}

#[test]
fn call_edges_split_by_caller() {
    let (p, o) = profile(NESTED);
    let outer = o.symbols["outer"];
    let leaf = o.symbols["leaf"];
    // leaf = NOP 2 + RTS 6 = 8 inclusive per call.
    assert_eq!(
        p.edge(Some(outer), leaf),
        EdgeStat {
            calls: 2,
            inclusive: 16
        }
    );
    assert_eq!(
        p.edge(None, leaf),
        EdgeStat {
            calls: 1,
            inclusive: 8
        }
    );
    // outer = 2 JSR (12) + 16 + RTS 6 = 34.
    assert_eq!(
        p.edge(None, outer),
        EdgeStat {
            calls: 1,
            inclusive: 34
        }
    );
    assert_eq!(p.call_edges().len(), 3);
    assert_eq!(p.edge(Some(leaf), outer), EdgeStat::default());
}

#[test]
fn folded_stacks_sum_to_total_and_match_exclusive() {
    let (p, o) = profile(NESTED);
    let syms = SymbolTable::from_asm(&o).unwrap();
    let folded = p.folded(Some(&syms));
    let mut lines: Vec<&str> = folded.lines().collect();
    lines.sort();
    assert_eq!(
        lines,
        vec!["top 12", "top;leaf 8", "top;outer 18", "top;outer;leaf 16"]
    );
    let sum: u64 = folded
        .lines()
        .map(|l| l.rsplit(' ').next().unwrap().parse::<u64>().unwrap())
        .sum();
    assert_eq!(sum, p.total_cycles());
    let outer = p.routine(o.symbols["outer"]).unwrap();
    assert_eq!(outer.exclusive, 18);
    // Without symbols frames are hex addresses.
    assert!(p
        .folded(None)
        .contains(&format!("top;${:04X} 8", o.symbols["leaf"])));
}

#[test]
fn dot_graph_lists_nodes_and_edges() {
    let (p, o) = profile(NESTED);
    let dot = p.call_graph_dot();
    let leaf = o.symbols["leaf"];
    let outer = o.symbols["outer"];
    assert!(dot.starts_with("digraph calls {"));
    assert!(dot.contains(&format!("top -> r{leaf:04X} [label=\"1x 8\"]")));
    assert!(dot.contains(&format!("r{outer:04X} -> r{leaf:04X} [label=\"2x 16\"]")));
    assert!(dot.trim_end().ends_with('}'));
}

#[test]
fn coverage_hits_count_executions() {
    let src = "  LDX #3\nloop: DEX\n  BNE loop\n  BRK\n";
    let (mut cpu, _) = cpu_for(src);
    let mut c = Coverage::new();
    while cpu.bus.peek(cpu.pc) != 0 {
        c.step(&mut cpu).unwrap();
    }
    assert_eq!(c.hits(0x8000), 1);
    assert_eq!(c.hits(0x8002), 3);
    assert_eq!(c.hits(0x8003), 3);
    assert_eq!(c.hits(0x8001), 0, "operand bytes are not hits");
    let mut twice = c.clone();
    twice.merge(&c);
    assert_eq!(twice.hits(0x8002), 6);
}

#[test]
fn source_lines_skip_blank_comments_data_and_equates() {
    let src = "; header\n\nSCREEN = $0400\n  .org $8000\nstart:  LDA #1 ; go\n\n  STA SCREEN\ntable: .byte 1,2\n  BRK\n";
    let o = asm::assemble_with_symbols(src, 0x8000).unwrap();
    let lines = source_lines(src, &o).unwrap();
    assert_eq!(
        lines,
        vec![
            SourceLine {
                line: 5,
                address: 0x8000
            },
            SourceLine {
                line: 7,
                address: 0x8002
            },
            SourceLine {
                line: 9,
                address: 0x8007
            },
        ]
    );
}

#[test]
fn lcov_tracefile_format() {
    let src = "  LDA #1\n  BEQ skip\n  LDA #2\nskip: BRK\n";
    let (mut cpu, o) = cpu_for(src);
    let mut c = Coverage::new();
    while cpu.bus.peek(cpu.pc) != 0 {
        c.step(&mut cpu).unwrap();
    }
    let lines = source_lines(src, &o).unwrap();
    let lcov = c.to_lcov("t", "prog.asm", &lines);
    assert_eq!(
        lcov,
        "TN:t\nSF:prog.asm\nDA:1,1\nDA:2,1\nDA:3,1\nDA:4,0\nLF:4\nLH:3\nend_of_record\n"
    );
}
