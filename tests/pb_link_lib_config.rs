//! Object libraries with on-demand members, and ld65-style memory/segment configs
//! with alignment, load vs run addresses and the copy table.

use mos6502::link::{
    assemble_object, link_with_config, link_with_libraries, parse_link_config, resolve_libraries,
    Library, LinkOptions, Object,
};

fn obj(name: &str, src: &str) -> Object {
    assemble_object(name, src).unwrap_or_else(|e| panic!("{e}"))
}

fn lib() -> Library {
    Library {
        members: vec![
            obj(
                "print",
                ".section code\n.global print\n.import putc\nprint: jsr putc\n rts\n",
            ),
            obj(
                "putc",
                ".section code\n.global putc\nputc: sta $D012\n rts\n",
            ),
            obj("unused", ".section code\n.global never\nnever: brk\n"),
        ],
    }
}

#[test]
fn only_needed_members_are_pulled_transitively() {
    let main = obj(
        "main",
        ".section code\n.org $8000\n.import print\nstart: jsr print\n",
    );
    let all = resolve_libraries(&[main.clone()], &[lib()], &LinkOptions::default()).unwrap();
    let names: Vec<&str> = all.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["main", "print", "putc"]);
    let l = link_with_libraries(
        &[main],
        &[lib()],
        &LinkOptions {
            reloc_base: 0x9000,
            ..LinkOptions::default()
        },
    )
    .unwrap();
    assert!(l.symbols.contains_key("putc") && !l.symbols.contains_key("never"));
}

#[test]
fn nothing_needed_pulls_nothing_and_first_library_wins() {
    let main = obj("main", ".section code\n.org $8000\nstart: nop\n");
    assert_eq!(
        resolve_libraries(&[main.clone()], &[lib()], &LinkOptions::default())
            .unwrap()
            .len(),
        1
    );
    let other = Library {
        members: vec![obj("alt", ".section code\n.global putc\nputc: rts\n")],
    };
    let needs = obj("m", ".section code\n.org $8000\n.import putc\n jsr putc\n");
    let all = resolve_libraries(
        &[needs.clone()],
        &[other.clone(), lib()],
        &LinkOptions::default(),
    )
    .unwrap();
    assert_eq!(all[1].name, "alt");
    let all = resolve_libraries(&[needs], &[lib(), other], &LinkOptions::default()).unwrap();
    assert_eq!(all[1].name, "putc");
}

#[test]
fn defined_by_options_is_not_pulled() {
    let main = obj(
        "main",
        ".section code\n.org $8000\n.import putc\n jsr putc\n",
    );
    let mut o = LinkOptions::default();
    o.define.insert("putc".into(), 0xFFEE);
    assert_eq!(resolve_libraries(&[main], &[lib()], &o).unwrap().len(), 1);
}

