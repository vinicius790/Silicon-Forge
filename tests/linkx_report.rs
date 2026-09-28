//! Linker: `.segment`, `.res`, per-section size limits, fill to a bank boundary,
//! ca65-style `.dbg` export and the relocation report.

use mos6502::link::{
    assemble_object, format_relocation_report, link_with_options, parse_link_options,
    relocation_report, LinkOptions, Object, RelocKind, SectionFill, SectionLimit,
};

fn obj(name: &str, src: &str) -> Object {
    assemble_object(name, src).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn segment_reopens_and_res_reserves() {
    let o = obj(
        "m",
        ".segment \"CODE\"\n.org $8000\nstart: nop\n.segment DATA\n.res 3, $55\n.segment \"CODE\"\njmp start\n",
    );
    assert_eq!(o.sections.len(), 2);
    assert_eq!(o.sections[0].bytes, [0xEA, 0x4C, 0, 0]);
    assert_eq!(o.sections[1].bytes, [0x55; 3]);
    let l = link_with_options(
        &[o],
        &LinkOptions {
            reloc_base: 0x0300,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(l.segments[0].bytes, [0xEA, 0x4C, 0x00, 0x80]);
    assert!(assemble_object("x", ".section a\n.res 2, $100\n").is_err());
    assert!(assemble_object("x", ".section a\n.section a\n").is_err());
}

#[test]
fn size_limit_every_boundary() {
    for n in 0usize..=8 {
        let src = format!(".section code\n.org $8000\n.res {n:X}\n");
        let objects = [obj("m", &src)];
        for max in 0usize..=8 {
            let o = LinkOptions {
                limits: vec![SectionLimit {
                    name: "code".into(),
                    max,
                }],
                ..Default::default()
            };
            let r = link_with_options(&objects, &o);
            assert_eq!(r.is_ok(), n <= max, "n={n} max={max}");
            if let Err(e) = r {
                assert_eq!(
                    e.message,
                    format!("seção code excede o limite: {n} > {max} bytes")
                );
            }
        }
    }
}

#[test]
fn limit_sums_same_named_sections() {
    let a = obj("a", ".section data\n.res 3\n");
    let b = obj("b", ".section data\n.res 2\n");
    let opts = parse_link_options("limit data 4\n", 0x0200).unwrap();
    assert!(link_with_options(&[a.clone(), b.clone()], &opts)
        .unwrap_err()
        .message
        .contains("5 > 4"));
    let opts = parse_link_options("limit data 5\n", 0x0200).unwrap();
    assert!(link_with_options(&[a, b], &opts).is_ok());
    assert!(parse_link_options("limit data\n", 0).is_err());
}

#[test]
fn fill_to_bank_boundary() {
    let o = obj("m", ".section code\n.org $8000\nnop\nnop\nnop\n");
    let opts = parse_link_options("fill code $4000 $FF\n", 0).unwrap();
    assert_eq!(
        opts.fills,
        [SectionFill {
            name: "code".into(),
            boundary: 0x4000,
            byte: 0xFF
        }]
    );
    let l = link_with_options(&[o.clone()], &opts).unwrap();
    assert_eq!(l.segments[0].bytes.len(), 0x4000);
    assert_eq!(&l.segments[0].bytes[..3], &[0xEA; 3]);
    assert!(l.segments[0].bytes[3..].iter().all(|&b| b == 0xFF));
    assert_eq!((l.placements[0].len, l.placements[0].padding), (3, 0x3FFD));
    // Whole 64 KiB bank: up to $FFFF.
    let l = link_with_options(
        &[o.clone()],
        &parse_link_options("fill code 65536 0\n", 0).unwrap(),
    )
    .unwrap();
    assert_eq!(l.segments[0].bytes.len(), 0x8000);
    // Already on a boundary: no padding.
    let exact = obj("e", ".section code\n.org $8000\n.res $10\n");
    let l = link_with_options(
        &[exact],
        &parse_link_options("fill code $10 0\n", 0).unwrap(),
    )
    .unwrap();
    assert_eq!(l.placements[0].padding, 0);
}

#[test]
fn fill_every_boundary_size() {
    for boundary in 1u32..=64 {
        let o = obj("m", ".section code\n.org $8001\nnop\nnop\n");
        let opts = LinkOptions {
            fills: vec![SectionFill {
                name: "code".into(),
                boundary,
                byte: 0xEE,
            }],
            ..Default::default()
        };
        let l = link_with_options(&[o], &opts).unwrap();
        let end = 0x8001 + l.segments[0].bytes.len() as u32;
        assert_eq!(end % boundary, 0, "b={boundary}");
        assert!(end - 0x8003 < boundary);
    }
}

#[test]
fn fill_errors() {
    let a = obj("a", ".section code\n.org $8000\nnop\n");
    let b = obj("b", ".section vec\n.org $8010\nnop\n");
    let e = link_with_options(
        &[a.clone(), b],
        &parse_link_options("fill code $100 0\n", 0).unwrap(),
    )
    .unwrap_err();
    assert!(e.message.contains("sobrepõe vec"), "{}", e.message);
    let e = link_with_options(
        &[a.clone()],
        &parse_link_options("fill nope $100 0\n", 0).unwrap(),
    )
    .unwrap_err();
    assert!(e.message.contains("fill sem seção"));
    let e = link_with_options(
        &[a.clone()],
        &parse_link_options("region rom $8000 $80FF\nfill code $1000 0\n", 0).unwrap(),
    )
    .unwrap_err();
    assert!(e.message.contains("região"), "{}", e.message);
    assert!(parse_link_options("fill code 0 0\n", 0).is_err());
    assert!(parse_link_options("fill code $10 $100\n", 0).is_err());
    assert!(parse_link_options("fill code $10\n", 0).is_err());
}

#[test]
fn dbg_export() {
    let a = obj(
        "a",
        ".section code\n.org $8000\n.global start\nstart: lda tbl\nloop: jmp loop\n",
    );
    let b = obj("b", ".section zp\n.org $0010\n.global tbl\ntbl: .byte 1\n");
    let mut opts = LinkOptions::default();
    opts.define.insert("magic".into(), 0x1234);
    let l = link_with_options(&[a, b], &opts).unwrap();
    let dbg = l.to_dbg();
    let lines: Vec<&str> = dbg.lines().collect();
    assert_eq!(
        lines,
        [
            "version\tmajor=2,minor=0",
            "info\tseg=2,sym=3",
            "seg\tid=0,name=\"zp\",start=0x000010,size=0x0001,addrsize=zeropage,type=rw",
            "seg\tid=1,name=\"code\",start=0x008000,size=0x0006,addrsize=absolute,type=rw",
            "sym\tid=0,name=\"tbl\",addrsize=zeropage,val=0x0010,seg=0,type=lab",
            "sym\tid=1,name=\"magic\",addrsize=absolute,val=0x1234,type=lab",
            "sym\tid=2,name=\"start\",addrsize=absolute,val=0x8000,seg=1,type=lab",
        ]
    );
}

#[test]
fn relocation_report_lists_every_patch() {
    let a = obj(
        "a",
        ".section code\n.org $8000\n.global start\nstart: lda tbl\nldx #zpv\nloop: bne loop\njmp start\n",
    );
    let b = obj("b", ".section data\n.org $0300\n.global tbl\ntbl: .byte 1\n.global zpv\n.section zero\n.org $0020\nzpv: .byte 2\n");
    let objects = [a, b];
    let l = link_with_options(&objects, &LinkOptions::default()).unwrap();
    let r = relocation_report(&objects, &l);
    let summary: Vec<(u16, RelocKind, &str, u16, Vec<u8>)> = r
        .iter()
        .map(|e| {
            (
                e.site,
                e.kind,
                e.symbol.as_str(),
                e.target,
                e.patched.clone(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (0x8001, RelocKind::Absolute, "tbl", 0x0300, vec![0x00, 0x03]),
            (0x8004, RelocKind::AbsoluteByte, "zpv", 0x0020, vec![0x20]),
            (0x8006, RelocKind::Relative, "loop", 0x8005, vec![0xFE]),
            (
                0x8008,
                RelocKind::Absolute,
                "start",
                0x8000,
                vec![0x00, 0x80]
            ),
        ]
    );
    let text = format_relocation_report(&r);
    assert!(text.starts_with("RELOCAÇÕES\n"));
    assert!(
        text.contains("  $8001 ABS16 a:code tbl -> $0300 [00 03]\n"),
        "{text}"
    );
    assert!(
        text.contains("  $8006 REL8  a:code loop -> $8005 [FE]\n"),
        "{text}"
    );
    assert!(text.ends_with("  total: 4\n"));
}
