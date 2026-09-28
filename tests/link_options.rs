//! Linker-defined symbols (define/provide/section symbols), map and image output,
//! and bank-switched overlay links.

use mos6502::link::{
    assemble_object, link_banks, link_with_options, parse_link_options, Bank, LinkOptions, Object,
};

fn obj(name: &str, src: &str) -> Object {
    assemble_object(name, src).unwrap()
}

fn main_obj() -> Object {
    obj(
        "main",
        ".section code\n.org $8000\n.global start\nstart:\n  lda table\n  jmp handler\n",
    )
}

#[test]
fn provide_is_weak_define_is_strong() {
    let mut o = LinkOptions {
        reloc_base: 0x9000,
        ..LinkOptions::default()
    };
    o.provide.insert("table".into(), 0x1234);
    o.provide.insert("handler".into(), 0xABCD);
    let linked = link_with_options(&[main_obj()], &o).unwrap();
    assert_eq!(
        &linked.segments[0].bytes,
        &[0xAD, 0x34, 0x12, 0x4C, 0xCD, 0xAB]
    );
    // An object definition overrides the weak one.
    let lib = obj("lib", ".section code\n.global handler\nhandler:\n  rts\n");
    let linked = link_with_options(&[main_obj(), lib.clone()], &o).unwrap();
    assert_eq!(linked.symbols["handler"], 0x9000);
    assert_eq!(&linked.segments[0].bytes[3..], &[0x4C, 0x00, 0x90]);
    // A strong define that collides with an object global is a duplicate.
    let mut strong = o.clone();
    strong.define.insert("handler".into(), 0x1111);
    let err = link_with_options(&[main_obj(), lib], &strong).unwrap_err();
    assert!(
        err.message.contains("duplicado: handler"),
        "{}",
        err.message
    );
    // Without any definition the symbol is undefined.
    let err = link_with_options(&[main_obj()], &LinkOptions::default()).unwrap_err();
    assert!(err.message.contains("indefinido"), "{}", err.message);
}

#[test]
fn section_symbols_span_same_named_sections() {
    let a = obj("a", ".section data\n.byte 1\n.byte 2\n.byte 3\n.section code\n.org $8000\n  lda __data_size\n  ldx __data_start\n  ldy __data_end\n  jmp __code_end\n");
    let b = obj("b", ".section data\n.byte 4\n.byte 5\n");
    let o = LinkOptions {
        reloc_base: 0x0300,
        section_symbols: true,
        ..LinkOptions::default()
    };
    let l = link_with_options(&[a, b], &o).unwrap();
    assert_eq!(l.symbols["__data_start"], 0x0300);
    assert_eq!(l.symbols["__data_end"], 0x0305);
    assert_eq!(l.symbols["__data_size"], 5);
    assert_eq!(l.symbols["__code_start"], 0x8000);
    assert_eq!(l.symbols["__code_end"], 0x800C);
    let code = l.segments.iter().find(|s| s.address == 0x8000).unwrap();
    assert_eq!(&code.bytes[..3], &[0xAD, 0x05, 0x00]);
    assert_eq!(&code.bytes[9..], &[0x4C, 0x0C, 0x80]);
}

#[test]
fn map_and_image() {
    let lib = obj(
        "lib",
        ".section code\n.global handler\nhandler:\n  rts\n.global table\ntable:\n.byte 9\n",
    );
    let l = link_with_options(
        &[main_obj(), lib],
        &LinkOptions {
            reloc_base: 0x9000,
            ..LinkOptions::default()
        },
    )
    .unwrap();
    let map = l.map();
    assert!(map.contains("  $8000-$8005     6  code (main)"), "{map}");
    assert!(map.contains("  $9000-$9001     2  code (lib)"), "{map}");
    let syms: Vec<&str> = map.split("SÍMBOLOS\n").nth(1).unwrap().lines().collect();
    assert_eq!(syms, ["  $8000 start", "  $9000 handler", "  $9001 table"]);
    let img = l.image(0x8000, 0x9001, 0xFF);
    assert_eq!(img.len(), 0x1002);
    assert_eq!(img[6], 0xFF);
    assert_eq!(&img[0x1000..], &[0x60, 0x09]);
}

#[test]
fn script_with_provide_define_and_symbols() {
    let o = parse_link_options(
        "; comment\nsection code $8000\nprovide table $2000\ndefine handler $3000\nsymbols\n",
        0x9000,
    )
    .unwrap();
    assert_eq!(
        (o.provide["table"], o.define["handler"], o.section_symbols),
        (0x2000, 0x3000, true)
    );
    assert_eq!(o.script[0].address, 0x8000);
    let l = link_with_options(&[main_obj()], &o).unwrap();
    assert_eq!(l.symbols["__code_size"], 6);
    for bad in [
        "provide x",
        "define",
        "provide x 1 2",
        "provide x 1\nprovide x 2",
        "bogus a b",
    ] {
        assert!(parse_link_options(bad, 0).is_err(), "{bad}");
    }
}

fn bank(name: &str, value: u8) -> Bank {
    Bank {
        name: name.into(),
        objects: vec![obj(name, &format!(".section bank\n.org $A000\n.global entry\nentry:\n  lda #{value}\n  rts\n.global only_{name}\nonly_{name}:\n.byte {value}\n"))],
    }
}

