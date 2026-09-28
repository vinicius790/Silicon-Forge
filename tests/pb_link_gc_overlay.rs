//! `.import`/`.export` across objects, section garbage collection, overlay map.

use mos6502::link::{
    assemble_object, collect_garbage, link_banks, link_with_options, parse_link_options, Bank,
    LinkOptions, Object,
};

fn obj(name: &str, src: &str) -> Object {
    assemble_object(name, src).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn import_export_resolve_across_objects() {
    let main = obj(
        "main",
        ".section code\n.org $8000\n.import put, table\nstart: jsr put\n lda table\n",
    );
    let lib = obj(
        "lib",
        ".section lib\n.export put, table\nput: rts\ntable: .byte 7\n",
    );
    let l = link_with_options(
        &[main, lib],
        &LinkOptions {
            reloc_base: 0x9000,
            ..LinkOptions::default()
        },
    )
    .unwrap();
    assert_eq!(l.symbols["put"], 0x9000);
    assert_eq!(l.symbols["table"], 0x9001);
    let mut mem = vec![0u8; 0x10000];
    l.store(&mut mem);
    assert_eq!(&mem[0x8000..0x8006], [0x20, 0x00, 0x90, 0xAD, 0x01, 0x90]);
}

#[test]
fn import_rules() {
    let e = |src: &str| assemble_object("m", src).unwrap_err().message;
    assert!(e(".section c\n.import x\nx: nop\n").contains("definido aqui"));
    assert!(e(".section c\n.import x\n jmp y\n").contains("sem definição nem .import: y"));
    assert!(e(".section c\n.import x, x\n").contains("repetido"));
    assert!(e(".section c\n.export z\n").contains("sem definição"));
    // Without any .import the permissive rule stays.
    assert!(assemble_object("m", ".section c\n jmp y\n").is_ok());
    // An import nobody exports fails at link time.
    let m = obj("m", ".section c\n.org $8000\n.import gone\n jmp gone\n");
    assert!(link_with_options(&[m], &LinkOptions::default()).is_err());
}

fn gc_objects() -> Vec<Object> {
    vec![
        obj("a", ".section boot\n.org $8000\n.global start\nstart: jsr used\n jmp start\n.section dead\nunused: jsr helper\n"),
        obj("b", ".section lib\n.global used\nused: jsr deeper\n rts\n.section lib2\n.global deeper\ndeeper: rts\n"),
        obj("c", ".section junk\n.global helper\nhelper: rts\n.section vectors\n.org $FFFA\n.byte 1\n"),
    ]
}

#[test]
fn gc_keeps_only_reachable_sections() {
    let kept = collect_garbage(&gc_objects(), &["start".into()]).unwrap();
    let names: Vec<Vec<&str>> = kept
        .iter()
        .map(|o| o.sections.iter().map(|s| s.name.as_str()).collect())
        .collect();
    assert_eq!(names, [vec!["boot"], vec!["lib", "lib2"], vec![]]);
    assert!(kept[0].symbols.iter().all(|s| s.name != "unused"));
    assert!(kept[2].symbols.is_empty() && kept[2].notes.is_empty());
    let kept = collect_garbage(&gc_objects(), &["start".into(), "vectors".into()]).unwrap();
    assert_eq!(kept[2].sections.len(), 1);
    assert_eq!(kept[2].sections[0].name, "vectors");
    // `dead` as a root pulls `junk` in through helper.
    let kept = collect_garbage(&gc_objects(), &["dead".into()]).unwrap();
    assert_eq!(kept[2].sections[0].name, "junk");
    assert!(kept[0].sections.iter().all(|s| s.name == "dead"));
}

#[test]
fn gc_through_link_options_and_script() {
    let o = parse_link_options("gc\nkeep start\nkeep vectors\n", 0x9000).unwrap();
    assert!(o.gc);
    assert_eq!(o.keep, ["start", "vectors"]);
    let full = link_with_options(
        &gc_objects(),
        &LinkOptions {
            reloc_base: 0x9000,
            ..LinkOptions::default()
        },
    )
    .unwrap();
    let small = link_with_options(&gc_objects(), &o).unwrap();
    assert!(full.symbols.contains_key("helper"));
    assert!(!small.symbols.contains_key("helper"));
    assert_eq!(small.placements.len(), 4);
    assert_eq!(small.symbols["start"], 0x8000);
    assert!(parse_link_options("gc x\n", 0).is_err());
    assert!(parse_link_options("keep\n", 0).is_err());
    let none = LinkOptions {
        gc: true,
        ..LinkOptions::default()
    };
    assert!(link_with_options(&gc_objects(), &none)
        .unwrap_err()
        .message
        .contains("sem raiz"));
    assert!(collect_garbage(&gc_objects(), &["typo".into()])
        .unwrap_err()
        .message
        .contains("typo"));
}

#[test]
fn overlay_map_groups_bank_windows() {
    let fixed = obj("fixed", ".section fixed\n.org $C000\nreset: jmp reset\n");
    let bank = |name: &str, org: u16, n: usize| Bank {
        name: name.into(),
        objects: vec![obj(
            name,
            &format!(".section code\n.org ${org:04X}\n.res ${n:X}\n"),
        )],
    };
    let r = link_banks(
        &[fixed],
        &[
            bank("b0", 0xA000, 0x10),
            bank("b1", 0xA008, 0x10),
            bank("b2", 0xA100, 4),
            bank("b3", 0xB000, 2),
        ],
        &LinkOptions::default(),
    )
    .unwrap();
    let windows = r.overlays();
    assert_eq!(windows.len(), 1, "b2 and b3 overlap nobody");
    assert_eq!((windows[0].start, windows[0].end), (0xA000, 0xA017));
    assert_eq!(
        windows[0]
            .members
            .iter()
            .map(|m| m.bank.as_str())
            .collect::<Vec<_>>(),
        ["b0", "b1"]
    );
    let report = r.overlay_report();
    assert_eq!(
        report,
        "MAPA DE SOBREPOSIÇÃO\n  fixo $C000-$C002 fixed (fixed)\n  janela $A000-$A017 2 seções\n    $A000-$A00F b0:code (b0)\n    $A008-$A017 b1:code (b1)\n  total: 1 janelas\n"
    );
}
