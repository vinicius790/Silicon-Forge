//! The larger example programs (monitor, Forth, BASIC tokenizer, floating point,
//! LZSS) run on the CPU and are checked against Rust reference behavior. Inputs
//! are varied by replacing each example's data section with generated text.

use mos6502::asm::assemble;
use mos6502::bus::{FlatMemory, MemoryBus};
use mos6502::cpu::Cpu;

const WOZ: &str = include_str!("../examples/wozmon_lite.asm");
const FORTH: &str = include_str!("../examples/forth_lite.asm");
const BASIC: &str = include_str!("../examples/basic_tokenizer.asm");
const FLOAT: &str = include_str!("../examples/float_lite.asm");
const LZ: &str = include_str!("../examples/lz_decompress.asm");

fn run(src: &str, max_instr: u64) -> Cpu<FlatMemory> {
    let program = assemble(src, 0x8000).expect("assembles");
    let mut ram = FlatMemory::new();
    ram.load_at(0x8000, &program);
    ram.set_reset_vector(0x8000);
    let mut cpu = Cpu::new(ram);
    cpu.halt_on_brk = true;
    cpu.reset();
    cpu.run_instructions(max_instr).unwrap();
    assert!(cpu.is_halted(), "program reaches BRK");
    cpu
}

/// Replace everything after `label:` with `data`.
fn with_data(src: &str, label: &str, data: &str) -> String {
    let at = src.find(&format!("\n{label}:")).expect("label") + 1;
    format!("{}{label}:\n{data}", &src[..at])
}

/// `.byte` lines for raw bytes.
fn bytes_lines(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for c in bytes.chunks(16) {
        let items: Vec<String> = c.iter().map(|b| format!("${b:02x}")).collect();
        writeln!(out, "    .byte {}", items.join(", ")).unwrap();
    }
    out
}

fn text_at(cpu: &Cpu<FlatMemory>, addr: u16) -> String {
    let mut out = String::new();
    let mut a = addr;
    loop {
        let b = cpu.bus.peek(a);
        if b == 0 {
            return out;
        }
        out.push(b as char);
        a += 1;
    }
}

// ---- monitor ------------------------------------------------------------

fn woz(script: &str) -> Cpu<FlatMemory> {
    let mut bytes = script.replace('\n', "\r").into_bytes();
    bytes.push(0);
    run(&with_data(WOZ, "script", &bytes_lines(&bytes)), 2_000_000)
}

#[test]
fn monitor_bundled_script_output() {
    let cpu = run(WOZ, 100_000);
    assert_eq!(
        text_at(&cpu, 0x0400),
        "\r0300: 00\r0300: 11 22 33 44 55 66 77 88\r0308: 99 00\r0301: 22\
         \r0320: 00\r0320: A9\r0310: 5A\r0300: 11"
    );
    let stored: Vec<u8> = (0..9).map(|i| cpu.bus.peek(0x0300 + i)).collect();
    assert_eq!(
        stored,
        [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99]
    );
    assert_eq!(cpu.bus.peek(0x0310), 0x5A, "R ran the stored routine");
}

#[test]
fn monitor_block_examine_breaks_lines_on_multiples_of_eight() {
    let cpu = woz("0203.0212\n");
    let expect = "\r0203: 00 00 00 00 00\r0208: 00 00 00 00 00 00 00 00\r0210: 00 00 00";
    assert_eq!(text_at(&cpu, 0x0400), expect);
}

#[test]
fn monitor_store_runs_across_a_page_boundary_and_keeps_the_last_four_digits() {
    let cpu = woz("12302FE: 01 02 03 04\n02FE.0301\n");
    let text = text_at(&cpu, 0x0400);
    assert_eq!(text, "\r02FE: 00\r02FE: 01 02\r0300: 03 04", "{text:?}");
    assert_eq!(cpu.bus.peek(0x0301), 0x04);
}

#[test]
fn monitor_hex_digits_are_case_sensitive_and_junk_is_ignored() {
    let cpu = woz("0300: AB cd\n#0300\n");
    // 'c' and 'd' are not hex digits here, so only AB is stored.
    assert_eq!(cpu.bus.peek(0x0300), 0xAB);
    assert_eq!(cpu.bus.peek(0x0301), 0x00);
    assert!(text_at(&cpu, 0x0400).ends_with("\r0300: AB"));
}

