//! The AmityOS shell: the desktop background, the taskbar, the launcher and
//! the window switcher. It is a Wayland client of `amity-comp`, which starts
//! it, gives it a privileged connection for its layer-shell surfaces, and
//! tells it about windows over a private channel.
//!
//! Behaviour reference: docs/specs/shell-behaviour.md. This is the Phase 3
//! shell; the look is brought in line with the design screenshots in Phase 4.

mod channel;
mod shell;
mod view;

use iced::{Font, Pixels, Theme};
use iced_layershell::{
    build_pattern::daemon,
    reexport::{Anchor, KeyboardInteractivity, Layer},
    settings::{LayerShellSettings, Settings, StartMode},
};

use shell::Shell;

pub fn main() -> Result<(), iced_layershell::Error> {
    // Before anything starts a thread: this reads and changes the environment.
    let connected = channel::init();
    if !connected {
        eprintln!("amity-shell: no compositor channel (AMITY_SHELL_FD); running unconnected");
    }

    daemon(
        move || Shell::boot(connected),
        Shell::namespace,
        Shell::update,
        Shell::view,
    )
    .subscription(Shell::subscription)
    .style(Shell::style)
    .theme(Theme::Dark)
    .settings(Settings {
        // The surfaces opened at start are the desktop background, one per
        // output; the taskbar and the overlays are opened by the shell itself.
        layer_settings: LayerShellSettings {
            anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
            layer: Layer::Background,
            exclusive_zone: -1,
            // Anchored to all four edges: 0 lets the compositor choose the size.
            size: Some((0, 0)),
            margin: (0, 0, 0, 0),
            keyboard_interactivity: KeyboardInteractivity::None,
            start_mode: StartMode::AllScreens,
            events_transparent: false,
        },
        default_font: Font::with_name(amity_ui::tokens::typography::BODY.family),
        default_text_size: Pixels(amity_ui::tokens::typography::BODY.size),
        antialiasing: true,
        ..Settings::default()
    })
    .run()
}
