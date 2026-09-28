//! Virtual Retro Fantasy Console & Graphical Display.
//!
//! Emulates a 32x32 pixel bitmap display mapped directly into memory at $0200..=$05FF
//! with a 16-color retro palette (compatible with standard 6502 graphical specifications).
//! Also provides hardware random number generator at $00FE and keyboard input at $00FF.

use crate::bus::{FlatMemory, IoDevice, MemoryBus};

/// Standard 16-color retro palette RGB values.
pub const RETRO_PALETTE: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00), // 0: Black
    (0xFF, 0xFF, 0xFF), // 1: White
    (0x88, 0x00, 0x00), // 2: Red
    (0xAA, 0xFF, 0xEE), // 3: Cyan
    (0xCC, 0x44, 0xCC), // 4: Purple
    (0x00, 0xCC, 0x55), // 5: Green
    (0x00, 0x00, 0xAA), // 6: Blue
    (0xEE, 0xEE, 0x77), // 7: Yellow
    (0xDD, 0x88, 0x55), // 8: Orange
    (0x66, 0x44, 0x00), // 9: Brown
    (0xFF, 0x77, 0x77), // 10: Light Red
    (0x33, 0x33, 0x33), // 11: Dark Grey
    (0x77, 0x77, 0x77), // 12: Grey
    (0xAA, 0xFF, 0x66), // 13: Light Green
    (0x00, 0x88, 0xFF), // 14: Light Blue
    (0xBB, 0xBB, 0xBB), // 15: Light Grey
];

/// Palette index closest to an RGB color (squared distance; lowest index on ties).
pub fn nearest_color(r: u8, g: u8, b: u8) -> u8 {
    let d = |(pr, pg, pb): (u8, u8, u8)| {
        let sq = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
        sq(pr, r) + sq(pg, g) + sq(pb, b)
    };
    (0..16u8)
        .min_by_key(|&i| (d(RETRO_PALETTE[usize::from(i)]), i))
        .unwrap()
}

fn identity_palette() -> [u8; 16] {
    let mut palette = [0u8; 16];
    for (index, slot) in palette.iter_mut().enumerate() {
        *slot = index as u8;
    }
    palette
}

/// Virtual Graphics Screen Buffer (32x32 pixels = 1024 bytes).
pub struct Screen {
    pub pixels: [u8; 1024],
}

impl Screen {
    pub fn new() -> Self {
        Self { pixels: [0; 1024] }
    }

    /// Clears the screen with a specific palette color index (0..15).
    pub fn clear(&mut self, color: u8) {
        self.pixels.fill(color & 0x0F);
    }

    /// Sets pixel at (x, y) with color index.
    pub fn set_pixel(&mut self, x: usize, y: usize, color: u8) {
        if x < 32 && y < 32 {
            self.pixels[y * 32 + x] = color & 0x0F;
        }
    }

    /// Palette index at (x, y), `None` off screen.
    pub fn pixel(&self, x: usize, y: usize) -> Option<u8> {
        (x < 32 && y < 32).then(|| self.pixels[y * 32 + x] & 0x0F)
    }

    /// Binary PPM (P6), each pixel a `scale`×`scale` square. `scale` must be 1..=64.
    pub fn to_ppm(&self, scale: usize) -> Result<Vec<u8>, String> {
        if !(1..=64).contains(&scale) {
            return Err(format!("escala {scale} fora de 1..=64"));
        }
        let side = 32 * scale;
        let mut out = format!("P6\n{side} {side}\n255\n").into_bytes();
        out.reserve(side * side * 3);
        for y in 0..side {
            for x in 0..side {
                let (r, g, b) =
                    RETRO_PALETTE[usize::from(self.pixels[(y / scale) * 32 + x / scale] & 0x0F)];
                out.extend([r, g, b]);
            }
        }
        Ok(out)
    }