#[test]
fn monitor_examines_every_page_zero_value_it_was_given() {
    for v in [0u8, 0x0F, 0x7F, 0x80, 0xFF] {
        let cpu = woz(&format!("0340: {v:02X}\n0340\n"));
        assert!(text_at(&cpu, 0x0400).ends_with(&format!("\r0340: {v:02X}")));
    }
}

// ---- Forth ----------------------------------------------------------------

fn forth(program: &str) -> String {
    let mut bytes = program.as_bytes().to_vec();
    bytes.push(0);
    let cpu = run(&with_data(FORTH, "source", &bytes_lines(&bytes)), 5_000_000);
    assert_eq!(cpu.x, 0x20 - forth_depth(program), "data stack depth");
    text_at(&cpu, 0x0400)
}

/// Expected stack depth left by the test programs below (they all end balanced
/// unless stated).
fn forth_depth(program: &str) -> u8 {
    if program.contains("LEAVE2") {
        2
    } else {
        0
    }
}

#[test]
fn forth_bundled_program_output() {
    let cpu = run(FORTH, 1_000_000);
    assert_eq!(
        text_at(&cpu, 0x0400),
        "120 49 10 9 8 7 6 5 4 3 2 1 AD\r60000 7 65532 "
    );
    assert_eq!(cpu.x, 0x20);
}

#[test]
fn forth_arithmetic_wraps_at_sixteen_bits() {
    assert_eq!(
        forth("65535 1 + . 0 1 - . 256 256 * . 255 255 * ."),
        "0 65535 0 65025 "
    );
}

#[test]
fn forth_stack_words() {
    assert_eq!(
        forth("1 2 SWAP . . 1 2 OVER . . . 5 DUP . . 9 8 DROP ."),
        "1 2 1 2 1 5 5 9 "
    );
}

#[test]
fn forth_comparisons_leave_minus_one_or_zero() {
    assert_eq!(
        forth("3 3 = . 3 4 = . 3 4 < . 4 3 < . 4 3 > . 3 4 > ."),
        "65535 0 65535 0 65535 0 "
    );
}

#[test]
fn forth_nested_if_else_then() {
    let p = ": T IF IF 1 ELSE 2 THEN ELSE IF 3 ELSE 4 THEN THEN . ; 1 1 T 0 1 T 1 0 T 0 0 T";
    assert_eq!(forth(p), "1 2 3 4 ");
}

#[test]
fn forth_recursion_fibonacci() {
    let p =
        ": FIB DUP 2 < IF ELSE DUP 1 - FIB SWAP 2 - FIB + THEN ; 0 FIB . 1 FIB . 10 FIB . 12 FIB .";
    assert_eq!(forth(p), "0 1 55 144 ");
}

#[test]
fn forth_redefinition_uses_the_newest_word() {
    assert_eq!(forth(": X 1 . ; X : X 2 . ; X"), "1 2 ");
}

#[test]
fn forth_begin_until_counts_and_emits() {
    assert_eq!(
        forth("65 BEGIN DUP EMIT 1 + DUP 70 = UNTIL DROP CR"),
        "ABCDE\r"
    );
}

#[test]
fn forth_unknown_word_prints_a_question_mark_and_stops() {
    let mut bytes = b"1 . FROB 2 .".to_vec();
    bytes.push(0);
    let cpu = run(&with_data(FORTH, "source", &bytes_lines(&bytes)), 100_000);
    assert_eq!(text_at(&cpu, 0x0400), "1 ?");
}

#[test]
fn forth_values_left_on_the_stack_stay_there() {
    let _ = forth(": LEAVE2 ; 1 2 LEAVE2");
}

#[test]
fn forth_prints_every_power_of_two() {
    let mut p = String::from("1 ");
    let mut expect = String::new();
    for i in 0..16 {
        p.push_str("DUP . 2 * ");
        expect.push_str(&format!("{} ", 1u32 << i));
    }
    p.push_str("DROP");
    assert_eq!(forth(&p), expect);
}

// ---- BASIC tokenizer ---------------------------------------------------------

const KEYWORDS: [&str; 14] = [
    "PRINT", "LET", "IF", "THEN", "GOTO", "GOSUB", "RETURN", "END", "FOR", "TO", "NEXT", "REM",
    "INPUT", "STEP",
];