#[test]
fn archive_round_trip_and_corruption() {
    let bytes = lib().to_bytes().unwrap();
    assert_eq!(&bytes[..4], b"M65L");
    assert_eq!(Library::from_bytes(&bytes).unwrap(), lib());
    for cut in 0..bytes.len() {
        assert!(Library::from_bytes(&bytes[..cut]).is_err(), "cut {cut}");
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(Library::from_bytes(&extra).is_err());
    let mut ver = bytes.clone();
    ver[4] = 9;
    assert!(Library::from_bytes(&ver).is_err());
    let dup = Library {
        members: vec![
            obj("a", ".section c\n.global x\nx: nop\n"),
            obj("b", ".section c\n.global x\nx: nop\n"),
        ],
    };
    assert!(dup
        .index()
        .unwrap_err()
        .message
        .contains("exportado por a e b"));
    assert!(dup.to_bytes().is_err());
    assert_eq!(
        Library::from_bytes(&Library::default().to_bytes().unwrap()).unwrap(),
        Library::default()
    );
}

const CFG: &str = "# test config\nMEMORY {\n  ROM: start = $8000, size = $100, fill = yes, fillval = $FF;\n  RAM: start = $0200, size = $100;\n}\nSEGMENTS {\n  CODE: load = ROM;\n  DATA: load = ROM, run = RAM, define = yes;\n  TABLE: load = ROM, align = $10;\n  VECTORS: load = ROM, start = $80FA;\n}\n";

fn program() -> Vec<Object> {
    vec![
        obj("main", ".section code\n.import __data_load__, __data_run__, __data_size__, counter\nstart: lda counter\n.byte __data_size__\n.section vectors\n.byte 1\n"),
        obj("vars", ".section data\n.global counter\ncounter: .byte 7\n.byte 8\n.section table\n.byte $AA\n"),
    ]
}

#[test]
fn layout_load_run_align_start_and_defines() {
    let cfg = parse_link_config(CFG).unwrap();
    let r = link_with_config(&program(), &cfg, &LinkOptions::default()).unwrap();
    let seg = |n: &str| r.segments.iter().find(|s| s.name == n).unwrap().clone();
    assert_eq!(
        (seg("code").load, seg("code").run, seg("code").size),
        (0x8000, 0x8000, 4)
    );
    assert_eq!(
        (seg("data").load, seg("data").run, seg("data").size),
        (0x8004, 0x0200, 2)
    );
    assert_eq!(
        (seg("table").load, seg("table").run),
        (0x8010, 0x8010),
        "aligned past the DATA load bytes"
    );
    assert_eq!(seg("vectors").run, 0x80FA);
    assert_eq!(
        r.linked.symbols["counter"], 0x0200,
        "code sees the run address"
    );
    assert_eq!(r.linked.symbols["__data_load__"], 0x8004);
    assert_eq!(r.linked.symbols["__data_run__"], 0x0200);
    assert_eq!(r.linked.symbols["__data_size__"], 2);
    let (name, start, rom) = &r.images[0];
    assert_eq!((name.as_str(), *start, rom.len()), ("rom", 0x8000, 0x100));
    assert_eq!(
        &rom[..6],
        [0xAD, 0x00, 0x02, 2, 7, 8],
        "DATA bytes stored at their load address"
    );
    assert_eq!(rom[6], 0xFF);
    assert_eq!(rom[0x10], 0xAA);
    assert_eq!(rom[0xFA], 1);
    assert_eq!(r.images.len(), 1, "RAM holds nothing to load");
    assert_eq!(r.copy_table(), [(0x8004, 0x0200, 2)]);
    assert_eq!(
        r.copy_table_bytes(),
        [0x04, 0x80, 0x00, 0x02, 0x02, 0x00, 0x00, 0x00]
    );
}

#[test]
fn every_alignment_power_rounds_up() {
    for shift in 0..8u32 {
        let align = 1u32 << shift;
        let cfg = parse_link_config(&format!(
            "MEMORY {{ M: start = $1001, size = $1000; }}\nSEGMENTS {{ A: load = M; B: load = M, align = ${align:X}; }}"
        ))
        .unwrap();
        let objs = [obj("x", ".section a\n.byte 1\n.section b\n.byte 2\n")];
        let r = link_with_config(&objs, &cfg, &LinkOptions::default()).unwrap();
        let b = r.segments[1].run;
        assert_eq!(u32::from(b) % align, 0);
        assert!(u32::from(b) >= 0x1002 && u32::from(b) < 0x1002 + align);
    }
}

#[test]
fn config_errors() {
    let e = |t: &str| parse_link_config(t).unwrap_err().message;
    assert!(e("MEMORY { A: size = 1; }").contains("sem start"));
    assert!(e("MEMORY { A: start = 0; }").contains("sem size"));
    assert!(e("MEMORY { A: start = $FFFF, size = 2; }").contains("fora"));
    assert!(
        e("MEMORY { A: start = 0, size = $10; B: start = 8, size = $10; }").contains("sobrepõem")
    );
    assert!(e("MEMORY { A: start = 0, size = 1; A: start = 1, size = 1; }").contains("repetida"));
    assert!(e("MEMORY { A: start = 0, size = 1, bogus = 1; }").contains("desconhecido"));
    assert!(e("SEGMENTS { C: load = X; }").contains("desconhecida"));
    assert!(e("MEMORY { A: start = 0, size = 1; } SEGMENTS { C: run = A; }").contains("sem load"));
    assert!(
        e("MEMORY { A: start = 0, size = 1; } SEGMENTS { C: load = A, align = 0; }")
            .contains("align zero")
    );
    assert!(e("MEMORY { A: start = 0, size = 1, fill = maybe; }").contains("yes ou no"));
    assert!(e("FILES { }").contains("bloco desconhecido"));
    assert!(e("MEMORY A: start = 0;").contains("sem"));
    assert!(parse_link_config("MEMORY { A: start = 0, size = $10000; }").is_ok());

    let cfg = parse_link_config("MEMORY { M: start = $1000, size = 4; } SEGMENTS { A: load = M; S: load = M, start = $1000; }").unwrap();
    let le = |objs: &[Object]| {
        link_with_config(objs, &cfg, &LinkOptions::default())
            .unwrap_err()
            .message
    };
    assert!(le(&[obj("x", ".section a\n.res 5\n")]).contains("não cabe"));
    assert!(le(&[obj("x", ".section z\n.byte 1\n")]).contains("sem segmento"));
    assert!(le(&[obj("x", ".section a\n.org $1000\n.byte 1\n")]).contains(".org"));
    assert!(
        le(&[obj("x", ".section a\n.byte 1\n.section s\n.byte 1\n")]).contains("atrás do cursor")
    );
}
