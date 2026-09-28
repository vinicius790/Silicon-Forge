//! Save and restore CPU registers, a flat 64 KiB RAM image, a 6522, and a 6551.
//!
//! The encoder is hand-written so the crate stays free of serde. Snapshot version 2
//! is a single JSON object. Version 3 is the same object plus `scanline` (the
//! fantasy-console line, `0..32` as stored by the caller) and the `halted` flag
//! that version 2 already carried. The RAM image is a lowercase hex string of
//! exactly 131072 digits (65536 bytes). A document without a `version` field is
//! rejected. Before version 8, IRQ pin state and an in-progress bus-cycle sequence
//! are not part of the image; restoring abandons a cycle sequence and clears the
//! NMI latch. `capture` still writes version 2 so older tests keep
//! loading. [`MachineImage::at_scanline`] writes version 3.
//! [`MachineImage::at_console`] writes version 4, which adds the 16 palette
//! bytes and the 256 name-table bytes. [`MachineImage::at_next_instruction`]
//! writes version 5 and stores one disassembly line for the instruction at `PC`.
//! [`MachineImage::with_wait_state`] writes version 6 and adds `waiting` (65C02
//! `WAI` sleep). `STP` is the `halted` flag every version already carries.
//! [`MachineImage::with_phase_state`] writes version 7 and adds `phi1_open`, a PHI1
//! that `Cpu::step_phase` opened on an instruction boundary (`null` otherwise).
//! [`MachineImage::with_cycle_state`] writes version 8 and adds `cycle_state`, the 36
//! bytes of [`Cpu::cycle_state`]: a capture mid-instruction resumes exactly.
//! Version 0, 9 and 99 are rejected. Versions 2 through 7 still load, restore
//! `waiting` as false before 6, leave no PHI1 open before 7, and drop an
//! in-progress sequence before 8. The VIA's held
//! timer flags (`t1_flag_due`, `t2_flag_due`) are optional in every version.
//! Their listing is empty; v2/v3 palette is the identity and the name table is zeros.

use crate::acia::{Acia6551, AciaState};
use crate::bus::FlatMemory;
use crate::cpu::Cpu;
use crate::flags::Status;
use crate::via::{Via6522, ViaState};

pub const SNAPSHOT_VERSION: u32 = 2;
pub const SNAPSHOT_VERSION_V3: u32 = 3;
pub const SNAPSHOT_VERSION_V4: u32 = 4;
pub const SNAPSHOT_VERSION_V5: u32 = 5;
pub const SNAPSHOT_VERSION_V6: u32 = 6;
pub const SNAPSHOT_VERSION_V7: u32 = 7;
pub const SNAPSHOT_VERSION_V8: u32 = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineImage {
    pub version: u32,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    pub p: u8,
    pub cycles: u64,
    pub halted: bool,
    /// Fantasy-console scanline. Zero unless the image is version 3.
    pub scanline: u8,
    pub ram: Vec<u8>,
    pub via: ViaState,
    pub acia: AciaState,
    /// Fantasy-console palette. Identity unless the image is version 4.
    pub palette: [u8; 16],
    /// Fantasy-console name table, 256 bytes. Zeros unless the image is version 4.
    pub name_table: [u8; 256],
    /// Disassembly of the instruction at `PC`. Empty unless the image is version 5.
    pub next_listing: String,
    /// 65C02 `WAI` sleep. False unless the image is version 6 or later.
    pub waiting: bool,
    /// Boundary PHI1 left open by `step_phase`: address and R/W. `None` before version 7.
    pub phi1_open: Option<(u16, bool)>,
    /// [`Cpu::cycle_state`] bytes (36). `None` before version 8.
    pub cycle_state: Option<Vec<u8>>,
}

impl MachineImage {
    pub fn capture(cpu: &Cpu<FlatMemory>) -> Self {
        Self::capture_with(cpu, &Via6522::new(), &Acia6551::new())
    }

