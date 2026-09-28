//! Generic single-board computer assembled from a text description.
//!
//! The description is a TOML subset: `key = value` lines, `#` comments, and
//! `[[ram]]`, `[[rom]]`, `[[device]]` tables that may repeat. Values are quoted
//! strings, integers (`0x1F`, `$1F` or decimal), `true`/`false`, or byte lists
//! (`[0xA9, 1]`).
//!
//! ```text
//! name = "Apple I compatible"
//! cpu = "6502"                  # 6502 | 65c02 | 2a03
//!
//! [[ram]]
//! start = 0x0000
//! end = 0x0FFF
//!
//! [[device]]
//! kind = "apple1-pia"           # pia6821 | via6522 | acia6551 | riot6532 | apple1-pia
//! name = "kbd"
//! start = 0xD010
//! end = 0xD013
//! irq = false                   # whether the device drives the IRQ line
//!
//! [[rom]]
//! start = 0xFE00
//! image = "apple1-monitor"      # or: bytes = [...], or: fill = 0xEA with size = N
//! ```
//!
//! Every address outside RAM, ROM and devices is open: reads `$FF`, ignores
//! writes. Ranges may not overlap, and a ROM must fit below `$10000`. Devices get
//! the offset from their `start`; the peripheral types mask it themselves.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::acia::Acia6551;
use crate::bus::{IoDevice, SystemBus};
use crate::cpu::Cpu;
use crate::pia::{Pia6821, Riot6532};
use crate::systems::apple1::Apple1Pia;
use crate::via::Via6522;
use crate::CpuVariant;

/// The Apple I-compatible preset: 4 KiB RAM, the keyboard/display PIA at
/// `$D010`, the crate's own monitor at `$FE00` (the same layout as
/// [`crate::systems::apple1::Apple1`], minus the RAM it leaves everywhere else).
pub const APPLE1_CONFIG: &str = r#"
name = "Apple I compatible"
cpu = "6502"

[[ram]]
start = 0x0000
end = 0x0FFF

[[device]]
kind = "apple1-pia"
name = "kbd"
start = 0xD010
end = 0xD013

[[rom]]
start = 0xFE00
image = "apple1-monitor"
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    /// 1-based line, 0 when the error is about the whole board.
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Str(String),
    Int(u32),
    Bool(bool),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Pia6821,
    Via6522,
    Acia6551,
    Riot6532,
    Apple1Pia,
}

impl DeviceKind {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pia6821" => Self::Pia6821,
            "via6522" => Self::Via6522,
            "acia6551" => Self::Acia6551,
            "riot6532" => Self::Riot6532,
            "apple1-pia" => Self::Apple1Pia,
            _ => return None,
        })
    }

    /// Smallest window that reaches every register.
    pub fn min_size(self) -> u32 {
        match self {
            Self::Pia6821 | Self::Acia6551 | Self::Apple1Pia => 4,
            Self::Via6522 => 16,
            Self::Riot6532 => 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSpec {
    pub kind: DeviceKind,
    pub name: String,
    pub start: u16,
    pub end: u16,
    pub irq: bool,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomSpec {
    pub start: u16,
    pub bytes: Vec<u8>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardConfig {
    pub name: String,
    pub cpu: CpuVariant,
    pub ram: Vec<(u16, u16)>,
    pub roms: Vec<RomSpec>,
    pub devices: Vec<DeviceSpec>,
}

/// One row of [`BoardConfig::memory_map`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapEntry {
    pub start: u16,
    pub end: u16,
    pub what: String,
}

fn err<T>(line: usize, message: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError {
        line,
        message: message.into(),
    })
}

fn parse_int(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(h, 16).ok()
    } else if let Some(h) = s.strip_prefix('$') {
        u32::from_str_radix(h, 16).ok()
    } else {
        s.parse().ok()
    }
}

fn parse_value(s: &str, line: usize) -> Result<Value, ConfigError> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix('"') {
        return match rest.strip_suffix('"') {
            Some(v) if !v.contains('"') => Ok(Value::Str(v.to_string())),
            _ => err(line, "unterminated string"),
        };
    }
    if let Some(rest) = s.strip_prefix('[') {
        let inner = rest
            .strip_suffix(']')
            .ok_or_else(|| ConfigError {
                line,
                message: "unterminated list".into(),
            })?
            .trim();
        if inner.is_empty() {
            return Ok(Value::Bytes(Vec::new()));
        }
        let mut out = Vec::new();
        for item in inner.split(',') {
            match parse_int(item) {
                Some(n) if n <= 0xFF => out.push(n as u8),
                _ => return err(line, format!("not a byte: {}", item.trim())),
            }
        }
        return Ok(Value::Bytes(out));
    }
    match s {
        "true" => return Ok(Value::Bool(true)),
        "false" => return Ok(Value::Bool(false)),
        _ => {}
    }
    parse_int(s).map(Value::Int).ok_or_else(|| ConfigError {
        line,
        message: format!("bad value: {s}"),
    })
}

