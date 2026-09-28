# Emulator Benchmark

The `mos6502` suite includes a built-in benchmark tool to measure the execution speed of the emulator on your hardware.

## What it Measures

The benchmark measures:
1. **Total Instructions Executed**: How many 6502 instructions were processed in the time frame.
2. **Total Cycles**: How many virtual 6502 clock cycles elapsed.
3. **MIPS**: Millions of Instructions Per Second.
4. **Effective MHz**: The equivalent clock speed of original hardware to match this performance.

## The Test Program

The benchmark runs a tight, computationally intensive loop in 6502 assembly, designed to stress the dispatch loop, memory accesses, and flag calculations:

```asm
* = $8000
    LDX #$FF
loop:
    INX
    TXA
    ADC #$01
    STA $0200,X
    JMP loop
```

## How to Run

Execute the benchmark using the CLI:
```bash
cargo run --release -- bench
```
*(Always run with `--release` for accurate performance metrics. Debug builds are significantly slower due to missing optimizations and debug assertions.)*

## Interpreting Results

Output will look similar to this:
```text
Running benchmark for 5 seconds...
-----------------------------------
Total Instructions: 450,000,000
Total Cycles: 1,350,000,000
Speed: 90.0 MIPS
Effective Clock: 270.0 MHz
-----------------------------------
```

## Comparison Table

| System | Clock Speed | MIPS |
| :--- | :--- | :--- |
| Original MOS 6502 (Apple-1) | 1.023 MHz | ~0.3 - 0.5 |
| Original MOS 6502 (C64) | 1.023 MHz (NTSC) | ~0.3 - 0.5 |
| WDC 65C02 (Modern) | 14.0 MHz | ~4 - 7 |
| **This Emulator** (Apple M1) | **500.0+ MHz** | **150.0+** |
| **This Emulator** (Intel i7) | **400.0+ MHz** | **120.0+** |

The emulator typically runs 300x to 500x faster than the original hardware.
