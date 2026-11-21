//! The design tokens as iced values, and the styles shell surfaces share.
//!
//! This is the start of the component layer. Phase 4 extracts the full design
//! system from the screenshots; until then only what the Phase 3 shell needs
//! lives here, and values that are not yet measured from the design are
//! marked provisional.

use iced::{
    border::Border,
    font::{Family, Weight},
    widget::{button, container, text_input},
    Background, Color, Font, Shadow,
};

use crate::tokens::{accent, radius, surface, text, TypeStyle};

/// Converts a token colour.
pub fn color(token: crate::Color) -> Color {
    Color::from_rgba8(token.r, token.g, token.b, f32::from(token.a) / 255.0)
}

fn with_alpha(token: crate::Color, alpha: f32) -> Color {
    Color {
        a: alpha,
        ..color(token)
    }
}

/// Hairline around panels and controls. Provisional: white at 8 %.
pub fn hairline() -> Color {
    Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.08)
}

fn weight(value: u16) -> Weight {
    match value {
        0..=349 => Weight::Light,
        350..=449 => Weight::Normal,
        450..=549 => Weight::Medium,
        550..=649 => Weight::Semibold,
        _ => Weight::Bold,
    }
}

/// The font of a type style.
pub fn font(style: TypeStyle) -> Font {
    Font {
        family: Family::Name(style.family),
        weight: weight(style.weight),
        ..Font::DEFAULT
    }
}

fn panel(background: Color, radius: f32) -> container::Style {
    container::Style {
        text_color: Some(color(text::BODY)),
        background: Some(Background::Color(background)),
        border: Border {
            color: hairline(),
            width: 1.0,
            radius: radius.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// A translucent shell surface: the taskbar and the launcher. The compositor
/// is meant to blur what lies behind it (Phase 4).
///
/// No drop shadow yet: `iced_tiny_skia` 0.14 draws a quad's shadow without the
/// clip of a partial redraw, so each redraw darkens the rest of the surface
/// again. Shadows return with the Phase 4 renderer decision.
pub fn glass(radius: f32) -> container::Style {
    panel(color(surface::GLASS), radius)
}

/// The taskbar's surface.
pub fn taskbar() -> container::Style {
    glass(radius::TASKBAR)
}

/// The launcher's and the switcher's surface.
pub fn overlay_panel() -> container::Style {
    // More opaque than the taskbar until the compositor blurs its backdrop,
    // so text stays readable over any window.
    glass(radius::LAUNCHER).background(with_alpha(surface::SURFACE, 0.96))
}

/// The dimmed backdrop behind the launcher.
pub fn scrim() -> container::Style {
    container::Style {
        background: Some(Background::Color(with_alpha(surface::GROUND, 0.55))),
        ..container::Style::default()
    }
}

/// A key cap in a shortcut hint, such as "Super".
pub fn key_cap() -> container::Style {
    container::Style {
        text_color: Some(color(text::BODY)),
        ..panel(Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.06), radius::KEY)
    }
}

/// The square tile that stands in for an application icon.
pub fn monogram(active: bool) -> container::Style {
    let background = if active {
        with_alpha(accent::VIOLET, 0.32)
    } else {
        Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.07)
    };
    container::Style {
        text_color: Some(color(text::TEXT)),
        ..panel(background, radius::BUTTON)
    }
}

/// The marker under the taskbar entry of the active window.
pub fn indicator(active: bool) -> container::Style {
    container::Style {
        background: Some(Background::Color(if active {
            color(accent::VIOLET)
        } else {
            with_alpha(text::MUTED, 0.6)
        })),
        border: Border {
            radius: 2.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

fn flat_button(status: button::Status, highlighted: bool, radius: f32) -> button::Style {
    let background = match (status, highlighted) {
        (button::Status::Pressed, _) => Some(Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.14)),
        (button::Status::Hovered, _) => Some(Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.10)),
        (_, true) => Some(Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.07)),
        _ => None,
    };
    button::Style {
        background: background.map(Background::Color),
        text_color: color(text::TEXT),
        border: Border {
            radius: radius.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// A taskbar entry; `active` marks the focused window.
pub fn taskbar_button(status: button::Status, active: bool) -> button::Style {
    flat_button(status, active, radius::WINDOW)
}

/// A row in a list such as launcher results; `selected` follows the keyboard.
pub fn list_row(status: button::Status, selected: bool) -> button::Style {
    let mut style = flat_button(status, selected, radius::BUTTON);
    if selected {
        style.background = Some(Background::Color(with_alpha(accent::VIOLET, 0.22)));
        style.border = Border {
            color: with_alpha(accent::VIOLET, 0.55),
            width: 1.0,
            radius: radius::BUTTON.into(),
        };
    }
    style
}

/// The primary action button (the launcher button on the taskbar).
pub fn primary_button(status: button::Status) -> button::Style {
    let base = color(accent::VIOLET);
    let background = match status {
        button::Status::Hovered => Color { a: 0.9, ..base },
        button::Status::Pressed => Color { a: 0.8, ..base },
        _ => base,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: color(text::TEXT),
        border: Border {
            radius: radius::WINDOW.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// The search field of the launcher.
pub fn search_field(status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.05)),
        border: Border {
            color: if focused {
                with_alpha(accent::VIOLET, 0.8)
            } else {
                hairline()
            },
            width: if focused { 1.5 } else { 1.0 },
            radius: radius::WINDOW.into(),
        },
        icon: color(text::MUTED),
        placeholder: color(text::MUTED),
        value: color(text::TEXT),
        selection: with_alpha(accent::VIOLET, 0.45),
    }
}