fn basic(program: &str) -> Cpu<FlatMemory> {
    let mut bytes = program.replace('\n', "\r").into_bytes();
    bytes.push(0);
    run(&with_data(BASIC, "source", &bytes_lines(&bytes)), 2_000_000)
}

/// Reference cruncher with the example's rules.
fn crunch(program: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for line in program.lines().filter(|l| !l.trim().is_empty()) {
        let line = line.trim_start();
        let digits: String = line.chars().take_while(|c| c.is_ascii_digit()).collect();
        let n: u16 = digits.parse().unwrap();
        out.extend(n.to_le_bytes());
        let body = line[digits.len()..].as_bytes();
        let mut i = 0;
        while i < body.len() {
            let c = body[i];
            if c == b' ' {
                i += 1;
            } else if c == b'"' {
                let end = body[i + 1..].iter().position(|&b| b == b'"').unwrap() + i + 1;
                out.extend(&body[i..=end]);
                i = end + 1;
            } else if let Some((k, kw)) = KEYWORDS
                .iter()
                .enumerate()
                .find(|(_, kw)| body[i..].starts_with(kw.as_bytes()))
            {
                out.push(0x80 + k as u8);
                i += kw.len();
                if *kw == "REM" {
                    out.extend(&body[i..]);
                    i = body.len();
                }
            } else {
                out.push(c);
                i += 1;
            }
        }
        out.push(0);
    }
    out.extend([0xFF, 0xFF]);
    out
}

fn tokens(cpu: &Cpu<FlatMemory>, len: usize) -> Vec<u8> {
    (0..len as u16).map(|i| cpu.bus.peek(0x0600 + i)).collect()
}

#[test]
fn basic_bundled_program_tokens_and_listing() {
    let cpu = run(BASIC, 1_000_000);
    assert_eq!(cpu.bus.peek(0x10), 0);
    let listing = text_at(&cpu, 0x1000);
    assert_eq!(
        listing,
        "10 REM COUNT TO FIVE\r20 LET A=1\r30 PRINT \"A IS \";A\r40 LET A=A+1\r\
         50 IF A<6 THEN GOTO 30\r60 FOR I=1 TO 3 STEP 1\r70 GOSUB 100\r80 NEXT I\r\
         90 END\r100 PRINT \"SUB\": RETURN\r"
    );
    let source = listing.replace('\r', "\n");
    let want = crunch(&source);
    assert_eq!(tokens(&cpu, want.len()), want);
}

#[test]
fn basic_every_keyword_gets_its_token_and_lists_back() {
    for (k, kw) in KEYWORDS.iter().enumerate() {
        let cpu = basic(&format!("5 {kw} X\n"));
        let t = tokens(&cpu, 6);
        assert_eq!(t[2], 0x80 + k as u8, "{kw}");
        // REM keeps its own " X"; other keywords get one space added after them.
        assert_eq!(text_at(&cpu, 0x1000), format!("5 {kw} X\r"), "{kw}");
    }
}

#[test]
fn basic_strings_and_rem_keep_keywords_and_spaces_verbatim() {
    let p = "1 PRINT \"GOTO  THEN\"\n2 REM PRINT  IF\n";
    let cpu = basic(p);
    let want = crunch(p);
    assert_eq!(tokens(&cpu, want.len()), want);
    assert_eq!(
        text_at(&cpu, 0x1000),
        "1 PRINT \"GOTO  THEN\"\r2 REM PRINT  IF\r"
    );
}

#[test]
fn basic_line_numbers_up_to_65534() {
    let cpu = basic("1 END\n9999 END\n65534 END\n");
    assert_eq!(text_at(&cpu, 0x1000), "1 END\r9999 END\r65534 END\r");
    assert_eq!(&tokens(&cpu, 3)[..2], &[1, 0]);
}

#[test]
fn basic_reports_non_ascending_or_missing_line_numbers() {
    assert_eq!(basic("20 END\n10 END\n").bus.peek(0x10), 1);
    assert_eq!(basic("20 END\n20 END\n").bus.peek(0x10), 1);
    assert_eq!(basic("PRINT\n").bus.peek(0x10), 1);
}

