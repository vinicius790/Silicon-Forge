//! GDB Remote Serial Protocol (RSP) Server for MOS 6502.
//!
//! Implements a standard RSP server over localhost TCP (`127.0.0.1:PORT`).
//! Allows standard tools like `gdb-multiarch`, IDA Pro, Ghidra, or LLDB to attach
//! and perform source-level and instruction-level debugging on the 6502 core.
//!
//! Standard RSP Packet format: `$<payload>#<checksum>` with `+`/`-` ACK.
//! Supported commands:
//! - `?`: Query target halt reason (`S05` / SIGTRAP)
//! - `g` / `G`: Read / write registers
//! - `m` / `M`: Read / write memory
//! - `s`: Single step one instruction
//! - `c`: Continue execution until breakpoint or instruction limit
//! - `Z0` / `z0`: Insert / remove software breakpoint
//! - `qSupported`: Protocol feature negotiation
//! - `k`: Kill session

use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use crate::bus::MemoryBus;
use crate::cpu::Cpu;

/// Compute GDB 8-bit checksum: sum of characters modulo 256.
pub fn gdb_checksum(payload: &[u8]) -> u8 {
    payload.iter().fold(0u8, |acc, &b| acc.wrapping_add(b))
}

/// Format an RSP packet with payload: `$<payload>#<checksum>`.
pub fn format_packet(payload: &str) -> Vec<u8> {
    let cs = gdb_checksum(payload.as_bytes());
    format!("${payload}#{cs:02x}").into_bytes()
}

/// Parse hex string into byte buffer.
pub fn parse_hex_bytes(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Encode byte slice into lowercase hexadecimal string.
pub fn to_hex_string(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// GDB RSP Debug Session.
pub struct GdbServer {
    listener: TcpListener,
}

impl GdbServer {
    /// Bind to a loopback address (`127.0.0.1:0` for dynamic port, or specific port).
    pub fn bind(addr: &str) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        if !listener.local_addr()?.ip().is_loopback() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "GDB server only binds to loopback",
            ));
        }
        Ok(Self { listener })
    }

    pub fn port(&self) -> std::io::Result<u16> {
        Ok(self.listener.local_addr()?.port())
    }

    /// Accept one connection and run debug session until disconnect or kill.
    pub fn handle_client<B: MemoryBus>(
        &self,
        cpu: &mut Cpu<B>,
        max_instructions_per_continue: u64,
    ) -> std::io::Result<()> {
        let (mut stream, _) = self.listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        let mut breakpoints = HashSet::<u16>::new();
        let mut buf = [0u8; 4096];

        loop {
            let n = match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock {
                        continue;
                    }
                    break;
                }
            };

            let req = &buf[..n];
            // Send ACK `+` for incoming packet
            if req.contains(&b'$') {
                let _ = stream.write_all(b"+");
            }

            if let Some(cmd) = extract_payload(req) {
                let response = handle_command(cpu, cmd, &mut breakpoints, max_instructions_per_continue);
                if let Some(resp) = response {
                    if resp == "KILL" {
                        break;
                    }
                    let pkt = format_packet(&resp);
                    let _ = stream.write_all(&pkt);
                }
            }
        }

        Ok(())
    }
}

/// Extract payload from `$payload#cs` packet.
fn extract_payload(bytes: &[u8]) -> Option<&str> {
    let start = bytes.iter().position(|&b| b == b'$')? + 1;
    let end = bytes.iter().position(|&b| b == b'#')?;
    if start <= end {
        std::str::from_utf8(&bytes[start..end]).ok()
    } else {
        None
    }
}

