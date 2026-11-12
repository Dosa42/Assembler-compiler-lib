//! `ufa_baremetal_core` — Rust port of the `ufa_baremetal` Python package.
//!
//! The six K-invariants codified as pure Rust predicates over `.s` text,
//! ported from `k_invariants.py` / `parse_s.py` / `verdict.py` /
//! `codegen_audit.py` / `regression_map.py`.
//!
//! Per the UFA Bare-Metal Canon brief §2.6, `k_invariants` is meant to stay
//! the ONE catalog file — adding a K means adding one function here and one
//! `K_REGISTRY` entry, no subclass hierarchies, no plugin loading. That
//! discipline is preserved in this port: [`k_invariants`] stays a single
//! module.
//!
//! Non-negotiables carried over from brief §5.1: deterministic (same `.s`
//! in, same verdicts out — no wall-clock/pid noise in the check logic
//! itself), pure functional core (no I/O, no global mutable state inside a
//! check), verdicts are only PASS/FAIL/NOT_APPLICABLE (no probabilistic
//! scoring), stdlib only.

pub mod codegen_audit;
pub mod datetime;
pub mod json;
pub mod k_invariants;
pub mod parse_s;
pub mod pattern;
pub mod regression_map;
pub mod sha256;
pub mod verdict;

pub use verdict::{Status, Verdict};