#[test]
fn basic_reports_an_unterminated_string() {
    assert_eq!(basic("10 PRINT \"OOPS\n20 END\n").bus.peek(0x10), 2);
}

#[test]
fn basic_crunch_matches_the_reference_on_a_longer_program() {
    let mut p = String::new();
    for n in 1..=40u16 {
        let kw = KEYWORDS[n as usize % KEYWORDS.len()];
        p.push_str(&format!(
            "{} {kw} A{}=B+{} : \"S {n}\"\n",
            n * 10,
            n % 7,
            n * 3
        ));
    }
    let cpu = basic(&p);
    assert_eq!(cpu.bus.peek(0x10), 0);
    let want = crunch(&p);
    assert_eq!(tokens(&cpu, want.len()), want);
}

// ---- floating point -------------------------------------------------------

/// Unpacked value: exponent (0 = zero), 24-bit mantissa with the top bit set, sign.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fp {
    e: u8,
    m: u32,
    s: bool,
}

fn fp_pack(f: Fp) -> [u8; 4] {
    if f.e == 0 {
        return [0; 4];
    }
    [
        f.e,
        ((f.m >> 16) as u8 & 0x7F) | if f.s { 0x80 } else { 0 },
        (f.m >> 8) as u8,
        f.m as u8,
    ]
}

fn fp_unpack(b: [u8; 4]) -> Fp {
    Fp {
        e: b[0],
        m: (u32::from(b[1] | 0x80) << 16) | (u32::from(b[2]) << 8) | u32::from(b[3]),
        s: b[1] & 0x80 != 0,
    }
}

fn fp_value(b: [u8; 4]) -> f64 {
    let f = fp_unpack(b);
    if f.e == 0 {
        return 0.0;
    }
    let v = f64::from(f.m) / f64::from(1u32 << 24) * 2f64.powi(i32::from(f.e) - 128);
    if f.s {
        -v
    } else {
        v
    }
}

fn fp_from(v: f64) -> [u8; 4] {
    if v == 0.0 {
        return [0; 4];
    }
    let (mut m, mut e) = (v.abs(), 128i32);
    while m >= 1.0 {
        m /= 2.0;
        e += 1;
    }
    while m < 0.5 {
        m *= 2.0;
        e -= 1;
    }
    let mant = (m * f64::from(1u32 << 24)) as u32;
    assert!((1..=255).contains(&e));
    fp_pack(Fp {
        e: e as u8,
        m: mant,
        s: v < 0.0,
    })
}

const ZERO: Fp = Fp {
    e: 0,
    m: 0,
    s: false,
};
const MAXF: Fp = Fp {
    e: 0xFF,
    m: 0xFF_FFFF,
    s: false,
};

/// Reference of the example's algorithms, truncation included.
fn ref_mul(a: Fp, b: Fp) -> (Fp, bool) {
    if a.e == 0 || b.e == 0 {
        return (ZERO, false);
    }
    let s = a.s ^ b.s;
    let e = i32::from(a.e) + i32::from(b.e) - 0x80;
    if e <= 0 {
        return (ZERO, false);
    }
    if e > 255 {
        return (Fp { s, ..MAXF }, true);
    }
    let p = u64::from(a.m) * u64::from(b.m);
    let (mut m, mut e) = ((p >> 24) as u32, e);
    if m & 0x80_0000 == 0 {
        m = ((p >> 23) & 0xFF_FFFF) as u32;
        e -= 1;
        if e == 0 {
            return (Fp { s, ..ZERO }, false);
        }
    }
    (Fp { e: e as u8, m, s }, false)
}

fn ref_add(a: Fp, b: Fp) -> (Fp, bool) {
    if b.e == 0 {
        return (a, false);
    }
    if a.e == 0 {
        return (b, false);
    }
    let (x, mut y) = if a.e >= b.e { (a, b) } else { (b, a) };
    let d = x.e - y.e;
    if d >= 24 {
        return (x, false);
    }
    y.m >>= d;
    if x.s == y.s {
        let sum = x.m + y.m;
        if sum > 0xFF_FFFF {
            if x.e == 0xFF {
                return (Fp { s: x.s, ..MAXF }, true);
            }
            return (
                Fp {
                    e: x.e + 1,
                    m: sum >> 1,
                    s: x.s,
                },
                false,
            );
        }
        return (Fp { m: sum, ..x }, false);
    }
    let (mut m, s) = if x.m >= y.m {
        (x.m - y.m, x.s)
    } else {
        (y.m - x.m, y.s)
    };
    if m == 0 {
        return (Fp { s, ..ZERO }, false);
    }
    let mut e = i32::from(x.e);
    while m & 0x80_0000 == 0 {
        m <<= 1;
        e -= 1;
        if e == 0 {
            return (Fp { s, ..ZERO }, false);
        }
    }
    (Fp { e: e as u8, m, s }, false)
}

