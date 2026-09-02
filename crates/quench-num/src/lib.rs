//! The numbers that do not fit in a register.
//!
//! Everything else Quench has is a fixed number of bits — a `b64` is exactly 64, an
//! `i32` exactly 32 — and lives in a register, never touching the heap. `e` is not: it
//! is an exact rational, and adding two of them multiplies their denominators with
//! nothing to bound how large the result gets.
//!
//! `e` exists for numbers that are absurdly large *and* exactly represented, which is
//! what decides the shape of everything here. See `notes/e-is-big-and-exact.md`.

pub mod big;

pub use big::Big;
