//! Weak library symbols, ld65 map, o65 round-trip.

use mos6502::link::{assemble_object, link_with_options, Library, LinkOptions, O65File, Object};

fn obj(name: &str, src: &str) -> Object {
    assemble_object(name, src).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn weak_symbol_overridden_by_strong() {
    let weak = obj("w", ".section code\n.weak put\nput: lda #1\n rts\n");
    let strong = obj("s", ".section code\n.global put\nput: lda #2\n rts\n");
    let main = obj("m", ".section code\n.org $8000\n.import put\n jsr put\n");
    let l = link_with_options(
        &[main, weak.clone(), strong],
        &LinkOptions {
            reloc_base: 0x9000,
            ..LinkOptions::default()
        },
    )
    .unwrap();
    assert_eq!(
        l.segments
            .iter()
            .find(|s| s.address == l.symbols["put"])
            .unwrap()
            .bytes[1],
        2
    );
    // Two weaks are fine.
    let w2 = obj("w2", ".section code\n.weak put\nput: lda #3\n rts\n");
    assert!(link_with_options(
        &[
            obj("m", ".section code\n.org $8000\n.import put\n jsr put\n"),
            weak.clone(),
            w2
        ],
        &LinkOptions {
            reloc_base: 0x9000,
            ..LinkOptions::default()
        },
    )
    .is_ok());
}

#[test]
fn library_prefers_strong_member() {
    let lib = Library {
        members: vec![
            obj("weak", ".section code\n.weak util\nutil: rts\n"),
            obj("strong", ".section code\n.global util\nutil: nop\n rts\n"),
        ],
    };
    assert!(lib.index().is_ok());
    let main = obj("m", ".section code\n.org $8000\n.import util\n jsr util\n");
    let all = mos6502::link::resolve_libraries(&[main], &[lib], &LinkOptions::default()).unwrap();
    assert!(all.iter().any(|o| o.name == "strong"));
}

#[test]
fn map_ld65_and_o65_round_trip() {
    let a = obj(
        "a",
        ".section code\n.org $8000\n.global start\nstart: nop\n rts\n",
    );
    let b = obj(
        "b",
        ".section data\n.org $9000\n.global tbl\ntbl: .byte 1
.byte 2\n",
    );
    let l = link_with_options(&[a, b], &LinkOptions::default()).unwrap();
    let map = l.map_ld65();
    assert!(map.contains("Modules list:"));
    assert!(map.contains("Segment list:"));
    assert!(map.contains("Exports list by name:"));
    assert!(map.contains("start"));
    assert!(map.contains("008000") || map.contains("8000"));
    let o65 = O65File::from_linked(&l).unwrap();
    let bytes = o65.to_bytes();
    let back = O65File::from_bytes(&bytes).unwrap();
    assert_eq!(back.text, o65.text);
    assert_eq!(back.data, o65.data);
    assert!(back.exports.iter().any(|e| e.name == "start"));
    for cut in 0..bytes.len().min(40) {
        assert!(O65File::from_bytes(&bytes[..cut]).is_err());
    }
}
