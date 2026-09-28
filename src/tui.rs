//! Painel de terminal (TUI) com sequências ANSI puras, sem dependências.
//!
//! [`TuiDashboard::frame`] monta a tela inteira como texto (registradores, flags,
//! seis instruções a partir do PC, 48 bytes de memória em hex/ASCII e 16 bytes da
//! pilha em volta de SP); [`TuiDashboard::run_with`] lê comandos de qualquer `BufRead`
//! e escreve em qualquer `Write`, então sessões inteiras são testáveis. Comandos
//! ([`TuiCommand::parse`]): `s`/vazio passo, `c [n]` n instruções (padrão 100, para
//! no primeiro erro ou halt), `r` reset, `h` halt, `m <end>` move o dump, `q` sai.
//! Erros da CPU aparecem na linha de status do próximo quadro em vez de sumir.

use crate::bus::MemoryBus;
use crate::cpu::Cpu;
use crate::disasm::disasm_variant;
use crate::flags::bits;
use std::fmt::Write as _;
use std::io::{self, BufRead, Write};

const RULE: &str =
    "+-----------------------------------------------------------------------------------+";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiCommand {
    Step,
    Continue(u32),
    Reset,
    Halt,
    Mem(u16),
    Quit,
}

impl TuiCommand {
    pub fn parse(line: &str) -> Result<Self, String> {
        let line = line.trim().to_ascii_lowercase();
        let mut parts = line.split_whitespace();
        let head = parts.next().unwrap_or("s");
        let arg = parts.next();
        if parts.next().is_some() {
            return Err(format!("argumentos demais: {line}"));
        }
        let number = |text: &str, radix| {
            let digits = text.trim_start_matches('$').trim_start_matches("0x");
            u32::from_str_radix(digits, radix).map_err(|_| format!("número inválido: {text}"))
        };
        match (head, arg) {
            ("s", None) => Ok(Self::Step),
            ("c", None) => Ok(Self::Continue(100)),
            ("c", Some(n)) => match number(n, 10)? {
                0 => Err("c precisa de n > 0".into()),
                n => Ok(Self::Continue(n)),
            },
            ("r", None) => Ok(Self::Reset),
            ("h", None) => Ok(Self::Halt),
            ("m", Some(a)) => u16::try_from(number(a, 16)?)
                .map(Self::Mem)
                .map_err(|_| format!("endereço fora de 16 bits: {a}")),
            ("q", None) => Ok(Self::Quit),
            _ => Err(format!("comando desconhecido: {line}")),
        }
    }
}

pub struct TuiDashboard {
    pub mem_view_addr: u16,
    pub is_running: bool,
    /// Last error or note, shown under the header of the next frame.
    pub status: String,
}

impl TuiDashboard {
    pub fn new() -> Self {
        Self {
            mem_view_addr: 0x0000,
            is_running: true,
            status: String::new(),
        }
    }

