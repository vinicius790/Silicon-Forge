//! Full ld65-style `.dbg` export: file, mod, seg, span, scope, line, sym.

use mos6502::link::{assemble_object, link_with_options, LinkOptions};

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_once('\t')?
        .1
        .split(',')
        .find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
}

#[test]
fn every_record_kind_is_written_and_consistent() {
    let a = assemble_object(
        "a",
        ".section code\n.org $8000\n.global start\nstart: lda tbl\nloop: jmp loop\n",
    )
    .unwrap();
    let b = assemble_object(
        "b",
        ".section zp\n.org $0010\n.global tbl\ntbl: .byte 1\n.byte 2\n.byte 3\n",
    )
    .unwrap();
    let mut opts = LinkOptions::default();
    opts.define.insert("magic".into(), 0x1234);
    let l = link_with_options(&[a, b], &opts).unwrap();
    let dbg = l.to_dbg_full();
    let lines: Vec<&str> = dbg.lines().collect();
    assert_eq!(lines[0], "version\tmajor=2,minor=0");
    let kind = |k: &str| {
        lines
            .iter()
            .filter(|l| l.starts_with(&format!("{k}\t")))
            .copied()
            .collect::<Vec<_>>()
    };
    let info = lines[1];
    for k in ["file", "line", "mod", "scope", "seg", "span", "sym"] {
        let n: usize = field(info, k).unwrap().parse().unwrap();
        assert_eq!(n, kind(k).len(), "{k} count");
        for (i, rec) in kind(k).iter().enumerate() {
            assert_eq!(field(rec, "id"), Some(i.to_string().as_str()), "{rec}");
        }
    }
    assert_eq!(
        kind("file"),
        [
            "file\tid=0,name=\"a\",size=0,mtime=0x00000000,mod=1",
            "file\tid=1,name=\"b\",size=0,mtime=0x00000000,mod=0"
        ]
    );
    assert_eq!(
        kind("mod"),
        ["mod\tid=0,name=\"b\",file=1", "mod\tid=1,name=\"a\",file=0"]
    );
    assert_eq!(
        kind("span"),
        [
            "span\tid=0,seg=1,start=0,size=3",
            "span\tid=1,seg=1,start=3,size=3",
            "span\tid=2,seg=0,start=0,size=1",
            "span\tid=3,seg=0,start=1,size=1",
            "span\tid=4,seg=0,start=2,size=1",
        ]
    );
    assert_eq!(
        kind("scope"),
        [
            "scope\tid=0,name=\"\",mod=0,size=3,span=2+3+4",
            "scope\tid=1,name=\"\",mod=1,size=6,span=0+1"
        ]
    );
    assert_eq!(
        kind("line"),
        [
            "line\tid=0,file=0,line=4,type=0,span=0",
            "line\tid=1,file=0,line=5,type=0,span=1",
            "line\tid=2,file=1,line=4,type=0,span=2",
            "line\tid=3,file=1,line=5,type=0,span=3",
            "line\tid=4,file=1,line=6,type=0,span=4",
        ]
    );
    assert_eq!(
        kind("sym"),
        [
            "sym\tid=0,name=\"tbl\",addrsize=zeropage,scope=0,def=2,val=0x0010,seg=0,type=lab",
            "sym\tid=1,name=\"magic\",addrsize=absolute,val=0x1234,type=lab",
            "sym\tid=2,name=\"start\",addrsize=absolute,scope=1,def=0,val=0x8000,seg=1,type=lab",
        ]
    );
    // Spans tile each segment's bytes exactly.
    for seg in kind("seg") {
        let id = field(seg, "id").unwrap();
        let size = usize::from_str_radix(field(seg, "size").unwrap().trim_start_matches("0x"), 16)
            .unwrap();
        let mut covered: Vec<(usize, usize)> = kind("span")
            .iter()
            .filter(|s| field(s, "seg") == Some(id))
            .map(|s| {
                (
                    field(s, "start").unwrap().parse().unwrap(),
                    field(s, "size").unwrap().parse().unwrap(),
                )
            })
            .collect();
        covered.sort();
        let mut at = 0;
        for (start, len) in covered {
            assert_eq!(start, at);
            at += len;
        }
        assert_eq!(at, size);
    }
    // The short export is unchanged.
    assert!(!l.to_dbg().contains("span\t"));
}