    pub fn capture_with(cpu: &Cpu<FlatMemory>, via: &Via6522, acia: &Acia6551) -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            a: cpu.a,
            x: cpu.x,
            y: cpu.y,
            sp: cpu.sp,
            pc: cpu.pc,
            p: cpu.p.raw(),
            cycles: cpu.cycles,
            halted: cpu.is_halted(),
            scanline: 0,
            ram: cpu.bus.data.to_vec(),
            via: via.state(),
            acia: acia.state(),
            palette: identity_palette(),
            name_table: [0; 256],
            next_listing: String::new(),
            waiting: false,
            phi1_open: None,
            cycle_state: None,
        }
    }

    pub fn restore(&self, cpu: &mut Cpu<FlatMemory>) -> Result<(), SnapshotError> {
        self.restore_cpu(cpu)
    }

    pub fn restore_cpu(&self, cpu: &mut Cpu<FlatMemory>) -> Result<(), SnapshotError> {
        if !supported_version(self.version) {
            return Err(SnapshotError::UnsupportedVersion(u64::from(self.version)));
        }
        if self.ram.len() != 65536 {
            return Err(SnapshotError::Length {
                what: "RAM",
                expected: 65536,
                got: self.ram.len(),
            });
        }
        // The only fallible step runs first, so an error leaves the CPU untouched.
        match &self.cycle_state {
            Some(bytes) => cpu
                .load_cycle_state(bytes)
                .map_err(SnapshotError::CycleState)?,
            None => {
                cpu.abandon_cycle_sequence();
                cpu.reopen_phi1(self.phi1_open);
            }
        }
        cpu.a = self.a;
        cpu.x = self.x;
        cpu.y = self.y;
        cpu.sp = self.sp;
        cpu.pc = self.pc;
        cpu.p = Status(self.p);
        cpu.cycles = self.cycles;
        cpu.set_halted(self.halted);
        cpu.set_waiting(self.waiting);
        cpu.bus.data.copy_from_slice(&self.ram);
        Ok(())
    }

    pub fn restore_with(
        &self,
        cpu: &mut Cpu<FlatMemory>,
        via: &mut Via6522,
        acia: &mut Acia6551,
    ) -> Result<(), SnapshotError> {
        self.restore_cpu(cpu)?;
        via.load_state(&self.via);
        acia.load_state(&self.acia);
        Ok(())
    }

    /// Version 3 image: keeps every captured field, records `scanline`, and
    /// leaves `halted` as it was captured (pass the CPU already halted, or set
    /// the field, before calling this).
    pub fn at_scanline(mut self, scanline: u8) -> Self {
        self.version = SNAPSHOT_VERSION_V3;
        self.scanline = scanline;
        self
    }

    /// Version 4 image: keeps the captured fields and records the palette and name table.
    pub fn at_console(mut self, palette: [u8; 16], name_table: [u8; 256]) -> Self {
        self.version = SNAPSHOT_VERSION_V4;
        self.palette = palette;
        self.name_table = name_table;
        self
    }

    /// Version 5 image: one disassembly line of the instruction at `PC` in `ram`.
    pub fn at_next_instruction(mut self) -> Self {
        let line = crate::disasm::disasm_at(&self.ram, self.pc);
        self.version = SNAPSHOT_VERSION_V5;
        self.next_listing = format!("{:04X}  {}  {}", self.pc, line.bytes_hex, line.text);
        self
    }

    /// Version 6 image: the v5 fields plus the `WAI` state of `cpu`.
    pub fn with_wait_state(mut self, cpu: &Cpu<FlatMemory>) -> Self {
        self = self.at_next_instruction();
        self.version = SNAPSHOT_VERSION_V6;
        self.waiting = cpu.is_waiting();
        self
    }

    /// Version 7 image: the v6 fields plus a PHI1 that `step_phase` opened on an
    /// instruction boundary. Device clocks already ran for that PHI1; restore resumes
    /// with its PHI2 and does not clock them again.
    pub fn with_phase_state(mut self, cpu: &Cpu<FlatMemory>) -> Self {
        self = self.with_wait_state(cpu);
        self.version = SNAPSHOT_VERSION_V7;
        self.phi1_open = cpu.open_phi1();
        self
    }

    /// Version 8 image: the v7 fields plus [`Cpu::cycle_state`], so a capture in the
    /// middle of an instruction, an interrupt sequence, or between two half cycles
    /// resumes on the exact next bus cycle.
    pub fn with_cycle_state(mut self, cpu: &Cpu<FlatMemory>) -> Self {
        self = self.with_phase_state(cpu);
        self.version = SNAPSHOT_VERSION_V8;
        self.cycle_state = Some(cpu.cycle_state());
        self
    }

    pub fn to_json(&self) -> String {
        let mut ram = String::with_capacity(self.ram.len() * 2);
        for byte in &self.ram {
            ram.push_str(&format!("{byte:02x}"));
        }
        let scanline = if self.version >= SNAPSHOT_VERSION_V3 {
            format!(",\"scanline\":{}", self.scanline)
        } else {
            String::new()
        };
        let video = if self.version >= SNAPSHOT_VERSION_V4 {
            format!(
                ",\"palette\":\"{}\",\"name_table\":\"{}\"",
                hex_bytes(&self.palette),
                hex_bytes(&self.name_table)
            )
        } else {
            String::new()
        };
        let listing = if self.version >= SNAPSHOT_VERSION_V5 {
            format!(",\"next_listing\":\"{}\"", self.next_listing)
        } else {
            String::new()
        };
        let wait = if self.version >= SNAPSHOT_VERSION_V6 {
            format!(",\"waiting\":{}", bool_json(self.waiting))
        } else {
            String::new()
        };
        let phase = if self.version >= SNAPSHOT_VERSION_V7 {
            match self.phi1_open {
                Some((address, write)) => format!(
                    ",\"phi1_open\":{{\"address\":{address},\"write\":{}}}",
                    bool_json(write)
                ),
                None => ",\"phi1_open\":null".to_string(),
            }
        } else {
            String::new()
        };
        let cycle = match (&self.cycle_state, self.version >= SNAPSHOT_VERSION_V8) {
            (Some(bytes), true) => format!(",\"cycle_state\":\"{}\"", hex_bytes(bytes)),
            _ => String::new(),
        };
        format!(
            concat!(
                "{{\"version\":{},\"a\":{},\"x\":{},\"y\":{},\"sp\":{},\"pc\":{},\"p\":{},",
                "\"cycles\":{},\"halted\":{halted}{scanline}{video}{listing}{wait}{phase}{cycle},\"ram\":\"{ram}\",",
                "\"via\":{via},\"acia\":{acia}}}"
            ),
            self.version,
            self.a,
            self.x,
            self.y,
            self.sp,
            self.pc,
            self.p,
            self.cycles,
            halted = if self.halted { "true" } else { "false" },
            scanline = scanline,
            video = video,
            listing = listing,
            wait = wait,
            phase = phase,
            cycle = cycle,
            ram = ram,
            via = via_json(&self.via),
            acia = acia_json(&self.acia),
        )
    }

    pub fn from_json(text: &str) -> Result<Self, SnapshotError> {
        let value = parse_json(text)?;
        let obj = value
            .as_object()
            .ok_or(SnapshotError::NotObject("machine image"))?;
        if !obj.iter().any(|(key, _)| key == "version") {
            return Err(SnapshotError::MissingField("version".into()));
        }
        let version = required(obj, "version")?.as_u32()?;
        if !supported_version(version) {
            return Err(SnapshotError::UnsupportedVersion(u64::from(version)));
        }
        let via = parse_via(required(obj, "via")?)?;
        let acia = parse_acia(required(obj, "acia")?)?;
        let image = Self {
            version,
            a: required(obj, "a")?.as_u8()?,
            x: required(obj, "x")?.as_u8()?,
            y: required(obj, "y")?.as_u8()?,
            sp: required(obj, "sp")?.as_u8()?,
            pc: required(obj, "pc")?.as_u16()?,
            p: required(obj, "p")?.as_u8()?,
            cycles: required(obj, "cycles")?.as_u64()?,
            halted: required(obj, "halted")?.as_bool()?,
            scanline: if version >= SNAPSHOT_VERSION_V3 {
                required(obj, "scanline")?.as_u8()?
            } else {
                0
            },
            ram: decode_ram(required(obj, "ram")?.as_str()?)?,
            via,
            acia,
            palette: if version >= SNAPSHOT_VERSION_V4 {
                decode_fixed(required(obj, "palette")?.as_str()?, 16)?
            } else {
                identity_palette()
            },
            name_table: if version >= SNAPSHOT_VERSION_V4 {
                decode_fixed(required(obj, "name_table")?.as_str()?, 256)?
            } else {
                [0; 256]
            },
            next_listing: if version >= SNAPSHOT_VERSION_V5 {
                required(obj, "next_listing")?.as_str()?.to_string()
            } else {
                String::new()
            },
            waiting: if version >= SNAPSHOT_VERSION_V6 {
                required(obj, "waiting")?.as_bool()?
            } else {
                false
            },
            phi1_open: if version >= SNAPSHOT_VERSION_V7 {
                match required(obj, "phi1_open")? {
                    Value::Null => None,
                    open => {
                        let fields = open
                            .as_object()
                            .ok_or(SnapshotError::NotObject("phi1_open"))?;
                        Some((
                            required(fields, "address")?.as_u16()?,
                            required(fields, "write")?.as_bool()?,
                        ))
                    }
                }
            } else {
                None
            },
            cycle_state: if version >= SNAPSHOT_VERSION_V8 {
                Some(decode_counted(required(obj, "cycle_state")?.as_str()?, 36)?)
            } else {
                None
            },
        };
        Ok(image)
    }
}