    /// The whole screen, starting with clear-screen and home.
    pub fn frame<B: MemoryBus>(&self, cpu: &Cpu<B>) -> String {
        let mut out = String::from("\x1b[H\x1b[2J");
        let halted = if cpu.is_halted() {
            "\x1b[31mYES\x1b[1;36m"
        } else {
            "\x1b[32mNO\x1b[1;36m "
        };
        let _ = writeln!(out, "\x1b[1;36m{RULE}\x1b[0m");
        let _ = writeln!(
            out,
            "\x1b[1;36m| \x1b[1;33mMOS 6502 SYSTEM MONITOR\x1b[1;36m - [VARIANT: {:?}] [CYCLES: {:<10}] [HALT: {halted}]  |\x1b[0m",
            cpu.variant, cpu.cycles
        );
        if !self.status.is_empty() {
            let _ = writeln!(out, "\x1b[31m  {}\x1b[0m", self.status);
        }
        let _ = writeln!(
            out,
            "\x1b[1;36m{RULE}\x1b[0m\n\x1b[1;37m  REGISTRADORES:\x1b[0m"
        );
        let _ = writeln!(
            out,
            "  A: \x1b[1;32m${:02X}\x1b[0m ({:>3})   X: \x1b[1;32m${:02X}\x1b[0m ({:>3})   Y: \x1b[1;32m${:02X}\x1b[0m ({:>3})   SP: \x1b[1;33m${:02X}\x1b[0m   PC: \x1b[1;35m${:04X}\x1b[0m",
            cpu.a, cpu.a, cpu.x, cpu.x, cpu.y, cpu.y, cpu.sp, cpu.pc
        );
        out.push_str("  FLAGS: [");
        for (name, bit) in [
            ("N", bits::N),
            ("V", bits::V),
            ("-", 0),
            ("B", bits::B),
            ("D", bits::D),
            ("I", bits::I),
            ("Z", bits::Z),
            ("C", bits::C),
        ] {
            match (bit, cpu.p.get(bit)) {
                (0, _) => out.push_str("- "),
                (_, true) => {
                    let _ = write!(out, "\x1b[1;32m{name}:1\x1b[0m ");
                }
                (_, false) => {
                    let _ = write!(out, "\x1b[31m{name}:0\x1b[0m ");
                }
            }
        }
        let _ = writeln!(out, "] ({})", cpu.p);
        let _ = writeln!(
            out,
            "\x1b[1;36m{RULE}\x1b[0m\n\x1b[1;37m  DISASSEMBLY AO VIVO:\x1b[0m"
        );
        let mut pc = cpu.pc;
        for i in 0..6 {
            let peek = |k: u16| cpu.bus.peek(pc.wrapping_add(k));
            let line = disasm_variant(peek(0), peek(1), peek(2), cpu.variant);
            let (mark, end) = if i == 0 {
                ("  \x1b[1;32m==> ", "\x1b[0m")
            } else {
                ("      ", "")
            };
            let _ = writeln!(
                out,
                "{mark}${pc:04X}: {:<9} {:<24}{end}",
                line.bytes_hex, line.text
            );
            pc = pc.wrapping_add(line.size as u16);
        }
        let base = self.mem_view_addr & 0xFFF0;
        let _ = writeln!(
            out,
            "\x1b[1;36m{RULE}\x1b[0m\n\x1b[1;37m  DUMP DE MEMÓRIA (INÍCIO: ${base:04X}):\x1b[0m"
        );
        for row in 0..3u16 {
            let addr = base.wrapping_add(row * 16);
            let bytes: Vec<u8> = (0..16)
                .map(|j| cpu.bus.peek(addr.wrapping_add(j)))
                .collect();
            let _ = write!(out, "  ${addr:04X}: ");
            for b in &bytes {
                let _ = write!(out, "{b:02X} ");
            }
            let ascii: String = bytes
                .iter()
                .map(|&b| {
                    if (0x20..=0x7E).contains(&b) {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            let _ = writeln!(out, "|{ascii}|");
        }
        // Sixteen stack bytes starting 8 below SP (clamped to the page), SP highlighted.
        let first = cpu.sp.saturating_sub(8).min(0xF0);
        let _ = writeln!(
            out,
            "\x1b[1;36m{RULE}\x1b[0m\n\x1b[1;37m  PILHA ($01{first:02X}..=$01{:02X} | SP aponta para $01{:02X}):\x1b[0m",
            first + 15,
            cpu.sp
        );
        out.push_str("  ");
        for i in first..=first + 15 {
            let val = cpu.bus.peek(0x0100 | u16::from(i));
            if i == cpu.sp {
                let _ = write!(out, "\x1b[1;33m[{val:02X}]\x1b[0m");
            } else {
                let _ = write!(out, "{val:02X} ");
            }
        }
        let _ = writeln!(
            out,
            "\n\x1b[1;36m{RULE}\x1b[0m\n\x1b[1;33m  COMANDOS: [S]tep  [C]ontinue [n]  [R]eset  [M]em <hex>  [H]alt  [Q]uit\x1b[0m"
        );
        out
    }

    pub fn render<B: MemoryBus>(&self, cpu: &Cpu<B>) {
        print!("{}", self.frame(cpu));
    }

    /// Run one parsed command; CPU errors go to [`TuiDashboard::status`].
    pub fn apply<B: MemoryBus>(&mut self, cpu: &mut Cpu<B>, command: TuiCommand) {
        self.status.clear();
        let result = match command {
            TuiCommand::Step => cpu.step().map(|_| ()),
            TuiCommand::Continue(n) => {
                let mut r = Ok(());
                for _ in 0..n {
                    if cpu.is_halted() {
                        break;
                    }
                    if let Err(e) = cpu.step() {
                        r = Err(e);
                        break;
                    }
                }
                r
            }
            TuiCommand::Reset => {
                cpu.reset();
                Ok(())
            }
            TuiCommand::Halt => {
                cpu.set_halted(true);
                Ok(())
            }
            TuiCommand::Mem(addr) => {
                self.mem_view_addr = addr;
                Ok(())
            }
            TuiCommand::Quit => {
                self.is_running = false;
                Ok(())
            }
        };
        if let Err(e) = result {
            self.status = e;
        }
    }

    /// Frame, prompt, command, until `q` or end of input.
    pub fn run_with<B: MemoryBus, R: BufRead, W: Write>(
        &mut self,
        cpu: &mut Cpu<B>,
        mut input: R,
        out: &mut W,
    ) -> io::Result<()> {
        let mut line = String::new();
        while self.is_running {
            write!(out, "{}\x1b[1;32m(TUI)>\x1b[0m ", self.frame(cpu))?;
            out.flush()?;
            line.clear();
            if input.read_line(&mut line)? == 0 {
                break;
            }
            match TuiCommand::parse(&line) {
                Ok(command) => self.apply(cpu, command),
                Err(e) => self.status = e,
            }
        }
        writeln!(out, "\x1b[0mSessão TUI encerrada.")
    }

    pub fn run_loop<B: MemoryBus>(&mut self, cpu: &mut Cpu<B>) -> io::Result<()> {
        self.run_with(cpu, io::stdin().lock(), &mut io::stdout())
    }
}

impl Default for TuiDashboard {
    fn default() -> Self {
        Self::new()
    }
}