fn float_run(ops: &[(u8, [u8; 4], [u8; 4])]) -> (Vec<[u8; 4]>, u8) {
    let mut data = Vec::new();
    for &(op, a, b) in ops {
        data.push(op);
        data.extend(a);
        data.extend(b);
    }
    data.push(0);
    let cpu = run(&with_data(FLOAT, "optable", &bytes_lines(&data)), 5_000_000);
    let results = (0..ops.len() as u16)
        .map(|i| std::array::from_fn(|k| cpu.bus.peek(0x0400 + 4 * i + k as u16)))
        .collect();
    (results, cpu.bus.peek(0x2B))
}

#[test]
fn float_bundled_table() {
    let cpu = run(FLOAT, 100_000);
    let r =
        |i: u16| -> [u8; 4] { std::array::from_fn(|k| cpu.bus.peek(0x0400 + 4 * i + k as u16)) };
    let values: Vec<f64> = (0..7).map(|i| fp_value(r(i))).collect();
    assert_eq!(values, [3.75, 1.5, -1.5, 100.0, 0.0, -18.0, 7.0]);
    assert_eq!(r(7), [0xFF, 0x7F, 0xFF, 0xFF]);
    assert_eq!(cpu.bus.peek(0x2B), 1);
}

/// Small xorshift so the test needs no extra crates.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn float_exactly_representable_cases_match_f64() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let mut ops = Vec::new();
    let mut want = Vec::new();
    for _ in 0..150 {
        // 11-bit mantissas and nearby exponents: sums and products are exact.
        let mk = |r: &mut Rng| {
            let m = (r.next() % 2048) as f64 - 1024.0;
            let e = (r.next() % 9) as i32 - 4;
            m * 2f64.powi(e)
        };
        let (a, b) = (mk(&mut rng), mk(&mut rng));
        let op = if rng.next() & 1 == 0 { b'+' } else { b'*' };
        want.push(if op == b'+' { a + b } else { a * b });
        ops.push((op, fp_from(a), fp_from(b)));
    }
    let (got, err) = float_run(&ops);
    assert_eq!(err, 0);
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(fp_value(*g), *w, "case {i}: {:?}", ops[i]);
    }
}

#[test]
fn float_random_full_mantissas_match_the_reference_bit_for_bit() {
    let mut rng = Rng(0xdead_beef_cafe_f00d);
    let mut ops = Vec::new();
    let mut want = Vec::new();
    let mut overflows = 0;
    for i in 0..300 {
        let mk = |r: &mut Rng| {
            let e = if i % 10 == 0 {
                (r.next() % 256) as u8
            } else {
                0x70 + (r.next() % 32) as u8
            };
            if e == 0 {
                return ZERO;
            }
            Fp {
                e,
                m: 0x80_0000 | (r.next() as u32 & 0x7F_FFFF),
                s: r.next() & 1 == 1,
            }
        };
        let (a, b) = (mk(&mut rng), mk(&mut rng));
        let op = if rng.next() & 1 == 0 { b'+' } else { b'*' };
        let (r, of) = if op == b'+' {
            ref_add(a, b)
        } else {
            ref_mul(a, b)
        };
        overflows += u8::from(of);
        want.push(fp_pack(r));
        ops.push((op, fp_pack(a), fp_pack(b)));
    }
    let (got, err) = float_run(&ops);
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(g, w, "case {i}: {:?}", ops[i]);
    }
    assert_eq!(err, overflows);
}