/// Process a single GDB command and return response payload.
pub fn handle_command<B: MemoryBus>(
    cpu: &mut Cpu<B>,
    cmd: &str,
    breakpoints: &mut HashSet<u16>,
    max_steps: u64,
) -> Option<String> {
    if cmd.is_empty() {
        return None;
    }

    // Query / Halt Reason
    if cmd == "?" {
        return Some("S05".into()); // SIGTRAP
    }

    // Handshake queries
    if cmd.starts_with("qSupported") {
        return Some("PacketSize=1000;hwbreak+;swbreak+".into());
    }
    if cmd.starts_with("qAttached") {
        return Some("1".into());
    }
    if cmd.starts_with("qTStatus") || cmd.starts_with("qC") || cmd.starts_with("qOffsets") {
        return Some("".into());
    }

    // Kill
    if cmd == "k" {
        return Some("KILL".into());
    }

    // Read Registers ('g')
    // Layout: A (8-bit), X (8-bit), Y (8-bit), SP (8-bit), P (8-bit), PC (16-bit little endian)
    if cmd == "g" {
        let regs = [
            cpu.regs.a,
            cpu.regs.x,
            cpu.regs.y,
            cpu.regs.sp,
            cpu.regs.p.bits(),
            (cpu.regs.pc & 0xFF) as u8,
            (cpu.regs.pc >> 8) as u8,
        ];
        return Some(to_hex_string(&regs));
    }

    // Write Registers ('G')
    if let Some(hex) = cmd.strip_prefix('G') {
        if let Some(bytes) = parse_hex_bytes(hex) {
            if bytes.len() >= 7 {
                cpu.regs.a = bytes[0];
                cpu.regs.x = bytes[1];
                cpu.regs.y = bytes[2];
                cpu.regs.sp = bytes[3];
                cpu.regs.p = crate::flags::Status::from_bits_truncate(bytes[4]);
                cpu.regs.pc = (bytes[5] as u16) | ((bytes[6] as u16) << 8);
                return Some("OK".into());
            }
        }
        return Some("E01".into());
    }

    // Single step ('s')
    if cmd == "s" || cmd.starts_with("s") {
        cpu.step();
        return Some("S05".into());
    }

    // Continue ('c')
    if cmd == "c" || cmd.starts_with("c") {
        let mut steps = 0u64;
        while steps < max_steps && !cpu.halted {
            cpu.step();
            steps += 1;
            if breakpoints.contains(&cpu.regs.pc) {
                break;
            }
        }
        return Some("S05".into());
    }

    // Read Memory ('m addr,length')
    if let Some(rest) = cmd.strip_prefix('m') {
        if let Some((addr_str, len_str)) = rest.split_once(',') {
            if let (Ok(addr), Ok(len)) = (
                u16::from_str_radix(addr_str, 16),
                usize::from_str_radix(len_str, 16),
            ) {
                let mut bytes = Vec::with_capacity(len);
                for i in 0..len {
                    let a = addr.wrapping_add(i as u16);
                    bytes.push(cpu.bus.peek(a));
                }
                return Some(to_hex_string(&bytes));
            }
        }
        return Some("E01".into());
    }

    // Write Memory ('M addr,length:hexbytes')
    if let Some(rest) = cmd.strip_prefix('M') {
        if let Some((addr_len, hex)) = rest.split_once(':') {
            if let Some((addr_str, _)) = addr_len.split_once(',') {
                if let (Ok(addr), Some(bytes)) = (
                    u16::from_str_radix(addr_str, 16),
                    parse_hex_bytes(hex),
                ) {
                    for (i, &b) in bytes.iter().enumerate() {
                        let a = addr.wrapping_add(i as u16);
                        cpu.bus.write(a, b);
                    }
                    return Some("OK".into());
                }
            }
        }
        return Some("E01".into());
    }

    // Breakpoints ('Z0,addr,kind' insert, 'z0,addr,kind' remove)
    if let Some(rest) = cmd.strip_prefix("Z0,") {
        if let Some((addr_str, _)) = rest.split_once(',') {
            if let Ok(addr) = u16::from_str_radix(addr_str, 16) {
                breakpoints.insert(addr);
                return Some("OK".into());
            }
        }
        return Some("E01".into());
    }
    if let Some(rest) = cmd.strip_prefix("z0,") {
        if let Some((addr_str, _)) = rest.split_once(',') {
            if let Ok(addr) = u16::from_str_radix(addr_str, 16) {
                breakpoints.remove(&addr);
                return Some("OK".into());
            }
        }
        return Some("OK".into());
    }

    // Unhandled command: empty response indicates unsupported
    Some("".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::FlatMemory;

    #[test]
    fn test_gdb_checksum_and_packet() {
        let cs = gdb_checksum(b"OK");
        assert_eq!(cs, b'O'.wrapping_add(b'K'));
        let pkt = format_packet("OK");
        assert_eq!(String::from_utf8(pkt).unwrap(), "$OK#9a");
    }

    #[test]
    fn test_gdb_commands_regs_and_memory() {
        let mut mem = FlatMemory::new();
        mem.data[0x1000] = 0xEA; // NOP
        let mut cpu = Cpu::new(mem);
        cpu.regs.pc = 0x1000;
        cpu.regs.a = 0x42;
        let mut bp = HashSet::new();

        // Query status
        assert_eq!(handle_command(&mut cpu, "?", &mut bp, 100).unwrap(), "S05");

        // Read memory
        let m = handle_command(&mut cpu, "m1000,1", &mut bp, 100).unwrap();
        assert_eq!(m, "ea");

        // Step
        let s = handle_command(&mut cpu, "s", &mut bp, 100).unwrap();
        assert_eq!(s, "S05");
        assert_eq!(cpu.regs.pc, 0x1001);
    }
}