fn supported_version(version: u32) -> bool {
    version == SNAPSHOT_VERSION
        || version == SNAPSHOT_VERSION_V3
        || version == SNAPSHOT_VERSION_V4
        || version == SNAPSHOT_VERSION_V5
        || version == SNAPSHOT_VERSION_V6
        || version == SNAPSHOT_VERSION_V7
        || version == SNAPSHOT_VERSION_V8
}

fn identity_palette() -> [u8; 16] {
    let mut palette = [0u8; 16];
    for (index, slot) in palette.iter_mut().enumerate() {
        *slot = index as u8;
    }
    palette
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn decode_fixed<const N: usize>(hex: &str, expected: usize) -> Result<[u8; N], SnapshotError> {
    let bytes = decode_counted(hex, expected)?;
    let mut out = [0u8; N];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn via_json(via: &ViaState) -> String {
    format!(
        concat!(
            "{{\"ddra\":{},\"ddrb\":{},\"ora\":{},\"orb\":{},\"pa_in\":{},\"pb_in\":{},",
            "\"t1_latch\":{},\"t1_counter\":{},\"t1_running\":{},\"t1_flag\":{},",
            "\"t2_latch\":{},\"t2_counter\":{},\"t2_running\":{},\"t2_flag\":{},",
            "\"ifr_sr\":{},\"ca1_flag\":{},\"cb1_flag\":{},\"ca1_high\":{},\"cb1_high\":{},\"pb6_high\":{},\"pb7\":{},\"sr_in\":{},\"ier\":{},\"acr\":{},\"pcr\":{},\"sr\":{},\"sr_left\":{},\"sr_out\":{},\"t1_reload\":{},\"sr_t2_left\":{},\"sr_clock_high\":{},\"t1_flag_due\":{},\"t2_flag_due\":{},\"pa_driven\":{},\"pb_driven\":{},\"pa_load_low\":{},\"pb_load_low\":{},\"ira_latch\":{},\"irb_latch\":{}}}"
        ),
        via.ddra,
        via.ddrb,
        via.ora,
        via.orb,
        via.pa_in,
        via.pb_in,
        via.t1_latch,
        via.t1_counter,
        bool_json(via.t1_running),
        bool_json(via.t1_flag),
        via.t2_latch,
        via.t2_counter,
        bool_json(via.t2_running),
        bool_json(via.t2_flag),
        bool_json(via.ifr_sr),
        bool_json(via.ca1_flag),
        bool_json(via.cb1_flag),
        bool_json(via.ca1_high),
        bool_json(via.cb1_high),
        bool_json(via.pb6_high),
        bool_json(via.pb7),
        bool_json(via.sr_in),
        via.ier,
        via.acr,
        via.pcr,
        via.sr,
        via.sr_left,
        via.sr_out,
        bool_json(via.t1_reload),
        via.sr_t2_left,
        bool_json(via.sr_clock_high),
        bool_json(via.t1_flag_due),
        bool_json(via.t2_flag_due),
        via.pa_driven,
        via.pb_driven,
        via.pa_load_low,
        via.pb_load_low,
        via.ira_latch,
        via.irb_latch,
    )
}

fn acia_json(acia: &AciaState) -> String {
    format!(
        concat!(
            "{{\"command\":{},\"control\":{},\"overrun\":{},\"loopback\":{},",
            "\"baud_period\":{},\"tx_left\":{},\"rx_left\":{},",
            "\"tx\":{},\"rx\":{},\"rx_waiting\":{},\"irq\":{},",
            "\"dcd_high\":{},\"dsr_high\":{},\"cts_high\":{},\"echoed\":\"{}\",",
            "\"crystal_cpu_hz\":{},\"crystal_acc\":{},\"external_clock\":{},",
            "\"tx_line\":\"{}\",\"rx_line\":\"{}\"}}"
        ),
        acia.command,
        acia.control,
        bool_json(acia.overrun),
        bool_json(acia.loopback),
        acia.baud_period,
        acia.tx_left,
        acia.rx_left,
        opt_json(acia.tx),
        opt_json(acia.rx),
        opt_json(acia.rx_waiting),
        bool_json(acia.irq),
        bool_json(acia.dcd_high),
        bool_json(acia.dsr_high),
        bool_json(acia.cts_high),
        hex_bytes(&acia.echoed),
        acia.crystal_cpu_hz
            .map_or_else(|| "null".to_string(), |hz| hz.to_string()),
        acia.crystal_acc,
        bool_json(acia.external_clock),
        acia.tx_line
            .iter()
            .map(|&b| if b { '1' } else { '0' })
            .collect::<String>(),
        hex_bytes(&acia.rx_line),
    )
}

fn bool_json(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn opt_json(value: Option<u8>) -> String {
    match value {
        Some(byte) => byte.to_string(),
        None => "null".into(),
    }
}

/// Field added after the format shipped; images written before it omit it.
fn optional<'a>(obj: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    obj.iter().find(|(k, _)| k == key).map(|(_, value)| value)
}

fn required<'a>(obj: &'a [(String, Value)], key: &str) -> Result<&'a Value, SnapshotError> {
    obj.iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
        .ok_or_else(|| SnapshotError::MissingField(key.into()))
}