#[test]
fn float_edge_cases() {
    let one = fp_from(1.0);
    let tiny = [0x01, 0x00, 0x00, 0x00];
    let huge = [0xFF, 0x7F, 0xFF, 0xFF];
    let ops = [
        (b'*', tiny, tiny),                   // underflow -> 0
        (b'+', one, fp_from(2f64.powi(-30))), // below the last bit: unchanged
        (b'+', huge, huge),                   // carry out of exponent $FF: overflow
        (b'*', one, [0; 4]),                  // times zero
        (b'+', fp_from(-3.0), fp_from(3.0)),  // exact cancellation
        (b'*', fp_from(-2.0), fp_from(-0.25)),
    ];
    let (got, err) = float_run(&ops);
    assert_eq!(got[0], [0; 4]);
    assert_eq!(got[1], one);
    assert_eq!(got[2], huge);
    assert_eq!(got[3], [0; 4]);
    assert_eq!(got[4], [0; 4]);
    assert_eq!(fp_value(got[5]), 0.5);
    assert_eq!(err, 1);
}

// ---- LZSS ------------------------------------------------------------------

fn lz_compress(data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u16).to_le_bytes().to_vec();
    let mut i = 0;
    while i < data.len() {
        let flag_at = out.len();
        out.push(0);
        let mut flags = 0u8;
        for bit in 0..8 {
            if i >= data.len() {
                break;
            }
            let mut best = (0usize, 0usize);
            for off in 1..=i.min(4096) {
                let mut l = 0;
                while l < 18 && i + l < data.len() && data[i + l - off] == data[i + l] {
                    l += 1;
                }
                if l > best.0 {
                    best = (l, off);
                }
            }
            if best.0 >= 3 {
                let d = best.1 - 1;
                out.push(d as u8);
                out.push((((d >> 4) & 0xF0) | (best.0 - 3)) as u8);
                i += best.0;
            } else {
                flags |= 1 << bit;
                out.push(data[i]);
                i += 1;
            }
        }
        out[flag_at] = flags;
    }
    out
}

fn lz_roundtrip(data: &[u8]) {
    let packed = lz_compress(data);
    let cpu = run(&with_data(LZ, "packed", &bytes_lines(&packed)), 20_000_000);
    let got: Vec<u8> = (0..data.len() as u16)
        .map(|i| cpu.bus.peek(0x0400 + i))
        .collect();
    assert_eq!(got, data, "len {}", data.len());
    assert_eq!(
        cpu.bus.peek(0x0400 + data.len() as u16),
        0,
        "no byte past the end"
    );
}

#[test]
fn lz_bundled_stream() {
    let cpu = run(LZ, 1_000_000);
    let text = b"TO BE OR NOT TO BE, THAT IS THE QUESTION. TO BE OR NOT TO BE, THAT IS THE ANSWER. ABABABABABABABABABABABAB!";
    let got: Vec<u8> = (0..text.len() as u16)
        .map(|i| cpu.bus.peek(0x0400 + i))
        .collect();
    assert_eq!(got, text);
    assert_eq!(lz_compress(text).len(), 67);
}

#[test]
fn lz_empty_and_tiny_inputs() {
    lz_roundtrip(b"");
    lz_roundtrip(b"A");
    lz_roundtrip(b"AAA");
    lz_roundtrip(b"AAAA");
}

#[test]
fn lz_overlapping_runs_of_every_short_period() {
    for period in 1..=5usize {
        let data: Vec<u8> = (0..200).map(|i| b'a' + (i % period) as u8).collect();
        lz_roundtrip(&data);
    }
}

#[test]
fn lz_random_and_structured_data() {
    let mut rng = Rng(0x0bad_5eed_1234_4321);
    let random: Vec<u8> = (0..600).map(|_| rng.next() as u8).collect();
    lz_roundtrip(&random);
    let low_entropy: Vec<u8> = (0..1500)
        .map(|_| b"ACGT"[(rng.next() % 4) as usize])
        .collect();
    lz_roundtrip(&low_entropy);
    let text = include_str!("../examples/crc16.asm").as_bytes();
    lz_roundtrip(text);
}

#[test]
fn lz_long_distance_matches_reach_back_4096_bytes() {
    let mut rng = Rng(42);
    let block: Vec<u8> = (0..64).map(|_| rng.next() as u8).collect();
    let mut data = block.clone();
    data.extend((0..4000).map(|_| rng.next() as u8));
    data.extend(&block);
    let packed = lz_compress(&data);
    assert!(packed.len() < data.len() + data.len() / 8 + 10);
    lz_roundtrip(&data);
}
