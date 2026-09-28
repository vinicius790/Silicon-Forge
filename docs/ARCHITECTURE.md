# Architecture Overview

The MOS 6502 Emulator Suite is designed around strong separation of concerns, leveraging Rust's trait system for modularity.

## Module Dependency Diagram

```text
                        +-------------+
                        |     CLI     |
                        | (main.rs)   |
                        +------+------+
                               |
       +-----------------------+-----------------------+
       |                       |                       |
+------v------+         +------v------+         +------v------+
|             |         |             |         |             |
|   Systems   |         |    Tools    |         |    Bench    |
| (apple1.rs) |         |  (tui.rs)   |         | (bench.rs)  |
| (kim1.rs)   |         | (disasm.rs) |         |             |
| (basic.rs)  |         +------+------+         +------+------+
+------+------+                |                       |
       |                       |                       |
       +-----------------------+-----------------------+
                               |
                        +------v------+
                        |             |
                        |   Cpu<B>    |
                        |  (cpu.rs)   |
                        |             |
                        +------+------+
                               |
                        +------v------+
                        |             |
                        |  MemoryBus  |
                        |  (bus.rs)   |
                        |             |
                        +------+------+
                               |
               +---------------+---------------+
               |               |               |
        +------v------+ +------v------+ +------v------+
        |             | |             | |             |
        | FlatMemory  | |  SystemBus  | |  Graphics   |
        |             | |             | |             |
        +-------------+ +-------------+ +-------------+
```

## Core Traits

* `MemoryBus`: The primary trait the `Cpu` requires to function. It defines `read`, `write`, and `peek` operations.
* `IoDevice`: Used by `SystemBus` to map complex hardware devices (timers, video chips, VIAs) into the 6502 memory space.

## Modules

* **`graphics`**: Fantasy-console framebuffer, input/RNG device, mapped bus, and terminal renderer.
* **`bench`**: CPU throughput benchmark exposed by the CLI.
* **`systems/apple1`**: WozMon ROM and a host-driven keyboard/display interface.
* **`systems/basic`**: Host-side educational BASIC subset; it is not a 6502 ROM.
* **`systems/kim1`** and **`systems/beneater`**: Partial hardware models, not complete ROM-backed machines.
