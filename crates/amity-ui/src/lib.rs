//! Shared AmityOS design tokens and (from Phase 4) iced components.
//!
//! Token values come from the "AmityOS shell" design-language screenshot
//! (04 - Shell Design Language): v1, dark theme. Every Amity app and shell
//! surface takes colour, type, shape and depth from this crate, never from
//! local constants.

#[cfg(feature = "iced")]
pub mod kit;
pub mod tokens;

pub use tokens::{contrast_ratio, Color};
