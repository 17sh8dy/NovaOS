//! The kernel's font lives in `crates/gfx` (shared with the desktop); this
//! keeps the old `crate::font` path working.

pub use nova_gfx::font::*;