    /// Renders the 32x32 graphical screen to the terminal using ANSI 24-bit half-blocks.
    /// Each terminal character cell renders two vertical pixels using the upper-half block ('▀').
    pub fn render_to_terminal(&self) -> String {
        let mut out = String::with_capacity(32 * 32 * 20);
        out.push_str("\x1b[H"); // Cursor para o topo sem piscar

        out.push_str("\x1b[1;36m+--------------------------------+\x1b[0m\n");
        for y in (0..32).step_by(2) {
            out.push_str("\x1b[1;36m|\x1b[0m");
            for x in 0..32 {
                let top_color_idx = (self.pixels[y * 32 + x] & 0x0F) as usize;
                let btm_color_idx = (self.pixels[(y + 1) * 32 + x] & 0x0F) as usize;

                let (tr, tg, tb) = RETRO_PALETTE[top_color_idx];
                let (br, bg, bb) = RETRO_PALETTE[btm_color_idx];

                // ANSI 24-bit Truecolor: foreground (tr, tg, tb) e background (br, bg, bb)
                out.push_str(&format!(
                    "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m▀",
                    tr, tg, tb, br, bg, bb
                ));
            }
            out.push_str("\x1b[0m\x1b[1;36m|\x1b[0m\n");
        }
        out.push_str("\x1b[1;36m+--------------------------------+\x1b[0m\n");
        out
    }
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

/// Dispositivo de I/O de tela e periféricos de jogos.
/// Mapeado em memória:
/// - $00FE: Gerador de Números Pseudo-Aleatórios (RNG)
/// - $00FF: Registro de Teclado (última tecla ASCII pressionada)
/// - $0200..=$05FF: Framebuffer de Vídeo 32x32 (1024 bytes)
pub struct FantasyConsoleDevice {
    pub screen: Screen,
    pub last_key: u8,
    /// Scanline currently being drawn, `0..32`. The CPU reads it at `$00FD`.
    pub scanline: u8,
    /// Pattern table. Each sprite is 64 bytes, 8×8, row-major. Zero is transparent.
    pub patterns: Vec<u8>,
    /// Sixteen remap registers. A non-zero pattern nibble `i` is written as `palette[i]`.
    /// The default is the identity (`1` stays `1`) so older blit tests keep their colors.
    /// Index 0 is never consulted: a zero source pixel stays transparent.
    pub palette: [u8; 16],
    /// 256-byte name table, 16 columns by 16 rows of tile indices. The 32×32 screen
    /// shows the top-left 4×4 tiles (8×8 each). Not a PPU.
    pub name_table: [u8; 256],
    sprite_index: [u8; 2],
    sprite_x: [u8; 2],
    sprite_y: [u8; 2],
    opaque: [[u32; 32]; 2],
    /// Set only when sprite 1's opaque pixels overlap sprite 0's. The CPU reads it at `$00F4`.
    pub collision: bool,
    rng_state: u32,
}

impl FantasyConsoleDevice {
    pub fn new() -> Self {
        Self {
            screen: Screen::new(),
            last_key: 0,
            scanline: 0,
            patterns: Vec::new(),
            palette: identity_palette(),
            name_table: [0; 256],
            sprite_index: [0; 2],
            sprite_x: [0; 2],
            sprite_y: [0; 2],
            opaque: [[0; 32]; 2],
            collision: false,
            rng_state: 0x12345678,
        }
    }

    /// Alimenta o buffer com uma tecla do usuário.
    pub fn press_key(&mut self, key: u8) {
        self.last_key = key;
    }

    /// Square-wave PCM for a fantasy-console tone. No sound device is opened.
    pub fn tone(
        &self,
        frequency_hz: f32,
        sample_rate: u32,
        duration_secs: f32,
    ) -> Result<Vec<i16>, String> {
        crate::audio::square_wave(frequency_hz, sample_rate, duration_secs)
    }

    /// Gera um byte pseudo-aleatório de 8 bits (Xorshift32).
    pub fn next_random_byte(&mut self) -> u8 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        (x & 0xFF) as u8
    }

    /// Replace the pattern table. Sprite `n` is bytes `n*64 .. n*64+64`.
    pub fn set_patterns(&mut self, bytes: &[u8]) {
        self.patterns = bytes.to_vec();
    }

    /// Palette register. `index` and `color` are masked to a nibble.
    pub fn set_palette(&mut self, index: u8, color: u8) {
        self.palette[(index & 0x0F) as usize] = color & 0x0F;
    }

    fn mapped_color(&self, index: u8) -> u8 {
        self.palette[(index & 0x0F) as usize] & 0x0F
    }

