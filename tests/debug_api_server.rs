//! Debug API: pure request handling over many inputs, and real loopback sockets on
//! port 0 that shut down after a fixed number of requests.

use mos6502::debug_api::{handle, DebugServer};
use mos6502::{Cpu, FlatMemory};
use std::io::{Read, Write};
use std::net::TcpStream;

fn cpu() -> Cpu<FlatMemory> {
    let mut mem = FlatMemory::new();
    mem.load_at(0x0400, &[0xE8, 0xE8, 0xE8, 0x02]);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x0400;
    cpu
}

fn req(method: &str, target: &str, body: &str) -> Vec<u8> {
    format!(
        "{method} {target} HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

#[test]
fn routes_methods_and_validation() {
    let mut c = cpu();
    assert_eq!(handle(&mut c, &req("POST", "/step", "{\"n\":2}")).0, 200);
    assert_eq!(c.x, 2);
    assert_eq!(
        handle(&mut c, &req("POST", "/step", "")).0,
        200,
        "empty body = 1"
    );
    assert_eq!(c.x, 3);
    for bad in ["{\"n\":0}", "{\"n\":-1}", "{\"n\":\"x\"}", "{\"n\":10001}"] {
        assert_eq!(handle(&mut c, &req("POST", "/step", bad)).0, 400, "{bad}");
    }
    let (status, body) = handle(&mut c, &req("POST", "/step", "{\"n\":5}"));
    assert_eq!(status, 200);
    assert!(
        body.contains("\"halted\":true,\"stepped\":1"),
        "JAM halts and stops the run: {body}"
    );
    for (m, p) in [
        ("POST", "/registers"),
        ("DELETE", "/memory"),
        ("GET", "/step"),
    ] {
        assert_eq!(handle(&mut c, &req(m, p, "")).0, 405, "{m} {p}");
    }
    assert_eq!(handle(&mut c, &req("GET", "/nope", "")).0, 404);
    assert_eq!(
        handle(&mut c, b"GET / HTTP/1.1\r\nContent-Length: 9\r\n\r\nabc").0,
        400
    );
    assert_eq!(
        handle(&mut c, b"GET / HTTP/1.1\r\nContent-Length: x\r\n\r\n").0,
        400
    );
}

#[test]
fn memory_round_trips_for_every_start_page_and_rejects_bad_ranges() {
    let mut c = cpu();
    for page in 0..=255u32 {
        let start = page * 256 + 3;
        let bytes = format!("[{},{},255]", page, 255 - page);
        let put = format!("{{\"start\":{start},\"bytes\":{bytes}}}");
        let (status, _) = handle(&mut c, &req("PUT", "/memory", &put));
        if start + 3 > 65536 {
            assert_eq!(status, 400);
            continue;
        }
        assert_eq!(status, 200, "{start}");
        let (status, body) = handle(
            &mut c,
            &req("GET", &format!("/memory?start={start}&len=3"), ""),
        );
        assert_eq!(
            (status, body),
            (200, format!("{{\"start\":{start},\"bytes\":{bytes}}}"))
        );
    }
    for q in [
        "start=65535&len=2",
        "len=257",
        "start=x",
        "len=-1",
        "start=70000&len=0",
    ] {
        assert_eq!(
            handle(&mut c, &req("GET", &format!("/memory?{q}"), "")).0,
            400,
            "{q}"
        );
    }
    assert_eq!(
        handle(
            &mut c,
            &req("PUT", "/memory", "{\"start\":0,\"bytes\":[256]}")
        )
        .0,
        400
    );
    assert_eq!(
        handle(&mut c, &req("PUT", "/memory", "{\"bytes\":[1]}")).0,
        400
    );
    assert_eq!(
        handle(&mut c, &req("PUT", "/memory", "{\"start\":0}")).0,
        400
    );
}

#[test]
fn loopback_server_on_port_0_answers_then_stops() {
    let server = DebugServer::bind("127.0.0.1:0").unwrap();
    let port = server.local_port().unwrap();
    assert_ne!(port, 0);
    let client = std::thread::spawn(move || {
        let mut answers = Vec::new();
        for raw in [
            req("POST", "/step", "{\"n\":1}"),
            req("GET", "/registers", ""),
            b"garbage\r\n\r\n".to_vec(),
        ] {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(&raw).unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            answers.push(out);
        }
        answers
    });
    let mut c = cpu();
    server.serve_requests(&mut c, 3).unwrap();
    drop(server);
    let answers = client.join().unwrap();
    assert!(answers[0].starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(answers[1].contains("\"x\":1"), "{}", answers[1]);
    assert!(answers[2].starts_with("HTTP/1.1 404"), "{}", answers[2]);
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_err(),
        "listener closed"
    );
}

#[test]
fn non_loopback_addresses_are_refused() {
    match DebugServer::bind("0.0.0.0:0") {
        Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied),
        Ok(_) => panic!("bound a non-loopback address"),
    }
}
