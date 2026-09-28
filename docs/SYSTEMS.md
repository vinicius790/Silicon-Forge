# Emulated Systems

The suite contains hardware models and educational environments at different levels of completeness. A module or command should not be confused with an authentic firmware image: only the Apple-1 currently includes its monitor ROM.

## 1. 🍏 Apple-1 (1976)

The legendary computer created by Steve Wozniak and Steve Jobs.

*   **Processor**: 6502 @ 1.023 MHz
*   **Memory**: 8KB RAM by default ($0000 - $1FFF)
*   **ROM**: WozMon ($FF00 - $FFFF)
*   **I/O**: PIA 6820 handling keyboard input and display output.

### Usage
Run the Apple-1 emulation:
```bash
cargo run -- apple1
```
The WozMon ROM is mapped at `$FF00`. The terminal currently provides a host-side command interface; it does not emulate keyboard polling cycle by cycle.

## 2. 🍞 Ben Eater's Breadboard Computer

A modern 6502 system popularized by Ben Eater's YouTube series.

*   **Memory**: 64 KiB backing memory; the example model maps a ROM image at `$8000` when supplied.
*   **I/O**: 6522 VIA mapped at $6000, connected to an HD44780 LCD controller.
*   **Status**: The system type is an embeddable component; there is no dedicated CLI launcher yet.

## 3. 🎹 KIM-1 (1976)

The classic MOS Technology evaluation board.

*   **Memory**: 1KB RAM ($0000 - $03FF)
*   **Monitor ROM**: Not bundled; the reset vector currently points to `$1C00` without monitor bytes mapped there.
*   **I/O**: Partial 6530 RRIOT register and display model; keypad and timer behavior are not fully implemented.
*   **Status**: Embeddable hardware skeleton; no dedicated CLI launcher yet.

## 4. 💾 Microsoft 6502 BASIC (1977)

A host-side educational interpreter for a small numeric BASIC subset. It is not the Microsoft/OSI 6502 binary and does not execute BASIC statements on the emulated CPU.

*   **Language**: Numbered lines, `LET`/implicit assignment, numeric variables, `PRINT`/`?`, `LIST`, `NEW`, `RUN`, `REM`, and `END`.
*   **Expressions**: Parentheses, unary signs, and `+`, `-`, `*`, `/` precedence.
*   **Status**: Host-side interpreter, not a 6502 system or authentic BASIC ROM.

### Usage
```bash
cargo run -- basic
```
Example session: `10 LET A=2+3*4`, `20 PRINT A`, then `RUN` prints `14`. Type `HELP` for a short command list.

## 5. 🕹️ Fantasy Console

A runnable 6502 fantasy console with a 32x32 indexed-color framebuffer, pseudo-random register, and single-byte keyboard register.

*   **Video**: 32x32 pixel screen, 16 colors.
*   **VRAM Map**: $0200 - $05FF maps directly to the 1024 pixels on screen.
*   **I/O**: $00FE (RNG seed/data), $00FF (single-byte keyboard), $0200-$05FF (framebuffer).
*   **Execution**: Loads a flat binary at $8000 and runs it until BRK or a cycle limit.

Build and run the included framebuffer demo:
```bash
cargo run --release -- asm examples/fantasy_palette.asm -o target/fantasy_palette.bin
cargo run --release -- fantasy target/fantasy_palette.bin --cycles 50000
```
The CLI defaults to 100,000 cycles and stops early on `BRK`.