    /// Paint one visible name-table cell. `col` and `row` are tile coordinates.
    /// Only the top-left 4×4 of the 16×16 name table lands on the 32×32 screen.
    /// The name byte selects an 8×8 pattern. Source color 0 is skipped; any other
    /// index is written through the palette register.
    pub fn render_name_tile(&mut self, col: u8, row: u8) {
        if col >= 4 || row >= 4 {
            return;
        }
        let name = self.name_table[(row as usize) * 16 + col as usize];
        let tile = self.pattern(name);
        let origin_x = col as usize * 8;
        let origin_y = row as usize * 8;
        for y in 0..8 {
            for x in 0..8 {
                let index = tile[y * 8 + x] & 0x0F;
                if index == 0 {
                    continue;
                }
                self.screen.pixels[(origin_y + y) * 32 + origin_x + x] = self.mapped_color(index);
            }
        }
    }

    fn pattern(&self, index: u8) -> [u8; 64] {
        let start = index as usize * 64;
        let mut tile = [0u8; 64];
        if start + 64 <= self.patterns.len() {
            tile.copy_from_slice(&self.patterns[start..start + 64]);
        }
        tile
    }

    /// Blit one 8×8 sprite. `slot` 0 is the first sprite. `slot` 1 recomputes collision
    /// against sprite 0: the flag is set only where both tiles have a non-zero pixel
    /// on the same screen coordinate. Pixels past the 32×32 edge are clipped.
    pub fn blit_sprite(&mut self, slot: usize) {
        let slot = slot.min(1);
        let tile = self.pattern(self.sprite_index[slot]);
        let origin_x = self.sprite_x[slot] as usize;
        let origin_y = self.sprite_y[slot] as usize;
        self.opaque[slot] = [0; 32];
        for row in 0..8 {
            for col in 0..8 {
                let index = tile[row * 8 + col] & 0x0F;
                let x = origin_x + col;
                let y = origin_y + row;
                if x >= 32 || y >= 32 || index == 0 {
                    continue;
                }
                let color = self.mapped_color(index);
                self.opaque[slot][y] |= 1u32 << x;
                self.screen.pixels[y * 32 + x] = color;
            }
        }
        if slot == 1 {
            self.collision = self.opaque[0]
                .iter()
                .zip(self.opaque[1].iter())
                .any(|(left, right)| left & right != 0);
        }
    }

    /// Sincroniza o framebuffer a partir da memória do barramento ($0200..=$05FF).
    pub fn sync_from_bus<B: MemoryBus>(&mut self, bus: &B) {
        for i in 0..1024 {
            self.screen.pixels[i] = bus.peek(0x0200 + i as u16);
        }
    }
}

impl Default for FantasyConsoleDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl IoDevice for FantasyConsoleDevice {
    fn read(&mut self, offset: u16) -> u8 {
        match offset {
            0 => self.next_random_byte(),
            1 => {
                let k = self.last_key;
                self.last_key = 0; // Leitura consome a tecla
                k
            }
            0x0102..=0x0501 => {
                let idx = (offset - 0x0102) as usize;
                self.screen.pixels[idx] & 0x0F
            }
            _ => 0,
        }
    }

    fn write(&mut self, offset: u16, value: u8) {
        match offset {
            0 => {
                self.rng_state = if value == 0 {
                    0x6D2B_79F5
                } else {
                    value as u32
                };
            }
            1 => {
                self.last_key = value;
            }
            0x0102..=0x0501 => {
                let idx = (offset - 0x0102) as usize;
                self.screen.pixels[idx] = value & 0x0F;
            }
            _ => {}
        }
    }

    fn peek(&self, offset: u16) -> u8 {
        match offset {
            0 => (self.rng_state & 0xFF) as u8,
            1 => self.last_key,
            0x0102..=0x0501 => self.screen.pixels[(offset - 0x0102) as usize] & 0x0F,
            _ => 0,
        }
    }
}

/// Memory bus for running programs on the fantasy console without shadowing stack RAM.
pub struct FantasyConsoleBus {
    pub memory: FlatMemory,
    pub device: FantasyConsoleDevice,
}

impl FantasyConsoleBus {
    pub fn new() -> Self {
        Self {
            memory: FlatMemory::new(),
            device: FantasyConsoleDevice::new(),
        }
    }

    pub fn load_at(&mut self, base: u16, bytes: &[u8]) {
        self.memory.load_at(base, bytes);
    }