fn parse_via(value: &Value) -> Result<ViaState, SnapshotError> {
    let obj = value.as_object().ok_or(SnapshotError::NotObject("via"))?;
    Ok(ViaState {
        ddra: required(obj, "ddra")?.as_u8()?,
        ddrb: required(obj, "ddrb")?.as_u8()?,
        ora: required(obj, "ora")?.as_u8()?,
        orb: required(obj, "orb")?.as_u8()?,
        pa_in: required(obj, "pa_in")?.as_u8()?,
        pb_in: required(obj, "pb_in")?.as_u8()?,
        t1_latch: required(obj, "t1_latch")?.as_u16()?,
        t1_counter: required(obj, "t1_counter")?.as_u16()?,
        t1_running: required(obj, "t1_running")?.as_bool()?,
        t1_flag: required(obj, "t1_flag")?.as_bool()?,
        t2_latch: required(obj, "t2_latch")?.as_u16()?,
        t2_counter: required(obj, "t2_counter")?.as_u16()?,
        t2_running: required(obj, "t2_running")?.as_bool()?,
        t2_flag: required(obj, "t2_flag")?.as_bool()?,
        ifr_sr: required(obj, "ifr_sr")?.as_bool()?,
        ca1_flag: required(obj, "ca1_flag")?.as_bool()?,
        cb1_flag: required(obj, "cb1_flag")?.as_bool()?,
        ca1_high: required(obj, "ca1_high")?.as_bool()?,
        cb1_high: required(obj, "cb1_high")?.as_bool()?,
        pb6_high: required(obj, "pb6_high")?.as_bool()?,
        pb7: required(obj, "pb7")?.as_bool()?,
        sr_in: required(obj, "sr_in")?.as_bool()?,
        ier: required(obj, "ier")?.as_u8()?,
        acr: required(obj, "acr")?.as_u8()?,
        pcr: required(obj, "pcr")?.as_u8()?,
        sr: required(obj, "sr")?.as_u8()?,
        sr_left: required(obj, "sr_left")?.as_u8()?,
        sr_out: required(obj, "sr_out")?.as_u8()?,
        t1_reload: optional(obj, "t1_reload").map_or(Ok(false), Value::as_bool)?,
        sr_t2_left: optional(obj, "sr_t2_left").map_or(Ok(0), Value::as_u16)?,
        sr_clock_high: optional(obj, "sr_clock_high").map_or(Ok(true), Value::as_bool)?,
        t1_flag_due: optional(obj, "t1_flag_due").map_or(Ok(false), Value::as_bool)?,
        t2_flag_due: optional(obj, "t2_flag_due").map_or(Ok(false), Value::as_bool)?,
        pa_driven: optional(obj, "pa_driven").map_or(Ok(0xFF), Value::as_u8)?,
        pb_driven: optional(obj, "pb_driven").map_or(Ok(0xFF), Value::as_u8)?,
        pa_load_low: optional(obj, "pa_load_low").map_or(Ok(0), Value::as_u8)?,
        pb_load_low: optional(obj, "pb_load_low").map_or(Ok(0), Value::as_u8)?,
        ira_latch: optional(obj, "ira_latch").map_or(Ok(0), Value::as_u8)?,
        irb_latch: optional(obj, "irb_latch").map_or(Ok(0), Value::as_u8)?,
    })
}

fn parse_acia(value: &Value) -> Result<AciaState, SnapshotError> {
    let obj = value.as_object().ok_or(SnapshotError::NotObject("acia"))?;
    Ok(AciaState {
        command: required(obj, "command")?.as_u8()?,
        control: required(obj, "control")?.as_u8()?,
        overrun: required(obj, "overrun")?.as_bool()?,
        loopback: required(obj, "loopback")?.as_bool()?,
        baud_period: required(obj, "baud_period")?.as_u32()?,
        tx_left: required(obj, "tx_left")?.as_u32()?,
        rx_left: required(obj, "rx_left")?.as_u32()?,
        tx: required(obj, "tx")?.as_opt_u8()?,
        rx: required(obj, "rx")?.as_opt_u8()?,
        rx_waiting: required(obj, "rx_waiting")?.as_opt_u8()?,
        irq: optional(obj, "irq").map_or(Ok(false), Value::as_bool)?,
        dcd_high: optional(obj, "dcd_high").map_or(Ok(false), Value::as_bool)?,
        dsr_high: optional(obj, "dsr_high").map_or(Ok(false), Value::as_bool)?,
        cts_high: optional(obj, "cts_high").map_or(Ok(false), Value::as_bool)?,
        echoed: match optional(obj, "echoed") {
            Some(value) => {
                let hex = value.as_str()?;
                decode_counted(hex, hex.len() / 2)?
            }
            None => Vec::new(),
        },
        crystal_cpu_hz: match optional(obj, "crystal_cpu_hz") {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.as_u32()?),
        },
        crystal_acc: optional(obj, "crystal_acc").map_or(Ok(0), Value::as_u64)?,
        external_clock: optional(obj, "external_clock").map_or(Ok(false), Value::as_bool)?,
        tx_line: match optional(obj, "tx_line") {
            None => Vec::new(),
            Some(v) => v
                .as_str()?
                .bytes()
                .map(|c| match c {
                    b'0' => Ok(false),
                    b'1' => Ok(true),
                    _ => Err(SnapshotError::WrongType("tx_line of 0/1 digits")),
                })
                .collect::<Result<_, _>>()?,
        },
        rx_line: match optional(obj, "rx_line") {
            None => [0; 8],
            Some(v) => {
                let packed: [u8; 8] = decode_fixed(v.as_str()?, 8)?;
                if !Acia6551::rx_line_valid(packed) {
                    return Err(SnapshotError::OutOfRange {
                        kind: "rx_line",
                        value: u64::from(packed[0]),
                    });
                }
                packed
            }
        },
    })
}

