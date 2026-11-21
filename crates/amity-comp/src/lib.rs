//! The AmityOS Wayland compositor: window management, outputs, input,
//! XWayland, and (later) per-Space sessions and the shell protocol.
//!
//! The core started from Smithay's "anvil" example compositor (MIT; see
//! LICENSE-SMITHAY.txt) and is being reworked into AmityOS's own compositor.
#![warn(rust_2018_idioms)]
// If no backend is enabled, a large portion of the codebase is unused.
// So silence this useless warning for the CI.
#![cfg_attr(
    not(any(feature = "winit", feature = "x11", feature = "udev")),
    allow(dead_code, unused_imports)
)]

#[cfg(any(feature = "udev", feature = "xwayland"))]
pub mod cursor;
pub mod debug;
#[cfg(feature = "udev")]
pub mod display_info;
pub mod drawing;
pub mod focus;
pub mod input_handler;
pub mod layout;
pub mod render;
pub mod shell;
pub mod shell_link;
pub mod state;
#[cfg(feature = "udev")]
pub mod udev;
#[cfg(feature = "winit")]
pub mod winit;
pub mod wm;

pub use state::{AmityState, ClientState};