    pub fn set_reset_vector(&mut self, entry: u16) {
        self.memory.set_reset_vector(entry);
    }

    pub fn set_patterns(&mut self, bytes: &[u8]) {
        self.device.set_patterns(bytes);
    }

    pub fn scanline(&self) -> u8 {
        self.device.scanline
    }

    /// `line` is masked to the 32-line display (`0..32`).
    pub fn set_scanline(&mut self, line: u8) {
        self.device.scanline = line & 31;
    }

    /// Advance one scanline, wrapping from 31 back to 0.
    pub fn tick_scanline(&mut self) {
        self.device.scanline = (self.device.scanline + 1) & 31;
    }
}

impl Default for FantasyConsoleBus {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryBus for FantasyConsoleBus {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x00F4 => u8::from(self.device.collision),
            0x00F5 => self.device.sprite_index[1],
            0x00F6 => self.device.sprite_x[1],
            0x00F7 => self.device.sprite_y[1],
            0x00F8 => self.device.sprite_index[0],
            0x00F9 => self.device.sprite_x[0],
            0x00FA => self.device.sprite_y[0],
            0x00FD => self.device.scanline,
            0x00FE => self.device.read(0),
            0x00FF => self.device.read(1),
            0x0200..=0x05FF => self.device.read(addr - 0x00FE),
            _ => self.memory.read(addr),
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x00F4 => self.device.collision = value != 0,
            0x00F5 => self.device.sprite_index[1] = value,
            0x00F6 => self.device.sprite_x[1] = value,
            0x00F7 => {
                self.device.sprite_y[1] = value;
                self.device.blit_sprite(1);
            }
            0x00F8 => self.device.sprite_index[0] = value,
            0x00F9 => self.device.sprite_x[0] = value,
            0x00FA => {
                self.device.sprite_y[0] = value;
                self.device.blit_sprite(0);
            }
            0x00FD => self.device.scanline = value & 31,
            0x00FE => self.device.write(0, value),
            0x00FF => self.device.write(1, value),
            0x0200..=0x05FF => self.device.write(addr - 0x00FE, value),
            _ => self.memory.write(addr, value),
        }
    }

    fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x00F4 => u8::from(self.device.collision),
            0x00F5 => self.device.sprite_index[1],
            0x00F6 => self.device.sprite_x[1],
            0x00F7 => self.device.sprite_y[1],
            0x00F8 => self.device.sprite_index[0],
            0x00F9 => self.device.sprite_x[0],
            0x00FA => self.device.sprite_y[0],
            0x00FD => self.device.scanline,
            0x00FE => self.device.peek(0),
            0x00FF => self.device.peek(1),
            0x0200..=0x05FF => self.device.peek(addr - 0x00FE),
            _ => self.memory.peek(addr),
        }
    }
}

// ---------------------------------------------------------------------------
// zlib / DEFLATE (RFC 1950 / 1951) and PNG (ISO/IEC 15948), implemented here so
// frames export without dependencies.
// ---------------------------------------------------------------------------

/// Adler-32 checksum of a zlib stream (RFC 1950).
pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            acc: 0,
            n: 0,
        }
    }
    /// `count` low bits of `value`, least significant first.
    fn bits(&mut self, value: u32, count: u32) {
        self.acc |= value << self.n;
        self.n += count;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }
    /// A Huffman code, most significant bit first.
    fn code(&mut self, code: u32, len: u32) {
        let rev = code.reverse_bits() >> (32 - len);
        self.bits(rev, len);
    }
    fn align(&mut self) {
        if self.n > 0 {
            self.out.push(self.acc as u8);
            self.acc = 0;
            self.n = 0;
        }
    }
}

fn fixed_literal(w: &mut BitWriter, sym: u16) {
    match sym {
        0..=143 => w.code(0x30 + u32::from(sym), 8),
        144..=255 => w.code(0x190 + u32::from(sym - 144), 9),
        256..=279 => w.code(u32::from(sym - 256), 7),
        _ => w.code(0xC0 + u32::from(sym - 280), 8),
    }
}