fn decode_ram(hex: &str) -> Result<Vec<u8>, SnapshotError> {
    decode_counted(hex, 65536)
}

fn decode_counted(hex: &str, expected_bytes: usize) -> Result<Vec<u8>, SnapshotError> {
    if hex.len() != expected_bytes * 2 {
        return Err(SnapshotError::Length {
            what: "hex",
            expected: expected_bytes * 2,
            got: hex.len(),
        });
    }
    let mut ram = Vec::with_capacity(65536);
    let bytes = hex.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_val(bytes[i])?;
        let lo = hex_val(bytes[i + 1])?;
        ram.push((hi << 4) | lo);
        i += 2;
    }
    Ok(ram)
}

fn hex_val(byte: u8) -> Result<u8, SnapshotError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(SnapshotError::BadHex(byte)),
    }
}

#[derive(Debug, Clone)]
enum Value {
    Null,
    Bool(bool),
    Number(u64),
    String(String),
    Object(Vec<(String, Value)>),
}

impl Value {
    fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(fields) => Some(fields),
            _ => None,
        }
    }

    fn as_bool(&self) -> Result<bool, SnapshotError> {
        match self {
            Value::Bool(value) => Ok(*value),
            _ => Err(SnapshotError::WrongType("true or false")),
        }
    }

    fn as_u64(&self) -> Result<u64, SnapshotError> {
        match self {
            Value::Number(value) => Ok(*value),
            _ => Err(SnapshotError::WrongType("number")),
        }
    }

    fn as_u32(&self) -> Result<u32, SnapshotError> {
        let value = self.as_u64()?;
        u32::try_from(value).map_err(|_| SnapshotError::OutOfRange {
            kind: "number",
            value,
        })
    }

    fn as_u16(&self) -> Result<u16, SnapshotError> {
        let value = self.as_u64()?;
        u16::try_from(value).map_err(|_| SnapshotError::OutOfRange {
            kind: "word",
            value,
        })
    }

    fn as_u8(&self) -> Result<u8, SnapshotError> {
        let value = self.as_u64()?;
        u8::try_from(value).map_err(|_| SnapshotError::OutOfRange {
            kind: "byte",
            value,
        })
    }

    fn as_opt_u8(&self) -> Result<Option<u8>, SnapshotError> {
        match self {
            Value::Null => Ok(None),
            Value::Number(value) => {
                u8::try_from(*value)
                    .map(Some)
                    .map_err(|_| SnapshotError::OutOfRange {
                        kind: "byte",
                        value: *value,
                    })
            }
            _ => Err(SnapshotError::WrongType("a byte or null")),
        }
    }

    fn as_str(&self) -> Result<&str, SnapshotError> {
        match self {
            Value::String(text) => Ok(text),
            _ => Err(SnapshotError::WrongType("string")),
        }
    }
}

