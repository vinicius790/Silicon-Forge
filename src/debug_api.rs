//! Localhost debug API. Standard library only (`std::net`).
//!
//! Bind to `127.0.0.1` (the CLI refuses any other host). One HTTP/1.1 request per
//! connection, then the socket closes. `serve_requests` stops after `max_requests`
//! accepts so tests can shut the listener down. Nothing here is left running by the
//! test suite.
//!
//! - `GET /registers` — `a,x,y,sp,pc,p,cycles,halted`
//! - `POST /step` body `{"n":N}` — run N instructions via `Cpu::step` (cap 10_000)
//! - `GET /memory?start=S&len=L` — up to 256 bytes, side-effect free (`peek`)
//! - `PUT /memory` body `{"start":S,"bytes":[...]}` — write those bytes

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use crate::bus::{FlatMemory, MemoryBus};
use crate::cpu::Cpu;

pub struct DebugServer {
    listener: TcpListener,
}

impl DebugServer {
    /// `addr` should be `127.0.0.1:0` or `127.0.0.1:PORT`.
    /// Fails with `PermissionDenied` for any address that is not loopback.
    pub fn bind(addr: &str) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        if !listener.local_addr()?.ip().is_loopback() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "debug API only binds loopback",
            ));
        }
        Ok(Self { listener })
    }

    pub fn local_port(&self) -> std::io::Result<u16> {
        Ok(self.listener.local_addr()?.port())
    }

    /// Accept and handle `max_requests` connections, then return. The listener is
    /// dropped with `self`.
    pub fn serve_requests(
        &self,
        cpu: &mut Cpu<FlatMemory>,
        max_requests: usize,
    ) -> std::io::Result<()> {
        for _ in 0..max_requests {
            let (mut stream, _) = self.listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            stream.set_write_timeout(Some(Duration::from_secs(2)))?;
            match read_request(&mut stream) {
                Ok(request) => {
                    let (status, body) = dispatch(cpu, &request);
                    write_response(&mut stream, status, &body)?;
                }
                Err(error) => {
                    let body = format!("{{\"error\":\"{error}\"}}");
                    write_response(&mut stream, 400, &body)?;
                }
            }
        }
        Ok(())
    }
}

/// Parse one raw request and answer it, without a socket.
pub fn handle(cpu: &mut Cpu<FlatMemory>, raw: &[u8]) -> (u16, String) {
    let mut cursor = std::io::Cursor::new(raw.to_vec());
    match read_from(&mut cursor) {
        Ok(request) => dispatch(cpu, &request),
        Err(error) => (400, format!("{{\"error\":\"{error}\"}}")),
    }
}

struct Request {
    method: String,
    path: String,
    query: String,
    body: String,
}

fn dispatch(cpu: &mut Cpu<FlatMemory>, request: &Request) -> (u16, String) {
    let path = request.path.as_str();
    match (request.method.as_str(), path) {
        ("GET", "/registers") => (200, registers_json(cpu)),
        ("POST", "/step") => {
            let n = match request.body.trim() {
                "" => 1,
                body => match json_u64(body, "n") {
                    Some(n) if n > 0 => n,
                    _ => return (400, "{\"error\":\"n must be a positive integer\"}".into()),
                },
            };
            if n > 10_000 {
                return (400, "{\"error\":\"n exceeds 10000\"}".into());
            }
            let mut stepped = 0u64;
            for _ in 0..n {
                if cpu.is_halted() {
                    break;
                }
                if let Err(error) = cpu.step() {
                    let escaped = error.replace('"', "'");
                    return (500, format!("{{\"error\":\"{escaped}\"}}"));
                }
                stepped += 1;
            }
            let mut body = registers_json(cpu);
            body.pop();
            body.push_str(&format!(",\"stepped\":{stepped}}}"));
            (200, body)
        }
        ("GET", "/memory") => {
            let (start, len) = match (
                query_u32(&request.query, "start"),
                query_u32(&request.query, "len"),
            ) {
                (Some(Ok(s)), Some(Ok(l))) => (s, l),
                (Some(Ok(s)), None) => (s, 16),
                (None, Some(Ok(l))) => (0, l),
                (None, None) => (0, 16),
                _ => return (400, "{\"error\":\"start and len must be integers\"}".into()),
            };
            if start > 65535 || len > 256 || start + len > 65536 {
                return (400, "{\"error\":\"memory range out of bounds\"}".into());
            }
            let mut bytes = Vec::with_capacity(len as usize);
            for offset in 0..len {
                bytes.push(cpu.bus.peek((start as u16).wrapping_add(offset as u16)));
            }
            (
                200,
                format!("{{\"start\":{start},\"bytes\":{}}}", fmt_bytes(&bytes)),
            )
        }
        ("PUT", "/memory") => {
            let start = match json_u64(&request.body, "start") {
                Some(value) => value,
                None => return (400, "{\"error\":\"missing start\"}".into()),
            };
            let bytes = match json_byte_array(&request.body) {
                Some(Ok(bytes)) => bytes,
                Some(Err(())) => return (400, "{\"error\":\"bytes must be 0..=255\"}".into()),
                None => return (400, "{\"error\":\"missing bytes\"}".into()),
            };
            if start > 65535 || bytes.len() > 4096 || start + bytes.len() as u64 > 65536 {
                return (400, "{\"error\":\"memory range out of bounds\"}".into());
            }
            for (offset, byte) in bytes.iter().enumerate() {
                cpu.bus.write(start as u16 + offset as u16, *byte);
            }
            (
                200,
                format!("{{\"start\":{start},\"written\":{}}}", bytes.len()),
            )
        }
        (_, "/registers" | "/step" | "/memory") => {
            (405, "{\"error\":\"method not allowed\"}".into())
        }
        _ => (404, "{\"error\":\"not found\"}".into()),
    }
}