struct Table {
    kind: String,
    line: usize,
    keys: HashMap<String, (Value, usize)>,
}

impl Table {
    fn take(&mut self, key: &str) -> Option<(Value, usize)> {
        self.keys.remove(key)
    }

    fn addr(&mut self, key: &str) -> Result<u16, ConfigError> {
        match self.take(key) {
            Some((Value::Int(n), l)) => {
                u16::try_from(n).or_else(|_| err(l, format!("{key} beyond $FFFF")))
            }
            Some((_, l)) => err(l, format!("{key} must be an integer")),
            None => err(self.line, format!("[[{}]] needs {key}", self.kind)),
        }
    }

    fn finish(self) -> Result<(), ConfigError> {
        if let Some((k, (_, l))) = self.keys.into_iter().min_by_key(|(_, (_, l))| *l) {
            return err(l, format!("unknown key {k}"));
        }
        Ok(())
    }
}

fn builtin_image(name: &str) -> Option<Vec<u8>> {
    match name {
        "apple1-monitor" => Some(crate::systems::apple1::monitor_image()),
        "kim1-monitor" => Some(crate::systems::kim1::monitor_rom()),
        _ => None,
    }
}

impl BoardConfig {
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let mut top = Table {
            kind: "board".into(),
            line: 0,
            keys: HashMap::new(),
        };
        let mut tables: Vec<Table> = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = i + 1;
            let content = match raw.find('#') {
                Some(p) if !raw[..p].contains('"') || raw[..p].matches('"').count() % 2 == 0 => {
                    &raw[..p]
                }
                _ => raw,
            }
            .trim();
            if content.is_empty() {
                continue;
            }
            if let Some(h) = content.strip_prefix("[[") {
                let kind = h
                    .strip_suffix("]]")
                    .ok_or_else(|| ConfigError {
                        line,
                        message: "bad table header".into(),
                    })?
                    .trim();
                if !matches!(kind, "ram" | "rom" | "device") {
                    return err(line, format!("unknown table [[{kind}]]"));
                }
                tables.push(Table {
                    kind: kind.into(),
                    line,
                    keys: HashMap::new(),
                });
                continue;
            }
            let (k, v) = content.split_once('=').ok_or_else(|| ConfigError {
                line,
                message: "expected key = value".into(),
            })?;
            let k = k.trim();
            if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return err(line, format!("bad key {k:?}"));
            }
            let v = parse_value(v, line)?;
            let t = tables.last_mut().unwrap_or(&mut top);
            if t.keys.insert(k.into(), (v, line)).is_some() {
                return err(line, format!("duplicate key {k}"));
            }
        }
        let name = match top.take("name") {
            Some((Value::Str(s), _)) => s,
            Some((_, l)) => return err(l, "name must be a string"),
            None => "SBC".into(),
        };
        let cpu = match top.take("cpu") {
            None => CpuVariant::NMOS,
            Some((Value::Str(s), l)) => match s.to_ascii_lowercase().as_str() {
                "6502" => CpuVariant::NMOS,
                "65c02" => CpuVariant::CMOS,
                "2a03" => CpuVariant::Ricoh2A03,
                _ => return err(l, format!("unknown cpu {s}")),
            },
            Some((_, l)) => return err(l, "cpu must be a string"),
        };
        top.finish()?;
        let mut cfg = BoardConfig {
            name,
            cpu,
            ram: Vec::new(),
            roms: Vec::new(),
            devices: Vec::new(),
        };
        for mut t in tables {
            match t.kind.as_str() {
                "ram" => {
                    let (s, e) = (t.addr("start")?, t.addr("end")?);
                    if s > e {
                        return err(t.line, "ram start above end");
                    }
                    cfg.ram.push((s, e));
                }
                "rom" => {
                    let start = t.addr("start")?;
                    let image = t.take("image");
                    let bytes = t.take("bytes");
                    let fill = t.take("fill");
                    let size = t.take("size");
                    let data = match (image, bytes, fill, size) {
                        (Some((Value::Str(n), l)), None, None, None) => builtin_image(&n)
                            .ok_or_else(|| ConfigError {
                                line: l,
                                message: format!("unknown image {n}"),
                            })?,
                        (None, Some((Value::Bytes(b), _)), None, None) => b,
                        (None, None, Some((Value::Int(f), l)), Some((Value::Int(n), _))) => {
                            if f > 0xFF {
                                return err(l, "fill must be a byte");
                            }
                            vec![f as u8; n as usize]
                        }
                        _ => {
                            return err(
                                t.line,
                                "[[rom]] needs one of image, bytes, or fill with size",
                            )
                        }
                    };
                    if data.is_empty() {
                        return err(t.line, "empty ROM");
                    }
                    if usize::from(start) + data.len() > 0x10000 {
                        return err(t.line, "ROM runs past $FFFF");
                    }
                    cfg.roms.push(RomSpec {
                        start,
                        bytes: data,
                        line: t.line,
                    });
                }
                _ => {
                    let kind = match t.take("kind") {
                        Some((Value::Str(k), l)) => {
                            DeviceKind::parse(&k).ok_or_else(|| ConfigError {
                                line: l,
                                message: format!("unknown device kind {k}"),
                            })?
                        }
                        _ => return err(t.line, "[[device]] needs kind = \"...\""),
                    };
                    let (start, end) = (t.addr("start")?, t.addr("end")?);
                    if start > end {
                        return err(t.line, "device start above end");
                    }
                    if u32::from(end - start) + 1 < kind.min_size() {
                        return err(
                            t.line,
                            format!("{kind:?} needs at least {} bytes", kind.min_size()),
                        );
                    }
                    let name = match t.take("name") {
                        Some((Value::Str(n), _)) => n,
                        Some((_, l)) => return err(l, "name must be a string"),
                        None => format!("{kind:?}@{start:04X}").to_ascii_lowercase(),
                    };
                    if cfg.devices.iter().any(|d| d.name == name) {
                        return err(t.line, format!("duplicate device name {name}"));
                    }
                    let irq = match t.take("irq") {
                        Some((Value::Bool(b), _)) => b,
                        Some((_, l)) => return err(l, "irq must be true or false"),
                        None => false,
                    };
                    cfg.devices.push(DeviceSpec {
                        kind,
                        name,
                        start,
                        end,
                        irq,
                        line: t.line,
                    });
                }
            }
            t.finish()?;
        }
        cfg.check_overlaps()?;
        Ok(cfg)
    }

    fn ranges(&self) -> Vec<(u16, u16, String, usize)> {
        let mut v: Vec<_> = self
            .ram
            .iter()
            .map(|&(s, e)| (s, e, "ram".to_string(), 0))
            .collect();
        v.extend(self.roms.iter().map(|r| {
            (
                r.start,
                r.start + (r.bytes.len() - 1) as u16,
                "rom".into(),
                r.line,
            )
        }));
        v.extend(
            self.devices
                .iter()
                .map(|d| (d.start, d.end, d.name.clone(), d.line)),
        );
        v.sort();
        v
    }

    fn check_overlaps(&self) -> Result<(), ConfigError> {
        for w in self.ranges().windows(2) {
            if w[1].0 <= w[0].1 {
                return err(
                    w[1].3.max(w[0].3),
                    format!(
                        "{} ${:04X}-${:04X} overlaps {} ${:04X}-${:04X}",
                        w[1].2, w[1].0, w[1].1, w[0].2, w[0].0, w[0].1
                    ),
                );
            }
        }
        Ok(())
    }

    /// The whole 64 KiB in address order, open gaps included.
    pub fn memory_map(&self) -> Vec<MapEntry> {
        let mut out = Vec::new();
        let mut next = 0u32;
        for (s, e, what, _) in self.ranges() {
            if u32::from(s) > next {
                out.push(MapEntry {
                    start: next as u16,
                    end: s - 1,
                    what: "open".into(),
                });
            }
            out.push(MapEntry {
                start: s,
                end: e,
                what,
            });
            next = u32::from(e) + 1;
        }
        if next <= 0xFFFF {
            out.push(MapEntry {
                start: next as u16,
                end: 0xFFFF,
                what: "open".into(),
            });
        }
        out
    }

    pub fn build(&self) -> Result<Sbc, String> {
        let mut bus = SystemBus::new();
        let mut handles = Vec::new();
        for d in &self.devices {
            let (h, dev): (Handle, Box<dyn IoDevice>) = match d.kind {
                DeviceKind::Pia6821 => {
                    let rc = Rc::new(RefCell::new(Pia6821::new()));
                    (Handle::Pia(rc.clone()), Box::new(Shared(rc)))
                }
                DeviceKind::Via6522 => {
                    let rc = Rc::new(RefCell::new(Via6522::new()));
                    (Handle::Via(rc.clone()), Box::new(Shared(rc)))
                }
                DeviceKind::Acia6551 => {
                    let rc = Rc::new(RefCell::new(Acia6551::new()));
                    (Handle::Acia(rc.clone()), Box::new(Shared(rc)))
                }
                DeviceKind::Riot6532 => {
                    let rc = Rc::new(RefCell::new(Riot6532::new()));
                    (Handle::Riot(rc.clone()), Box::new(Shared(rc)))
                }
                DeviceKind::Apple1Pia => {
                    let rc = Rc::new(RefCell::new(Apple1Pia::new()));
                    (Handle::Apple1(rc.clone()), Box::new(Shared(rc)))
                }
            };
            bus.try_map_device(d.start, d.end, BoxDev(dev))
                .map_err(|e| format!("{}: {e:?}", d.name))?;
            handles.push((d.name.clone(), h, d.irq));
        }
        for r in &self.roms {
            bus.try_map_rom(r.start, &r.bytes)
                .map_err(|e| format!("rom at {:04X}: {e:?}", r.start))?;
        }
        for m in self.memory_map() {
            if m.what == "open" {
                bus.try_map_device(m.start, m.end, OpenBus)
                    .map_err(|e| format!("open bus: {e:?}"))?;
            }
        }
        let mut cpu = Cpu::with_variant(bus, self.cpu);
        cpu.reset();
        Ok(Sbc {
            cpu,
            name: self.name.clone(),
            devices: handles,
        })
    }
}