fn fixed_match(w: &mut BitWriter, len: usize, dist: usize) {
    let li = LEN_BASE
        .iter()
        .rposition(|&b| usize::from(b) <= len)
        .unwrap_or(0);
    fixed_literal(w, 257 + li as u16);
    w.bits(
        (len - usize::from(LEN_BASE[li])) as u32,
        u32::from(LEN_EXTRA[li]),
    );
    let di = DIST_BASE
        .iter()
        .rposition(|&b| usize::from(b) <= dist)
        .unwrap_or(0);
    w.code(di as u32, 5);
    w.bits(
        (dist - usize::from(DIST_BASE[di])) as u32,
        u32::from(DIST_EXTRA[di]),
    );
}

/// Raw DEFLATE: stored blocks (`level` 0) or one fixed-Huffman block with LZ77
/// matching over a 32 KiB window (`level` 1..=9 sets the hash-chain depth).
pub fn deflate(data: &[u8], level: u8) -> Vec<u8> {
    let mut w = BitWriter::new();
    if level == 0 {
        let mut chunks = data.chunks(65535).peekable();
        if chunks.peek().is_none() {
            w.bits(1, 1);
            w.bits(0, 2);
            w.align();
            w.out.extend_from_slice(&[0, 0, 0xFF, 0xFF]);
            return w.out;
        }
        while let Some(chunk) = chunks.next() {
            w.bits(u32::from(chunks.peek().is_none()), 1);
            w.bits(0, 2);
            w.align();
            let len = chunk.len() as u16;
            w.out.extend_from_slice(&len.to_le_bytes());
            w.out.extend_from_slice(&(!len).to_le_bytes());
            w.out.extend_from_slice(chunk);
        }
        return w.out;
    }
    const WINDOW: usize = 32768;
    let depth = usize::from(level.min(9)) * 8;
    let mut head = vec![usize::MAX; 1 << 15];
    let mut prev = vec![usize::MAX; data.len()];
    let hash = |i: usize| {
        ((usize::from(data[i]) << 10) ^ (usize::from(data[i + 1]) << 5) ^ usize::from(data[i + 2]))
            & 0x7FFF
    };
    w.bits(1, 1);
    w.bits(1, 2);
    let mut i = 0;
    let insert = |k: usize, head: &mut Vec<usize>, prev: &mut Vec<usize>| {
        if k + 3 <= data.len() {
            let h = hash(k);
            prev[k] = head[h];
            head[h] = k;
        }
    };
    while i < data.len() {
        let mut best = (0usize, 0usize);
        if i + 3 <= data.len() {
            let mut cand = head[hash(i)];
            let limit = (data.len() - i).min(258);
            let mut tries = 0;
            while cand != usize::MAX && i - cand <= WINDOW && tries < depth {
                let len = (0..limit)
                    .take_while(|&k| data[cand + k] == data[i + k])
                    .count();
                if len > best.0 {
                    best = (len, i - cand);
                    if len == limit {
                        break;
                    }
                }
                cand = prev[cand];
                tries += 1;
            }
        }
        if best.0 >= 3 {
            fixed_match(&mut w, best.0, best.1);
            for k in i..i + best.0 {
                insert(k, &mut head, &mut prev);
            }
            i += best.0;
        } else {
            fixed_literal(&mut w, u16::from(data[i]));
            insert(i, &mut head, &mut prev);
            i += 1;
        }
    }
    fixed_literal(&mut w, 256);
    w.align();
    w.out
}

/// zlib stream: header `78 01` (stored) or `78 9C`, DEFLATE data, Adler-32.
pub fn zlib_compress(data: &[u8], level: u8) -> Vec<u8> {
    let mut out = if level == 0 {
        vec![0x78, 0x01]
    } else {
        vec![0x78, 0x9C]
    };
    out.extend(deflate(data, level));
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u32,
}

impl BitReader<'_> {
    fn bit(&mut self) -> Result<u32, String> {
        let byte = *self.data.get(self.pos).ok_or("fluxo DEFLATE truncado")?;
        let b = u32::from(byte >> self.bit) & 1;
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.pos += 1;
        }
        Ok(b)
    }
    fn bits(&mut self, n: u32) -> Result<u32, String> {
        let mut v = 0;
        for i in 0..n {
            v |= self.bit()? << i;
        }
        Ok(v)
    }
    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.pos += 1;
        }
    }
}