fn registers_json(cpu: &Cpu<FlatMemory>) -> String {
    let regs = cpu.registers();
    format!(
        "{{\"a\":{},\"x\":{},\"y\":{},\"sp\":{},\"pc\":{},\"p\":{},\"cycles\":{},\"halted\":{}}}",
        regs.a,
        regs.x,
        regs.y,
        regs.sp,
        regs.pc,
        regs.p,
        regs.cycles,
        if cpu.is_halted() { "true" } else { "false" }
    )
}

fn fmt_bytes(bytes: &[u8]) -> String {
    let mut out = String::from("[");
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&byte.to_string());
    }
    out.push(']');
    out
}

fn json_u64(body: &str, key: &str) -> Option<u64> {
    let needle = format!("\"{key}\"");
    let index = body.find(&needle)? + needle.len();
    let rest = body[index..].trim_start().strip_prefix(':')?.trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// `None` when there is no `bytes` array; `Some(Err)` for an element outside 0..=255.
fn json_byte_array(body: &str) -> Option<Result<Vec<u8>, ()>> {
    let index = body.find("\"bytes\"")?;
    let rest = &body[index..];
    let start = rest.find('[')? + 1;
    let end = rest.find(']')?;
    let mut bytes = Vec::new();
    for part in rest[start..end].split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.parse::<u8>() {
            Ok(b) => bytes.push(b),
            Err(_) => return Some(Err(())),
        }
    }
    Some(Ok(bytes))
}

fn query_u32(query: &str, key: &str) -> Option<Result<u32, ()>> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == key).then(|| v.parse().map_err(|_| ()))
    })
}

fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    read_from(stream)
}

fn read_from<R: Read>(stream: &mut R) -> Result<Request, String> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    let header_end;
    loop {
        let n = stream.read(&mut tmp).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("connection closed".into());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find_header_end(&buf) {
            header_end = pos;
            break;
        }
        if buf.len() > 65_536 {
            return Err("headers too large".into());
        }
    }
    let header = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = header.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut fields = request_line.split_whitespace();
    let method = fields.next().unwrap_or("").to_string();
    let target = fields.next().unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut content_length = 0usize;
    for line in lines {
        if let Some(value) = line
            .split_once(':')
            .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .map(|(_, value)| value.trim())
        {
            content_length = value
                .parse()
                .map_err(|_| "bad content-length".to_string())?;
        }
    }
    if content_length > 65_536 {
        return Err("body too large".into());
    }
    while buf.len() < header_end + 4 + content_length {
        let n = stream.read(&mut tmp).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let body_bytes = &buf[header_end + 4..];
    if body_bytes.len() < content_length {
        return Err("body shorter than content-length".into());
    }
    let body =
        String::from_utf8_lossy(&body_bytes[..content_length.min(body_bytes.len())]).to_string();
    Ok(Request {
        method,
        path: path.to_string(),
        query: query.to_string(),
        body,
    })
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

fn write_response(stream: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn localhost_step_updates_the_accumulator_then_the_listener_stops() {
        let server = DebugServer::bind("127.0.0.1:0").unwrap();
        let port = server.local_port().unwrap();
        let mut cpu = Cpu::new(FlatMemory::new());
        // LDA #$42
        cpu.bus.load_at(0x8000, &[0xA9, 0x42]);
        cpu.bus.set_reset_vector(0x8000);
        cpu.reset();
        assert_eq!(cpu.a, 0);

        let handle = thread::spawn(move || {
            server.serve_requests(&mut cpu, 2).unwrap();
        });

        thread::sleep(Duration::from_millis(20));
        let step = http(port, "POST /step HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 7\r\nConnection: close\r\n\r\n{\"n\":1}");
        assert!(step.contains("\"stepped\":1"), "{step}");
        let regs = http(
            port,
            "GET /registers HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        );
        assert!(regs.contains("\"a\":66"), "{regs}");
        handle.join().unwrap();
    }

    fn http(port: u16, request: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut buf = String::new();
        stream.read_to_string(&mut buf).unwrap();
        buf
    }
}
