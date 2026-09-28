//! System examples and experimental hardware models built around the MOS 6502.
//!
//! The Apple-1 runs the project's own monitor (firmware/apple1_monitor.asm). Mini BASIC is a host-side educational
//! interpreter; KIM-1 and Ben Eater currently provide partial, embeddable models.

pub mod apple1;
pub mod basic;
pub mod beneater;
pub mod kim1;
pub mod nes2a03;
pub mod sbc;
pub mod term;
pub mod via_board;
pub mod c64;
pub mod atari2600;