/// Canonical Huffman table: code counts per length and symbols in code order.
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Self, String> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[usize::from(l)] += 1;
        }
        counts[0] = 0;
        let mut left: i32 = 1;
        for &c in &counts[1..] {
            left = (left << 1) - i32::from(c);
            if left < 0 {
                return Err("tabela de Huffman superdeterminada".into());
            }
        }
        let mut offs = [0u16; 16];
        for l in 1..15 {
            offs[l + 1] = offs[l] + counts[l];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[usize::from(offs[usize::from(l)])] = s as u16;
                offs[usize::from(l)] += 1;
            }
        }
        Ok(Self { counts, symbols })
    }

    fn decode(&self, r: &mut BitReader) -> Result<u16, String> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= r.bit()? as i32;
            let count = i32::from(self.counts[len]);
            if code - count < first {
                return Ok(self.symbols[(index + code - first) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err("código de Huffman inválido".into())
    }
}

/// Inflate raw DEFLATE (stored, fixed and dynamic blocks). `limit` caps the output.
pub fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let mut r = BitReader {
        data,
        pos: 0,
        bit: 0,
    };
    let mut out = Vec::new();
    loop {
        let last = r.bit()?;
        match r.bits(2)? {
            0 => {
                r.align();
                let h = data
                    .get(r.pos..r.pos + 4)
                    .ok_or("bloco armazenado truncado")?;
                let len = u16::from_le_bytes([h[0], h[1]]);
                if !len != u16::from_le_bytes([h[2], h[3]]) {
                    return Err("LEN/NLEN não conferem".into());
                }
                r.pos += 4;
                let body = data
                    .get(r.pos..r.pos + usize::from(len))
                    .ok_or("bloco armazenado truncado")?;
                out.extend_from_slice(body);
                r.pos += usize::from(len);
            }
            t @ (1 | 2) => {
                let (lit, dist) = if t == 1 {
                    let mut l = [8u8; 288];
                    l[144..256].fill(9);
                    l[256..280].fill(7);
                    (Huffman::new(&l)?, Huffman::new(&[5u8; 30])?)
                } else {
                    dynamic_tables(&mut r)?
                };
                loop {
                    let sym = lit.decode(&mut r)?;
                    match sym {
                        0..=255 => out.push(sym as u8),
                        256 => break,
                        257..=285 => {
                            let i = usize::from(sym - 257);
                            let len = usize::from(LEN_BASE[i])
                                + r.bits(u32::from(LEN_EXTRA[i]))? as usize;
                            let d = usize::from(dist.decode(&mut r)?);
                            if d >= 30 {
                                return Err("código de distância inválido".into());
                            }
                            let dist = usize::from(DIST_BASE[d])
                                + r.bits(u32::from(DIST_EXTRA[d]))? as usize;
                            if dist > out.len() {
                                return Err("distância antes do início".into());
                            }
                            for _ in 0..len {
                                out.push(out[out.len() - dist]);
                            }
                        }
                        _ => return Err("símbolo de comprimento inválido".into()),
                    }
                    if out.len() > limit {
                        return Err("saída acima do limite".into());
                    }
                }
            }
            _ => return Err("tipo de bloco 3 reservado".into()),
        }
        if out.len() > limit {
            return Err("saída acima do limite".into());
        }
        if last == 1 {
            return Ok(out);
        }
    }
}

fn dynamic_tables(r: &mut BitReader) -> Result<(Huffman, Huffman), String> {
    const ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let hlit = r.bits(5)? as usize + 257;
    let hdist = r.bits(5)? as usize + 1;
    let hclen = r.bits(4)? as usize + 4;
    let mut cl = [0u8; 19];
    for &o in &ORDER[..hclen] {
        cl[o] = r.bits(3)? as u8;
    }
    let clh = Huffman::new(&cl)?;
    let mut lengths = Vec::with_capacity(hlit + hdist);
    while lengths.len() < hlit + hdist {
        let sym = clh.decode(r)?;
        let (value, repeat) = match sym {
            0..=15 => (sym as u8, 1),
            16 => (
                *lengths.last().ok_or("repetição sem comprimento anterior")?,
                3 + r.bits(2)? as usize,
            ),
            17 => (0, 3 + r.bits(3)? as usize),
            _ => (0, 11 + r.bits(7)? as usize),
        };
        lengths.extend(std::iter::repeat_n(value, repeat));
    }
    if lengths.len() != hlit + hdist {
        return Err("comprimentos de código excedem HLIT+HDIST".into());
    }
    Ok((
        Huffman::new(&lengths[..hlit])?,
        Huffman::new(&lengths[hlit..])?,
    ))
}

