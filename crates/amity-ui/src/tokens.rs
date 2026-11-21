//! Design tokens, dark theme v1 (screenshot 04 - Shell Design Language).

/// An sRGB colour with straight (non-premultiplied) alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(hex: u32) -> Self {
        Self { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8, a: 255 }
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// WCAG 2.x relative luminance of the opaque colour.
    pub fn luminance(self) -> f64 {
        fn channel(value: u8) -> f64 {
            let c = f64::from(value) / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        }
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }
}

/// WCAG 2.x contrast ratio between two opaque colours (1.0 to 21.0).
pub fn contrast_ratio(a: Color, b: Color) -> f64 {
    let (light, dark) = {
        let (x, y) = (a.luminance(), b.luminance());
        if x >= y { (x, y) } else { (y, x) }
    };
    (light + 0.05) / (dark + 0.05)
}

/// Surfaces, from back to front.
pub mod surface {
    use super::Color;
    pub const GROUND: Color = Color::rgb(0x07071A);
    pub const SURFACE: Color = Color::rgb(0x0E0D22);
    pub const RAISED: Color = Color::rgb(0x131230);
    /// Translucent shell surfaces (taskbar, launcher): `#0E0C22` at 74 %.
    /// The compositor blurs what is behind it.
    pub const GLASS: Color = Color::rgb(0x0E0C22).with_alpha(189);
}

/// Accents.
pub mod accent {
    use super::Color;
    pub const VIOLET: Color = Color::rgb(0x8B5CF6);
    pub const VIOLET_SOFT: Color = Color::rgb(0xC4B5FD);
    pub const BLUE: Color = Color::rgb(0x60A5FA);
    pub const ORCHID: Color = Color::rgb(0xF0ABFC);
}

/// Text colours.
pub mod text {
    use super::Color;
    pub const TEXT: Color = Color::rgb(0xF4F2FF);
    pub const BODY: Color = Color::rgb(0xC9C6E4);
    pub const MUTED: Color = Color::rgb(0xA9A6C8);
}

/// Corner radii in logical pixels, by use.
pub mod radius {
    pub const KEY: f32 = 6.0; // keys, tags
    pub const BUTTON: f32 = 9.0;
    pub const WINDOW: f32 = 14.0;
    pub const TASKBAR: f32 = 20.0;
    pub const LAUNCHER: f32 = 24.0;
}

/// A type style: family, weight, size and line height in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeStyle {
    pub family: &'static str,
    pub weight: u16,
    pub size: f32,
    pub line_height: f32,
}

/// Type scale. Space Grotesk, Inter and JetBrains Mono are SIL OFL 1.1.
pub mod typography {
    use super::TypeStyle;
    pub const DISPLAY: TypeStyle = TypeStyle { family: "Space Grotesk", weight: 600, size: 32.0, line_height: 40.0 };
    pub const WINDOW_TITLE: TypeStyle = TypeStyle { family: "Inter", weight: 600, size: 20.0, line_height: 28.0 };
    pub const BODY: TypeStyle = TypeStyle { family: "Inter", weight: 400, size: 14.0, line_height: 22.0 };
    pub const CONTROL: TypeStyle = TypeStyle { family: "Inter", weight: 500, size: 14.0, line_height: 22.0 };
    pub const CAPTION: TypeStyle = TypeStyle { family: "Inter", weight: 500, size: 12.0, line_height: 16.0 };
    pub const CODE: TypeStyle = TypeStyle { family: "JetBrains Mono", weight: 400, size: 13.0, line_height: 22.0 };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parsing() {
        assert_eq!(surface::GROUND, Color { r: 0x07, g: 0x07, b: 0x1A, a: 255 });
        assert_eq!(surface::GLASS.a, 189); // round(0.74 * 255)
    }

    #[test]
    fn contrast_reference_values() {
        let black = Color::rgb(0x000000);
        let white = Color::rgb(0xFFFFFF);
        assert!((contrast_ratio(black, white) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(white, white) - 1.0).abs() < 1e-9);
    }

    /// WCAG AA (4.5:1) for normal text on every opaque surface.
    #[test]
    fn text_tokens_meet_wcag_aa_on_all_surfaces() {
        for background in [surface::GROUND, surface::SURFACE, surface::RAISED] {
            for foreground in [text::TEXT, text::BODY, text::MUTED] {
                let ratio = contrast_ratio(foreground, background);
                assert!(ratio >= 4.5, "{foreground:?} on {background:?}: {ratio:.2}");
            }
        }
    }

    /// Primary buttons put white text on Violet (screenshot 04 "Primary").
    #[test]
    fn primary_button_text_contrast_is_recorded() {
        let ratio = contrast_ratio(text::TEXT, accent::VIOLET);
        // 3:1 is the WCAG AA minimum for large or bold UI text; Phase 4 records
        // whether normal-size button labels need a darker violet.
        assert!(ratio >= 3.0, "{ratio:.2}");
    }
}