fn parse_json(text: &str) -> Result<Value, SnapshotError> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        index: 0,
    };
    let value = parser.parse_value()?;
    parser.skip();
    if parser.index != parser.bytes.len() {
        return Err(SnapshotError::Syntax(
            "trailing data after machine image".into(),
        ));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl Parser<'_> {
    fn parse_value(&mut self) -> Result<Value, SnapshotError> {
        self.skip();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') | Some(b'f') => Ok(Value::Bool(self.bool_value()?)),
            Some(b'n') => self.null_value(),
            Some(b'0'..=b'9') => Ok(Value::Number(self.u64_value()?)),
            _ => Err(SnapshotError::WrongType("a JSON value")),
        }
    }

    fn parse_object(&mut self) -> Result<Value, SnapshotError> {
        self.expect(b'{')?;
        let mut fields = Vec::new();
        loop {
            self.skip();
            if self.peek() == Some(b'}') {
                self.bump();
                break;
            }
            if !fields.is_empty() {
                self.expect(b',')?;
                self.skip();
            }
            let key = self.string()?;
            self.skip();
            self.expect(b':')?;
            let value = self.parse_value()?;
            fields.push((key, value));
        }
        Ok(Value::Object(fields))
    }

    fn skip(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.index += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    fn bump(&mut self) {
        self.index += 1;
    }

    fn expect(&mut self, byte: u8) -> Result<(), SnapshotError> {
        if self.peek() != Some(byte) {
            return Err(SnapshotError::Syntax(format!(
                "expected '{}' in machine image",
                byte as char
            )));
        }
        self.bump();
        Ok(())
    }

    fn string(&mut self) -> Result<String, SnapshotError> {
        self.expect(b'"')?;
        let start = self.index;
        while let Some(byte) = self.peek() {
            if byte == b'"' {
                let text = std::str::from_utf8(&self.bytes[start..self.index])
                    .map_err(|_| SnapshotError::Syntax("string is not utf-8".into()))?
                    .to_string();
                self.bump();
                return Ok(text);
            }
            if byte == b'\\' {
                return Err(SnapshotError::Syntax(
                    "escapes are not accepted in this image".into(),
                ));
            }
            self.bump();
        }
        Err(SnapshotError::Syntax("unterminated string".into()))
    }

    fn u64_value(&mut self) -> Result<u64, SnapshotError> {
        let start = self.index;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.bump();
        }
        if start == self.index {
            return Err(SnapshotError::WrongType("number"));
        }
        let text = std::str::from_utf8(&self.bytes[start..self.index]).unwrap();
        text.parse()
            .map_err(|_| SnapshotError::Syntax(format!("number out of range: {text}")))
    }

    fn bool_value(&mut self) -> Result<bool, SnapshotError> {
        if self.bytes[self.index..].starts_with(b"true") {
            self.index += 4;
            Ok(true)
        } else if self.bytes[self.index..].starts_with(b"false") {
            self.index += 5;
            Ok(false)
        } else {
            Err(SnapshotError::WrongType("true or false"))
        }
    }

    fn null_value(&mut self) -> Result<Value, SnapshotError> {
        if self.bytes[self.index..].starts_with(b"null") {
            self.index += 4;
            Ok(Value::Null)
        } else {
            Err(SnapshotError::WrongType("null"))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegChange {
    pub name: &'static str,
    pub before: u64,
    pub after: u64,
}

/// Registers that differ between two images, in architectural order.
pub fn diff_registers(before: &MachineImage, after: &MachineImage) -> Vec<RegChange> {
    let fields = [
        ("a", u64::from(before.a), u64::from(after.a)),
        ("x", u64::from(before.x), u64::from(after.x)),
        ("y", u64::from(before.y), u64::from(after.y)),
        ("sp", u64::from(before.sp), u64::from(after.sp)),
        ("pc", u64::from(before.pc), u64::from(after.pc)),
        ("p", u64::from(before.p), u64::from(after.p)),
        ("cycles", before.cycles, after.cycles),
        ("halted", u64::from(before.halted), u64::from(after.halted)),
    ];
    fields
        .into_iter()
        .filter(|(_, left, right)| left != right)
        .map(|(name, before, after)| RegChange {
            name,
            before,
            after,
        })
        .collect()
}

/// Registers, VIA timer latch bytes, palette bytes, and name-table bytes that differ.
/// Counter bytes are not listed. A palette change is `palette` and a name-table
/// change is `name_table`, not a register.
pub fn diff_image(before: &MachineImage, after: &MachineImage) -> Vec<RegChange> {
    let mut changes = diff_registers(before, after);
    let pairs = [
        (
            "t1_latch_lo",
            before.via.t1_latch as u8,
            after.via.t1_latch as u8,
        ),
        (
            "t1_latch_hi",
            (before.via.t1_latch >> 8) as u8,
            (after.via.t1_latch >> 8) as u8,
        ),
        (
            "t2_latch_lo",
            before.via.t2_latch as u8,
            after.via.t2_latch as u8,
        ),
        (
            "t2_latch_hi",
            (before.via.t2_latch >> 8) as u8,
            (after.via.t2_latch >> 8) as u8,
        ),
    ];
    for (name, left, right) in pairs {
        if left != right {
            changes.push(RegChange {
                name,
                before: u64::from(left),
                after: u64::from(right),
            });
        }
    }
    for (&left, &right) in before.palette.iter().zip(after.palette.iter()) {
        if left != right {
            changes.push(RegChange {
                name: "palette",
                before: u64::from(left),
                after: u64::from(right),
            });
        }
    }
    for (&left, &right) in before.name_table.iter().zip(after.name_table.iter()) {
        if left != right {
            changes.push(RegChange {
                name: "name_table",
                before: u64::from(left),
                after: u64::from(right),
            });
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{IoDevice, MemoryBus};

    #[test]
    fn json_round_trip_restores_registers_and_ram() {
        let mut cpu = Cpu::new(FlatMemory::new());
        cpu.a = 0x42;
        cpu.x = 0x10;
        cpu.y = 0x20;
        cpu.sp = 0xF0;
        cpu.pc = 0x1234;
        cpu.p = Status(0xA5);
        cpu.cycles = 99;
        cpu.set_halted(true);
        cpu.bus.write(0x0200, 0x7E);
        cpu.bus.write(0xFFFF, 0x01);

        let json = MachineImage::capture(&cpu).to_json();
        assert!(json.contains("\"version\":2"));
        let image = MachineImage::from_json(&json).unwrap();
        let mut restored = Cpu::new(FlatMemory::new());
        image.restore(&mut restored).unwrap();

        assert_eq!(restored.a, 0x42);
        assert_eq!(restored.x, 0x10);
        assert_eq!(restored.y, 0x20);
        assert_eq!(restored.sp, 0xF0);
        assert_eq!(restored.pc, 0x1234);
        assert_eq!(restored.p.raw(), 0xA5);
        assert_eq!(restored.cycles, 99);
        assert!(restored.is_halted());
        assert_eq!(restored.bus.data[0x0200], 0x7E);
        assert_eq!(restored.bus.data[0xFFFF], 0x01);
        assert_eq!(restored.bus.data[0x0000], 0);
    }

    #[test]
    fn version_2_round_trips_via_and_acia() {
        let mut cpu = Cpu::new(FlatMemory::new());
        cpu.a = 0x11;
        cpu.bus.write(0x10, 0x22);
        let mut via = Via6522::new();
        via.write(0x03, 0x01);
        via.write(0x01, 0x01);
        via.write(0x0B, 0x40);
        via.write(0x04, 0x05);
        via.write(0x05, 0x01);
        via.write(0x0E, 0xC0);
        let mut acia = Acia6551::new();
        acia.set_loopback(true);
        acia.write(2, 0x0B);
        acia.write(3, 0x1E);
        acia.set_baud_period(4);
        acia.write(0, 0x41);

        let json = MachineImage::capture_with(&cpu, &via, &acia).to_json();
        let image = MachineImage::from_json(&json).unwrap();
        assert_eq!(image.version, 2);
        let mut cpu2 = Cpu::new(FlatMemory::new());
        let mut via2 = Via6522::new();
        let mut acia2 = Acia6551::new();
        image
            .restore_with(&mut cpu2, &mut via2, &mut acia2)
            .unwrap();

        assert_eq!(cpu2.a, 0x11);
        assert_eq!(cpu2.bus.data[0x10], 0x22);
        assert_eq!(via2.ddra(), 0x01);
        assert_eq!(via2.ora_latch(), 0x01);
        assert_eq!(via2.state().acr, 0x40);
        assert_eq!(via2.t1_counter(), 0x0105);
        assert!(via2.t1_running());
        assert_eq!(via2.read(0x0E) & 0x40, 0x40);
        assert_eq!(acia2.peek(2), 0x0B);
        assert_eq!(acia2.peek(3), 0x1E);
        assert_eq!(acia2.baud_period(), 4);
        assert!(acia2.tx_pending());
        assert_eq!(acia2.pop_tx(), Some(0x41));
    }

    #[test]
    fn rejects_a_snapshot_missing_the_version_field() {
        let cpu = Cpu::new(FlatMemory::new());
        let json = MachineImage::capture(&cpu).to_json();
        let bad = json.replacen("\"version\":2,", "", 1);
        let err = MachineImage::from_json(&bad).unwrap_err();
        assert_eq!(err, SnapshotError::MissingField("version".into()));
        let bad_ver = json.replacen("\"version\":2", "\"version\":1", 1);
        assert!(MachineImage::from_json(&bad_ver).is_err());
    }

    #[test]
    fn rejects_a_truncated_ram_image() {
        let cpu = Cpu::new(FlatMemory::new());
        let mut json = MachineImage::capture(&cpu).to_json();
        let bad = json.replace(&"00".repeat(65536), &"00".repeat(3));
        assert_eq!(
            MachineImage::from_json(&bad).unwrap_err(),
            SnapshotError::Length {
                what: "hex",
                expected: 131072,
                got: 6
            }
        );
        let odd = json.replacen(&"00".repeat(65536), &format!("0g{}", "00".repeat(65535)), 1);
        assert_eq!(
            MachineImage::from_json(&odd).unwrap_err(),
            SnapshotError::BadHex(b'g')
        );
        json.push(' ');
        json.push('x');
        assert!(matches!(
            MachineImage::from_json(&json),
            Err(SnapshotError::Syntax(_))
        ));
    }

    #[test]
    fn version_4_round_trips_palette_and_name_table_and_rejects_version_0() {
        let cpu = Cpu::new(FlatMemory::new());
        let mut palette = identity_palette();
        palette[1] = 0x0A;
        let mut names = [0u8; 256];
        names[0] = 3;
        names[255] = 9;
        let image = MachineImage::capture(&cpu)
            .at_scanline(4)
            .at_console(palette, names);
        assert_eq!(image.version, 4);
        let json = image.to_json();
        assert!(json.contains("\"version\":4"), "{json}");
        let back = MachineImage::from_json(&json).unwrap();
        assert_eq!(back.version, 4);
        assert_eq!(back.scanline, 4);
        assert_eq!(back.palette, palette);
        assert_eq!(back.name_table, names);
        let bad = json.replacen("\"version\":4", "\"version\":0", 1);
        let err = MachineImage::from_json(&bad).unwrap_err();
        assert_eq!(err, SnapshotError::UnsupportedVersion(0));
    }
}

/// Why a machine image could not be read or restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    UnsupportedVersion(u64),
    MissingField(String),
    NotObject(&'static str),
    /// A JSON value of another type; the text names what was expected.
    WrongType(&'static str),
    OutOfRange {
        kind: &'static str,
        value: u64,
    },
    Length {
        what: &'static str,
        expected: usize,
        got: usize,
    },
    BadHex(u8),
    /// Malformed JSON or a construct this format does not accept.
    Syntax(String),
    /// The CPU rejected the bus-cycle state.
    CycleState(String),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion(v) => write!(f, "unsupported snapshot version {v}"),
            Self::MissingField(k) => write!(f, "missing {k}"),
            Self::NotObject(what) => write!(f, "{what} must be an object"),
            Self::WrongType(what) => write!(f, "expected {what}"),
            Self::OutOfRange { kind, value } => write!(f, "{kind} out of range: {value}"),
            Self::Length {
                what,
                expected,
                got,
            } => {
                write!(f, "{what} length {got}, expected {expected}")
            }
            Self::BadHex(b) => write!(f, "hex digit {:?} is not hexadecimal", *b as char),
            Self::Syntax(m) | Self::CycleState(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<SnapshotError> for String {
    fn from(err: SnapshotError) -> Self {
        err.to_string()
    }
}

// ---------------------------------------------------------------------------
// Generic compressed container with integrity check.
// ---------------------------------------------------------------------------

/// Container magic for [`pack`].
pub const PACK_MAGIC: &[u8; 4] = b"M65Z";
pub const PACK_VERSION: u8 = 1;
/// Header: magic, version, method, original length (u32 LE), CRC-32 (u32 LE).
pub const PACK_HEADER: usize = 14;
const METHOD_STORED: u8 = 0;
const METHOD_LZ: u8 = 1;

/// CRC-32 (IEEE 802.3, reflected, polynomial `$EDB88320`), as zip and PNG use it.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Byte-oriented LZ77. Tokens: a control byte `c < $80` is followed by `c + 1`
/// literal bytes; `c >= $80` is a match of `(c & $7F) + 3` bytes (3 to 130) copied
/// from a u16 LE distance back (1 to 65535; overlapping copies repeat, so runs work).
pub fn lz_compress(data: &[u8]) -> Vec<u8> {
    const MIN: usize = 3;
    const MAX: usize = 130;
    let mut out = Vec::with_capacity(data.len() / 2 + 16);
    let mut table = vec![usize::MAX; 1 << 14];
    let hash = |i: usize| {
        let v = u32::from(data[i]) | (u32::from(data[i + 1]) << 8) | (u32::from(data[i + 2]) << 16);
        (v.wrapping_mul(2_654_435_761) >> 18) as usize
    };
    let mut literals: Vec<u8> = Vec::new();
    let flush = |lits: &mut Vec<u8>, out: &mut Vec<u8>| {
        for chunk in lits.chunks(128) {
            out.push((chunk.len() - 1) as u8);
            out.extend_from_slice(chunk);
        }
        lits.clear();
    };
    let mut i = 0;
    while i < data.len() {
        let mut best = (0usize, 0usize);
        if i + MIN <= data.len() {
            let h = hash(i);
            let cand = table[h];
            table[h] = i;
            if cand != usize::MAX && i - cand <= 0xFFFF {
                let limit = (data.len() - i).min(MAX);
                let len = (0..limit)
                    .take_while(|&k| data[cand + k] == data[i + k])
                    .count();
                if len >= MIN {
                    best = (len, i - cand);
                }
            }
        }
        if best.0 >= MIN {
            flush(&mut literals, &mut out);
            out.push(0x80 | (best.0 - MIN) as u8);
            out.extend_from_slice(&(best.1 as u16).to_le_bytes());
            for k in i + 1..i + best.0 {
                if k + MIN <= data.len() {
                    table[hash(k)] = k;
                }
            }
            i += best.0;
        } else {
            literals.push(data[i]);
            i += 1;
        }
    }
    flush(&mut literals, &mut out);
    out
}

/// Inverse of [`lz_compress`]. `expected` bounds the output so a hostile stream
/// cannot grow without limit.
pub fn lz_decompress(stream: &[u8], expected: usize) -> Result<Vec<u8>, PackError> {
    let mut out = Vec::with_capacity(expected);
    let mut i = 0;
    while i < stream.len() {
        let c = stream[i];
        i += 1;
        if c < 0x80 {
            let n = usize::from(c) + 1;
            let lits = stream.get(i..i + n).ok_or(PackError::Corrupt)?;
            out.extend_from_slice(lits);
            i += n;
        } else {
            let len = usize::from(c & 0x7F) + 3;
            let d = stream.get(i..i + 2).ok_or(PackError::Corrupt)?;
            let dist = usize::from(u16::from_le_bytes([d[0], d[1]]));
            i += 2;
            if dist == 0 || dist > out.len() {
                return Err(PackError::Corrupt);
            }
            for _ in 0..len {
                out.push(out[out.len() - dist]);
            }
        }
        if out.len() > expected {
            return Err(PackError::Corrupt);
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackError {
    TooShort,
    BadMagic,
    Version(u8),
    Method(u8),
    /// Payload does not decode (bad token, distance, or overrun).
    Corrupt,
    Length {
        expected: u32,
        found: usize,
    },
    Crc {
        expected: u32,
        found: u32,
    },
    /// Input larger than 4 GiB.
    TooLarge,
}

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => f.write_str("contêiner menor que o cabeçalho"),
            Self::BadMagic => f.write_str("assinatura M65Z ausente"),
            Self::Version(v) => write!(f, "versão de contêiner {v} não suportada"),
            Self::Method(m) => write!(f, "método de compressão {m} desconhecido"),
            Self::Corrupt => f.write_str("dados comprimidos corrompidos"),
            Self::Length { expected, found } => {
                write!(f, "tamanho {found}, esperado {expected}")
            }
            Self::Crc { expected, found } => {
                write!(f, "CRC-32 {found:08X}, esperado {expected:08X}")
            }
            Self::TooLarge => f.write_str("entrada acima de 4 GiB"),
        }
    }
}

impl std::error::Error for PackError {}

/// Wrap any bytes (a JSON snapshot, a cycle-state blob, a RAM image) in an `M65Z`
/// container: LZ-compressed when that is smaller, stored otherwise, always with
/// length and CRC-32 of the original so [`unpack`] detects corruption.
pub fn pack(data: &[u8]) -> Result<Vec<u8>, PackError> {
    let len = u32::try_from(data.len()).map_err(|_| PackError::TooLarge)?;
    let lz = lz_compress(data);
    let (method, payload) = if lz.len() < data.len() {
        (METHOD_LZ, lz)
    } else {
        (METHOD_STORED, data.to_vec())
    };
    let mut out = Vec::with_capacity(PACK_HEADER + payload.len());
    out.extend_from_slice(PACK_MAGIC);
    out.push(PACK_VERSION);
    out.push(method);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Open an `M65Z` container and verify length and CRC-32.
pub fn unpack(container: &[u8]) -> Result<Vec<u8>, PackError> {
    if container.len() < PACK_HEADER {
        return Err(PackError::TooShort);
    }
    if &container[..4] != PACK_MAGIC {
        return Err(PackError::BadMagic);
    }
    if container[4] != PACK_VERSION {
        return Err(PackError::Version(container[4]));
    }
    let expected = u32::from_le_bytes(container[6..10].try_into().unwrap_or_default());
    let crc = u32::from_le_bytes(container[10..14].try_into().unwrap_or_default());
    let payload = &container[PACK_HEADER..];
    let data = match container[5] {
        METHOD_STORED => payload.to_vec(),
        METHOD_LZ => lz_decompress(payload, expected as usize)?,
        m => return Err(PackError::Method(m)),
    };
    if data.len() != expected as usize {
        return Err(PackError::Length {
            expected,
            found: data.len(),
        });
    }
    let found = crc32(&data);
    if found != crc {
        return Err(PackError::Crc {
            expected: crc,
            found,
        });
    }
    Ok(data)
}

/// True when `bytes` starts with the `M65Z` magic.
pub fn is_packed(bytes: &[u8]) -> bool {
    bytes.starts_with(PACK_MAGIC)
}