/// A device on the board, shared with the bus.
#[derive(Clone)]
pub enum Handle {
    Pia(Rc<RefCell<Pia6821>>),
    Via(Rc<RefCell<Via6522>>),
    Acia(Rc<RefCell<Acia6551>>),
    Riot(Rc<RefCell<Riot6532>>),
    Apple1(Rc<RefCell<Apple1Pia>>),
}

impl Handle {
    fn tick(&self) {
        match self {
            Handle::Via(v) => v.borrow_mut().tick(),
            Handle::Acia(a) => a.borrow_mut().tick(),
            Handle::Riot(r) => r.borrow_mut().tick(),
            Handle::Pia(_) | Handle::Apple1(_) => {}
        }
    }

    fn irq(&self) -> bool {
        match self {
            Handle::Via(v) => v.borrow().irq_asserted(),
            Handle::Acia(a) => a.borrow().irq_asserted(),
            Handle::Riot(r) => r.borrow().irq_asserted(),
            Handle::Pia(p) => p.borrow().irq_asserted(),
            Handle::Apple1(_) => false,
        }
    }
}

struct Shared<T>(Rc<RefCell<T>>);

impl<T: IoDevice> IoDevice for Shared<T> {
    fn read(&mut self, offset: u16) -> u8 {
        self.0.borrow_mut().read(offset)
    }
    fn write(&mut self, offset: u16, value: u8) {
        self.0.borrow_mut().write(offset, value);
    }
    fn peek(&self, offset: u16) -> u8 {
        self.0.borrow().peek(offset)
    }
}

