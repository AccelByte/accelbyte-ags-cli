//! Text decorations (symbols, prefixes). Extracted so non-ANSI backends can
//! replace or strip them.

pub const SYMBOL_SUCCESS: &str = "\u{2714}"; // ✔
pub const SYMBOL_ERROR: &str = "\u{2715}"; // ✖
pub const SYMBOL_WARNING: &str = "!";
pub const SYMBOL_INFO: &str = "\u{203a}"; // ›
pub const SYMBOL_STATUS: &str = "\u{2218}"; // ∘
pub const SYMBOL_FIX: &str = "\u{2192}"; // →