#[test]
fn banks_share_addresses_and_see_fixed_code_only() {
    let fixed = obj(
        "fixed",
        ".section fixed\n.org $C000\n.global reset\nreset:\n  jsr entry\n  jmp reset\n",
    );
    let banks: Vec<Bank> = (0..4).map(|i| bank(&format!("b{i}"), i)).collect();
    let r = link_banks(&[fixed.clone()], &banks, &LinkOptions::default()).unwrap();
    assert_eq!(r.fixed.segments.len(), 1);
    assert_eq!(
        &r.fixed.segments[0].bytes,
        &[0x20, 0x00, 0xA0, 0x4C, 0x00, 0xC0]
    );
    assert_eq!(r.fixed.symbols.keys().collect::<Vec<_>>(), ["reset"]);
    for (i, (name, l)) in r.banks.iter().enumerate() {
        assert_eq!(name, &format!("b{i}"));
        assert_eq!(l.segments.len(), 1);
        assert_eq!(l.segments[0].address, 0xA000);
        assert_eq!(l.segments[0].bytes, [0xA9, i as u8, 0x60, i as u8]);
        assert_eq!(l.symbols["reset"], 0xC000, "fixed visible");
        assert!(
            !l.symbols.contains_key(&format!("only_b{}", (i + 1) % 4)),
            "other banks invisible"
        );
    }
    // A bank referencing another bank's symbol fails.
    let mut bad = banks.clone();
    bad[1]
        .objects
        .push(obj("x", ".section more\n  jmp only_b0\n"));
    let e = link_banks(&[fixed.clone()], &bad, &LinkOptions::default()).unwrap_err();
    assert!(
        e.message.starts_with("banco b1:") && e.message.contains("only_b0"),
        "{}",
        e.message
    );
    // Entry at different addresses breaks the shared fixed code.
    let mut moved = banks.clone();
    moved[2] = Bank {
        name: "b2".into(),
        objects: vec![obj(
            "b2",
            ".section bank\n.org $A000\n  nop\n.global entry\nentry:\n  rts\n",
        )],
    };
    let e = link_banks(&[fixed.clone()], &moved, &LinkOptions::default()).unwrap_err();
    assert!(
        e.message.contains("parte fixa difere no banco b2"),
        "{}",
        e.message
    );
    // Overlap inside one bank is still an error; duplicate names and no banks too.
    let mut clash = banks.clone();
    clash[0]
        .objects
        .push(obj("c", ".section other\n.org $A001\n  nop\n"));
    assert!(
        link_banks(&[fixed.clone()], &clash, &LinkOptions::default())
            .unwrap_err()
            .message
            .contains("sobrepostas")
    );
    let dup = vec![bank("b0", 0), bank("b0", 1)];
    assert!(link_banks(&[fixed.clone()], &dup, &LinkOptions::default())
        .unwrap_err()
        .message
        .contains("duplicado"));
    assert!(link_banks(&[fixed], &[], &LinkOptions::default()).is_err());
}

#[test]
fn memory_regions_check_every_placement() {
    use mos6502::link::Region;
    let rom = |s: u16, e: u16| Region {
        name: format!("r{s:04X}"),
        start: s,
        end: e,
    };
    let code = obj("m", ".section code\n.org $8000\n  nop\n  nop\n  nop\n");
    // Sweep the region end around the section's last byte ($8002).
    for end in 0x7FF0u16..=0x8010 {
        let o = LinkOptions {
            regions: vec![rom(0x8000, end)],
            ..LinkOptions::default()
        };
        let r = link_with_options(&[code.clone()], &o);
        assert_eq!(r.is_ok(), end >= 0x8002, "end {end:04X}");
        if end < 0x8000 {
            assert!(r.unwrap_err().message.contains("vazia"));
        }
    }
    for start in 0x7FF0u16..=0x8010 {
        let o = LinkOptions {
            regions: vec![rom(start, 0xFFFF)],
            ..LinkOptions::default()
        };
        let r = link_with_options(&[code.clone()], &o);
        assert_eq!(r.is_ok(), start <= 0x8000, "start {start:04X}");
        if let Err(e) = r {
            assert!(
                e.message.contains("seção code ($8000-$8002) fora"),
                "{}",
                e.message
            );
        }
    }
    let o = parse_link_options("region ram $0200 $07FF\nregion rom $8000 $FFFF\n", 0x0200).unwrap();
    let data = obj("d", ".section data\n.byte 1\n.byte 2\n");
    let l = link_with_options(&[code.clone(), data.clone()], &o).unwrap();
    assert_eq!(
        l.region_usage(&o.regions),
        [
            ("ram".to_string(), 2, 0x600),
            ("rom".to_string(), 3, 0x8000)
        ]
    );
    let low = LinkOptions {
        reloc_base: 0x0100,
        ..o.clone()
    };
    assert!(link_with_options(&[data], &low)
        .unwrap_err()
        .message
        .contains("fora das regiões"));
    for bad in [
        "region a $10 $20\nregion b $20 $30",
        "region a $10 $20\nregion a $30 $40",
    ] {
        let o = parse_link_options(bad, 0).unwrap();
        assert!(link_with_options(&[code.clone()], &o).is_err(), "{bad}");
    }
    assert!(parse_link_options("region a $10", 0).is_err());
    assert!(parse_link_options("region a $10 $20 $30", 0).is_err());
}