struct BoxDev(Box<dyn IoDevice>);

impl IoDevice for BoxDev {
    fn read(&mut self, offset: u16) -> u8 {
        self.0.read(offset)
    }
    fn write(&mut self, offset: u16, value: u8) {
        self.0.write(offset, value);
    }
    fn peek(&self, offset: u16) -> u8 {
        self.0.peek(offset)
    }
}

/// Unpopulated address space.
struct OpenBus;

impl IoDevice for OpenBus {
    fn read(&mut self, _: u16) -> u8 {
        0xFF
    }
    fn write(&mut self, _: u16, _: u8) {}
    fn peek(&self, _: u16) -> u8 {
        0xFF
    }
}

/// A running board built from a [`BoardConfig`].
pub struct Sbc {
    pub cpu: Cpu<SystemBus>,
    pub name: String,
    devices: Vec<(String, Handle, bool)>,
}

impl Sbc {
    pub fn from_text(text: &str) -> Result<Self, String> {
        BoardConfig::parse(text).map_err(|e| e.to_string())?.build()
    }

    pub fn device(&self, name: &str) -> Option<&Handle> {
        self.devices.iter().find(|d| d.0 == name).map(|d| &d.1)
    }

    /// IRQ line: any device configured with `irq = true` asserting.
    pub fn irq(&self) -> bool {
        self.devices.iter().any(|(_, h, wired)| *wired && h.irq())
    }

    /// Run `n` instructions; devices tick once per CPU cycle, and a wired IRQ is
    /// taken at the instruction boundary when I is clear.
    pub fn run_instructions(&mut self, n: u64) -> Result<u64, String> {
        let mut total = 0;
        for _ in 0..n {
            let before = self.cpu.cycles;
            if self.irq() && !self.cpu.p.get(crate::flags::bits::I) {
                self.cpu.irq();
            } else {
                self.cpu.step()?;
            }
            let spent = self.cpu.cycles - before;
            for _ in 0..spent {
                for (_, h, _) in &self.devices {
                    h.tick();
                }
            }
            total += spent;
        }
        Ok(total)
    }

    pub fn run_cycles(&mut self, cycles: u64) -> Result<(), String> {
        let mut done = 0;
        while done < cycles {
            done += self.run_instructions(1)?.max(1);
        }
        Ok(())
    }
}