/// Inflate a zlib stream, checking header (CM 8, FCHECK, no dictionary) and Adler-32.
pub fn zlib_decompress(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    if data.len() < 6 {
        return Err("fluxo zlib curto demais".into());
    }
    if data[0] & 0x0F != 8 || ((u16::from(data[0]) << 8) | u16::from(data[1])) % 31 != 0 {
        return Err("cabeçalho zlib inválido".into());
    }
    if data[1] & 0x20 != 0 {
        return Err("dicionário predefinido não suportado".into());
    }
    let out = inflate(&data[2..data.len() - 4], limit)?;
    let tail = &data[data.len() - 4..];
    let want = u32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]);
    if adler32(&out) != want {
        return Err("Adler-32 não confere".into());
    }
    Ok(out)
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crate::snapshot::crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = (
        (p - i16::from(a)).abs(),
        (p - i16::from(b)).abs(),
        (p - i16::from(c)).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Truecolor 8-bit PNG of `rgb` (`width * height * 3` bytes). Each row picks the
/// filter (None, Sub, Up, Average, Paeth) with the smallest sum of absolute values,
/// the usual heuristic; `level` 0 stores, otherwise DEFLATE as in [`deflate`].
pub fn png_rgb(width: u32, height: u32, rgb: &[u8], level: u8) -> Result<Vec<u8>, String> {
    let stride = width as usize * 3;
    if width == 0 || height == 0 || rgb.len() != stride * height as usize {
        return Err(format!(
            "imagem {width}x{height} exige {} bytes RGB, recebeu {}",
            stride * height as usize,
            rgb.len()
        ));
    }
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    let zero = vec![0u8; stride];
    for y in 0..height as usize {
        let row = &rgb[y * stride..(y + 1) * stride];
        let up = if y == 0 {
            &zero[..]
        } else {
            &rgb[(y - 1) * stride..y * stride]
        };
        let mut best: Option<(u64, u8, Vec<u8>)> = None;
        for f in 0..5u8 {
            let filtered: Vec<u8> = (0..stride)
                .map(|i| {
                    let a = if i >= 3 { row[i - 3] } else { 0 };
                    let c = if i >= 3 { up[i - 3] } else { 0 };
                    let pred = match f {
                        0 => 0,
                        1 => a,
                        2 => up[i],
                        3 => ((u16::from(a) + u16::from(up[i])) / 2) as u8,
                        _ => paeth(a, up[i], c),
                    };
                    row[i].wrapping_sub(pred)
                })
                .collect();
            let cost = filtered
                .iter()
                .map(|&b| u64::from((b as i8).unsigned_abs()))
                .sum();
            if best.as_ref().is_none_or(|(c, _, _)| cost < *c) {
                best = Some((cost, f, filtered));
            }
        }
        let (_, f, filtered) = best.unwrap_or_default();
        raw.push(f);
        raw.extend(filtered);
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    png_chunk(&mut out, b"IHDR", &ihdr);
    png_chunk(&mut out, b"IDAT", &zlib_compress(&raw, level));
    png_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

/// Decode an 8-bit truecolor non-interlaced PNG (as written by [`png_rgb`] or
/// any encoder using that format): every chunk CRC is checked, IDAT chunks are
/// joined, all five filters are undone. Returns width, height and RGB bytes.
pub fn decode_png_rgb(png: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("assinatura PNG ausente".into());
    }
    let mut pos = 8;
    let mut header = None;
    let mut idat = Vec::new();
    loop {
        let len_bytes = png.get(pos..pos + 4).ok_or("PNG truncado")?;
        let len = u32::from_be_bytes(len_bytes.try_into().unwrap_or_default()) as usize;
        let body = png.get(pos + 4..pos + 8 + len).ok_or("PNG truncado")?;
        let crc = png
            .get(pos + 8 + len..pos + 12 + len)
            .ok_or("PNG truncado")?;
        if crate::snapshot::crc32(body).to_be_bytes() != crc {
            return Err(format!(
                "CRC do bloco {} inválido",
                String::from_utf8_lossy(&body[..4])
            ));
        }
        let (kind, data) = body.split_at(4);
        match kind {
            b"IHDR" => {
                if data.len() != 13 || data[8..13] != [8, 2, 0, 0, 0] {
                    return Err("só RGB 8 bits sem entrelaçamento".into());
                }
                let w = u32::from_be_bytes(data[0..4].try_into().unwrap_or_default());
                let h = u32::from_be_bytes(data[4..8].try_into().unwrap_or_default());
                header = Some((w, h));
            }
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    let (w, h) = header.ok_or("IHDR ausente")?;
    let stride = w as usize * 3;
    let raw = zlib_decompress(&idat, (stride + 1) * h as usize)?;
    if raw.len() != (stride + 1) * h as usize {
        return Err("tamanho dos dados da imagem incorreto".into());
    }
    let mut rgb = vec![0u8; stride * h as usize];
    for y in 0..h as usize {
        let f = raw[y * (stride + 1)];
        let line = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        for i in 0..stride {
            let a = if i >= 3 { rgb[y * stride + i - 3] } else { 0 };
            let b = if y > 0 { rgb[(y - 1) * stride + i] } else { 0 };
            let c = if i >= 3 && y > 0 {
                rgb[(y - 1) * stride + i - 3]
            } else {
                0
            };
            let pred = match f {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                4 => paeth(a, b, c),
                _ => return Err(format!("filtro PNG {f} inválido")),
            };
            rgb[y * stride + i] = line[i].wrapping_add(pred);
        }
    }
    Ok((w, h, rgb))
}

/// Binary PPM (P6) of an RGB buffer, to compare with [`png_rgb`] output.
pub fn ppm_rgb(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let mut out = format!("P6\n{width} {height}\n255\n").into_bytes();
    out.extend_from_slice(rgb);
    out
}

#[cfg(test)]
mod tests {
    use super::FantasyConsoleBus;
    use crate::bus::MemoryBus;

    #[test]
    fn address_map_keeps_ram_and_masks_framebuffer_colors() {
        let mut bus = FantasyConsoleBus::new();
        bus.write(0x0100, 0xA5);
        bus.write(0x0200, 0xFF);

        assert_eq!(bus.read(0x0100), 0xA5);
        assert_eq!(bus.read(0x0200), 0x0F);
        assert_eq!(bus.peek(0x0200), 0x0F);
        assert_eq!(bus.device.screen.pixels[0], 0x0F);
    }

    #[test]
    fn keyboard_register_returns_and_consumes_the_last_key() {
        let mut bus = FantasyConsoleBus::new();
        bus.write(0x00FF, b'K');

        assert_eq!(bus.read(0x00FF), b'K');
        assert_eq!(bus.read(0x00FF), 0);
    }

    #[test]
    fn tone_delegates_to_the_pcm_square_wave() {
        let device = super::FantasyConsoleDevice::new();
        let samples = device.tone(1000.0, 8000, 0.001).unwrap();
        assert_eq!(samples.len(), 8);
        assert_eq!(samples[0], 12_000);
        assert_eq!(samples[4], -12_000);
    }

    #[test]
    fn zero_rng_seed_does_not_lock_the_generator() {
        let mut bus = FantasyConsoleBus::new();
        bus.write(0x00FE, 0);

        assert_ne!(bus.device.rng_state, 0);
        let _ = bus.read(0x00FE);
        assert_ne!(bus.device.rng_state, 0);
    }

    #[test]
    fn bundled_palette_program_draws_all_framebuffer_pages() {
        let program =
            crate::asm::assemble(include_str!("../examples/fantasy_palette.asm"), 0x8000).unwrap();
        let mut bus = FantasyConsoleBus::new();
        bus.load_at(0x8000, &program);
        bus.set_reset_vector(0x8000);
        let mut cpu = crate::cpu::Cpu::new(bus);
        cpu.halt_on_brk = true;
        cpu.reset();

        cpu.run_cycles(50_000).unwrap();

        assert!(cpu.is_halted());
        for (index, expected) in [(0, 0), (1, 1), (15, 15), (255, 15), (256, 0), (1023, 15)] {
            assert_eq!(
                cpu.bus.device.screen.pixels[index], expected,
                "pixel {index}"
            );
        }
    }
}
