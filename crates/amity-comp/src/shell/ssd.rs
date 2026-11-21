//! Compositor-drawn title bars, for windows that do not draw their own frame:
//! X11 applications and Wayland clients that ask for server-side decoration.
//!
//! Layout follows docs/specs/shell-behaviour.md §6.4: a 36 px bar with three
//! 38 px controls from the right edge (close, maximize or restore, minimize).
//! Controls act on press; a double-click on the rest of the bar toggles
//! maximize. The window title is not drawn yet (Phase 4).

use std::cell::{RefCell, RefMut};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::{SolidColorBuffer, SolidColorRenderElement},
                Kind,
            },
            ImportAll, ImportMem, Renderer,
        },
    },
    desktop::WindowSurface,
    input::Seat,
    utils::{Logical, Physical, Point, Scale, Serial, Transform},
};

use super::{ResizeEdge, WindowElement, WindowRenderElement};
use crate::{
    layout::{self, Edges, Tiling},
    state::{AmityState, Backend},
};

pub const HEADER_BAR_HEIGHT: i32 = 36;
const BUTTON_WIDTH: i32 = 38;

/// Linux input code of the primary pointer button.
const BTN_LEFT: u32 = 0x110;
const DOUBLE_CLICK_MS: u32 = 400;
const DOUBLE_CLICK_DISTANCE: f64 = 4.0;

// Design tokens (crates/amity-ui): Raised surface, Body text, and a hover wash.
const BAR_COLOR: [f32; 4] = [0.075, 0.071, 0.188, 1.0];
const HOVER_COLOR: [f32; 4] = [0.165, 0.161, 0.290, 1.0];
const CLOSE_HOVER_COLOR: [f32; 4] = [0.776, 0.196, 0.263, 1.0];
const GLYPH_COLOR: [u8; 3] = [0xC9, 0xC6, 0xE4];

/// Glyphs are rasterized at twice the logical size so they stay sharp up to 200 % scale.
const GLYPH_SCALE: i32 = 2;
/// Stroke width of the control glyphs in logical pixels.
const GLYPH_STROKE: f32 = 1.25;

type Segment = ((f32, f32), (f32, f32));

const CLOSE_GLYPH: &[Segment] = &[((14.0, 13.0), (24.0, 23.0)), ((24.0, 13.0), (14.0, 23.0))];
const MINIMIZE_GLYPH: &[Segment] = &[((14.0, 18.0), (24.0, 18.0))];
const MAXIMIZE_GLYPH: &[Segment] = &[
    ((14.0, 13.0), (24.0, 13.0)),
    ((24.0, 13.0), (24.0, 23.0)),
    ((24.0, 23.0), (14.0, 23.0)),
    ((14.0, 23.0), (14.0, 13.0)),
];
const RESTORE_GLYPH: &[Segment] = &[
    // front window
    ((13.0, 15.5), (21.5, 15.5)),
    ((21.5, 15.5), (21.5, 24.0)),
    ((21.5, 24.0), (13.0, 24.0)),
    ((13.0, 24.0), (13.0, 15.5)),
    // the window behind it
    ((16.0, 15.5), (16.0, 12.5)),
    ((16.0, 12.5), (24.5, 12.5)),
    ((24.5, 12.5), (24.5, 21.0)),
    ((24.5, 21.0), (21.5, 21.0)),
];

pub struct WindowState {
    pub is_ssd: bool,
    pub header_bar: HeaderBar,
}

/// What a press on the title bar asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderAction {
    Close,
    ToggleMaximize,
    Minimize,
    Move,
    /// Resize by the frame edges under the pointer.
    Resize(Edges),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zone {
    Close,
    Maximize,
    Minimize,
    Title,
}

struct Glyphs {
    close: MemoryRenderBuffer,
    maximize: MemoryRenderBuffer,
    restore: MemoryRenderBuffer,
    minimize: MemoryRenderBuffer,
}

pub struct HeaderBar {
    pointer_loc: Option<Point<f64, Logical>>,
    width: i32,
    background: SolidColorBuffer,
    hover: SolidColorBuffer,
    hovered: Option<Zone>,
    glyphs: Option<Glyphs>,
    last_press: Option<(u32, Point<f64, Logical>)>,
}

fn distance_to_segment(point: (f32, f32), (a, b): Segment) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared == 0.0 {
        0.0
    } else {
        (((point.0 - a.0) * dx + (point.1 - a.1) * dy) / length_squared).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((point.0 - cx).powi(2) + (point.1 - cy).powi(2)).sqrt()
}

/// Coverage (0–1) of the pixel centred on `point` by the stroked segments.
/// Coordinates are logical; `scale` is pixels per logical unit.
fn stroke_coverage(point: (f32, f32), segments: &[Segment], scale: f32) -> f32 {
    let distance = segments
        .iter()
        .map(|&segment| distance_to_segment(point, segment))
        .fold(f32::INFINITY, f32::min);
    ((GLYPH_STROKE / 2.0 - distance) * scale + 0.5).clamp(0.0, 1.0)
}

/// Rasterizes a glyph into premultiplied ARGB pixels, one button in size.
fn rasterize(segments: &[Segment]) -> (Vec<u8>, i32, i32) {
    let (width, height) = (BUTTON_WIDTH * GLYPH_SCALE, HEADER_BAR_HEIGHT * GLYPH_SCALE);
    let scale = GLYPH_SCALE as f32;
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let point = ((x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale);
            let coverage = stroke_coverage(point, segments, scale);
            if coverage > 0.0 {
                let offset = ((y * width + x) * 4) as usize;
                let [red, green, blue] = GLYPH_COLOR;
                // Argb8888 is stored as B, G, R, A; colour is premultiplied by alpha.
                pixels[offset] = (f32::from(blue) * coverage).round() as u8;
                pixels[offset + 1] = (f32::from(green) * coverage).round() as u8;
                pixels[offset + 2] = (f32::from(red) * coverage).round() as u8;
                pixels[offset + 3] = (255.0 * coverage).round() as u8;
            }
        }
    }
    (pixels, width, height)
}

fn glyph_buffer(segments: &[Segment]) -> MemoryRenderBuffer {
    let (pixels, width, height) = rasterize(segments);
    MemoryRenderBuffer::from_slice(
        &pixels,
        Fourcc::Argb8888,
        (width, height),
        GLYPH_SCALE,
        Transform::Normal,
        None,
    )
}

impl HeaderBar {
    fn new() -> Self {
        HeaderBar {
            pointer_loc: None,
            width: 0,
            background: SolidColorBuffer::default(),
            hover: SolidColorBuffer::default(),
            hovered: None,
            glyphs: None,
            last_press: None,
        }
    }

    fn zone_at(&self, x: f64) -> Zone {
        let from_right = f64::from(self.width) - x;
        if from_right <= 0.0 || self.width < BUTTON_WIDTH * 3 {
            // Too narrow for controls: the whole bar moves the window.
            Zone::Title
        } else if from_right <= f64::from(BUTTON_WIDTH) {
            Zone::Close
        } else if from_right <= f64::from(BUTTON_WIDTH * 2) {
            Zone::Maximize
        } else if from_right <= f64::from(BUTTON_WIDTH * 3) {
            Zone::Minimize
        } else {
            Zone::Title
        }
    }

    pub fn pointer_enter(&mut self, loc: Point<f64, Logical>) {
        self.pointer_loc = Some(loc);
    }

    pub fn pointer_leave(&mut self) {
        self.pointer_loc = None;
    }

    /// Where the pointer is, relative to the window's outer top-left corner.
    pub fn pointer_location(&self) -> Option<Point<f64, Logical>> {
        self.pointer_loc
    }

    /// A pointer button changed over the frame. `edges` are the resize edges
    /// under the pointer. Returns what to do, if anything.
    pub fn pointer_button(
        &mut self,
        button: u32,
        pressed: bool,
        time: u32,
        edges: Edges,
    ) -> Option<HeaderAction> {
        if !pressed || button != BTN_LEFT {
            return None;
        }
        let loc = self.pointer_loc?;
        if edges.any() {
            return Some(HeaderAction::Resize(edges));
        }
        if loc.y >= f64::from(HEADER_BAR_HEIGHT) {
            // Below the bar, which only the resize band reaches.
            return None;
        }
        match self.zone_at(loc.x) {
            Zone::Close => Some(HeaderAction::Close),
            Zone::Maximize => Some(HeaderAction::ToggleMaximize),
            Zone::Minimize => Some(HeaderAction::Minimize),
            Zone::Title => {
                let double_click = self.last_press.is_some_and(|(last_time, last_loc)| {
                    time.wrapping_sub(last_time) < DOUBLE_CLICK_MS
                        && (loc.x - last_loc.x).abs() <= DOUBLE_CLICK_DISTANCE
                        && (loc.y - last_loc.y).abs() <= DOUBLE_CLICK_DISTANCE
                });
                if double_click {
                    // A third press starts a fresh sequence.
                    self.last_press = None;
                    Some(HeaderAction::ToggleMaximize)
                } else {
                    self.last_press = Some((time, loc));
                    Some(HeaderAction::Move)
                }
            }
        }
    }

    /// A touch went down on the bar: only the title area starts a move.
    pub fn touch_down(&mut self) -> Option<HeaderAction> {
        let loc = self.pointer_loc?;
        (self.zone_at(loc.x) == Zone::Title).then_some(HeaderAction::Move)
    }

    /// A touch was lifted from the bar: controls act on release.
    pub fn touch_up(&mut self) -> Option<HeaderAction> {
        let loc = self.pointer_loc?;
        match self.zone_at(loc.x) {
            Zone::Close => Some(HeaderAction::Close),
            Zone::Maximize => Some(HeaderAction::ToggleMaximize),
            Zone::Minimize => Some(HeaderAction::Minimize),
            Zone::Title => None,
        }
    }

    /// Updates the buffers for the current width and pointer position.
    pub fn redraw(&mut self, width: i32) {
        self.width = width.max(0);
        if self.width == 0 {
            return;
        }
        self.background.update((self.width, HEADER_BAR_HEIGHT), BAR_COLOR);
        self.hovered = self
            .pointer_loc
            .filter(|loc| loc.y < f64::from(HEADER_BAR_HEIGHT))
            .map(|loc| self.zone_at(loc.x))
            .filter(|zone| *zone != Zone::Title);
        let hover_color = if self.hovered == Some(Zone::Close) {
            CLOSE_HOVER_COLOR
        } else {
            HOVER_COLOR
        };
        self.hover.update((BUTTON_WIDTH, HEADER_BAR_HEIGHT), hover_color);
    }

    /// Render elements for the bar, topmost first.
    pub fn render_elements<R>(
        &mut self,
        renderer: &mut R,
        location: Point<i32, Physical>,
        scale: Scale<f64>,
        alpha: f32,
        maximized: bool,
    ) -> Vec<WindowRenderElement<R>>
    where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Clone + Send + 'static,
    {
        let mut elements = Vec::with_capacity(5);
        let width = self.width;
        if width == 0 {
            return elements;
        }
        let button_origin = |index: i32| -> Point<i32, Physical> {
            let offset: Point<i32, Logical> = Point::from((width - BUTTON_WIDTH * (index + 1), 0));
            location + offset.to_physical_precise_round(scale)
        };

        if self.width >= BUTTON_WIDTH * 3 {
            let glyphs = self.glyphs.get_or_insert_with(|| Glyphs {
                close: glyph_buffer(CLOSE_GLYPH),
                maximize: glyph_buffer(MAXIMIZE_GLYPH),
                restore: glyph_buffer(RESTORE_GLYPH),
                minimize: glyph_buffer(MINIMIZE_GLYPH),
            });
            let middle = if maximized {
                &glyphs.restore
            } else {
                &glyphs.maximize
            };
            for (index, buffer) in [&glyphs.close, middle, &glyphs.minimize].into_iter().enumerate() {
                if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    button_origin(index as i32).to_f64(),
                    buffer,
                    Some(alpha),
                    None,
                    None,
                    Kind::Unspecified,
                ) {
                    elements.push(WindowRenderElement::Glyph(element));
                }
            }
            let hovered_index = match self.hovered {
                Some(Zone::Close) => Some(0),
                Some(Zone::Maximize) => Some(1),
                Some(Zone::Minimize) => Some(2),
                _ => None,
            };
            if let Some(index) = hovered_index {
                elements.push(WindowRenderElement::Decoration(
                    SolidColorRenderElement::from_buffer(
                        &self.hover,
                        button_origin(index),
                        scale,
                        alpha,
                        Kind::Unspecified,
                    ),
                ));
            }
        }
        elements.push(WindowRenderElement::Decoration(
            SolidColorRenderElement::from_buffer(&self.background, location, scale, alpha, Kind::Unspecified),
        ));
        elements
    }
}

impl WindowElement {
    /// The frame edges a pointer at `point` (relative to the window's outer
    /// top-left corner) would resize. Only windows with a compositor-drawn
    /// title bar have such edges, and only while they float: a maximized,
    /// snapped or fullscreen window has a fixed size.
    pub fn resize_edges_at(&self, point: Point<f64, Logical>) -> Edges {
        if !self.has_header() || self.tiling() != Tiling::Floating {
            return Edges::default();
        }
        layout::resize_edges(point, smithay::desktop::space::SpaceElement::geometry(self).size)
    }

    pub fn decoration_state(&self) -> RefMut<'_, WindowState> {
        self.user_data().insert_if_missing(|| {
            RefCell::new(WindowState {
                is_ssd: false,
                header_bar: HeaderBar::new(),
            })
        });

        self.user_data()
            .get::<RefCell<WindowState>>()
            .unwrap()
            .borrow_mut()
    }

    pub fn set_ssd(&self, ssd: bool) {
        self.decoration_state().is_ssd = ssd;
    }
}

impl<BackendData: Backend> AmityState<BackendData> {
    /// Carries out a title-bar action. Runs from an idle callback: the action
    /// may start a pointer grab or change the window's decoration state, which
    /// must not happen inside the input callback that produced it.
    pub fn header_action(
        &mut self,
        window: &WindowElement,
        action: HeaderAction,
        seat: &Seat<Self>,
        serial: Serial,
    ) {
        match action {
            HeaderAction::Close => self.close_window(window),
            HeaderAction::Minimize => self.minimize_window(window),
            HeaderAction::ToggleMaximize => {
                let target = if window.tiling() == Tiling::Floating {
                    Tiling::Maximized
                } else {
                    Tiling::Floating
                };
                self.set_tiling(window, target);
            }
            HeaderAction::Resize(edges) => {
                let mut resize = ResizeEdge::NONE;
                for (held, edge) in [
                    (edges.left, ResizeEdge::LEFT),
                    (edges.right, ResizeEdge::RIGHT),
                    (edges.top, ResizeEdge::TOP),
                    (edges.bottom, ResizeEdge::BOTTOM),
                ] {
                    if held {
                        resize |= edge;
                    }
                }
                self.resize_window(window, seat, serial, resize);
            }
            HeaderAction::Move => match window.0.underlying_surface() {
                WindowSurface::Wayland(toplevel) => {
                    let toplevel = toplevel.clone();
                    self.move_request_xdg(&toplevel, seat, serial);
                }
                #[cfg(feature = "xwayland")]
                WindowSurface::X11(surface) => {
                    let surface = surface.clone();
                    self.move_request_x11(&surface);
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(width: i32) -> HeaderBar {
        let mut bar = HeaderBar::new();
        bar.redraw(width);
        bar
    }

    fn press(bar: &mut HeaderBar, x: f64, y: f64, time: u32) -> Option<HeaderAction> {
        bar.pointer_enter(Point::from((x, y)));
        bar.pointer_button(BTN_LEFT, true, time, Edges::default())
    }

    #[test]
    fn a_press_on_a_frame_edge_resizes() {
        let mut bar = bar(800);
        let left = Edges {
            left: true,
            ..Edges::default()
        };
        // On the left edge of the title bar: resize wins over move.
        bar.pointer_enter(Point::from((2.0, 10.0)));
        assert_eq!(
            bar.pointer_button(BTN_LEFT, true, 0, left),
            Some(HeaderAction::Resize(left))
        );
        // On the left edge further down, below the bar.
        bar.pointer_enter(Point::from((2.0, 300.0)));
        assert_eq!(
            bar.pointer_button(BTN_LEFT, true, 0, left),
            Some(HeaderAction::Resize(left))
        );
        // Below the bar without an edge nothing happens.
        assert_eq!(bar.pointer_button(BTN_LEFT, true, 0, Edges::default()), None);
        // The secondary button never resizes.
        assert_eq!(bar.pointer_button(0x111, true, 0, left), None);
    }

    #[test]
    fn control_zones_are_38_px_from_the_right() {
        let mut bar = bar(800);
        assert_eq!(press(&mut bar, 799.0, 10.0, 0), Some(HeaderAction::Close));
        assert_eq!(press(&mut bar, 763.0, 10.0, 0), Some(HeaderAction::Close));
        assert_eq!(
            press(&mut bar, 761.0, 10.0, 0),
            Some(HeaderAction::ToggleMaximize)
        );
        assert_eq!(
            press(&mut bar, 725.0, 10.0, 0),
            Some(HeaderAction::ToggleMaximize)
        );
        assert_eq!(press(&mut bar, 723.0, 10.0, 0), Some(HeaderAction::Minimize));
        assert_eq!(press(&mut bar, 687.0, 10.0, 0), Some(HeaderAction::Minimize));
        assert_eq!(press(&mut bar, 685.0, 10.0, 0), Some(HeaderAction::Move));
        assert_eq!(press(&mut bar, 0.0, 10.0, 5000), Some(HeaderAction::Move));
    }

    #[test]
    fn only_a_primary_press_acts() {
        let mut bar = bar(800);
        bar.pointer_enter(Point::from((790.0, 10.0)));
        assert_eq!(bar.pointer_button(BTN_LEFT, false, 0, Edges::default()), None);
        assert_eq!(bar.pointer_button(0x111, true, 0, Edges::default()), None);
        bar.pointer_leave();
        assert_eq!(bar.pointer_button(BTN_LEFT, true, 0, Edges::default()), None);
    }

    #[test]
    fn double_click_on_the_title_toggles_maximize() {
        let mut bar = bar(800);
        assert_eq!(press(&mut bar, 200.0, 10.0, 1000), Some(HeaderAction::Move));
        assert_eq!(
            press(&mut bar, 203.0, 12.0, 1399),
            Some(HeaderAction::ToggleMaximize)
        );
        // The history is reset: a third quick press starts over.
        assert_eq!(press(&mut bar, 203.0, 12.0, 1500), Some(HeaderAction::Move));
        // Too slow.
        assert_eq!(press(&mut bar, 203.0, 12.0, 1900), Some(HeaderAction::Move));
        // Too far away.
        assert_eq!(press(&mut bar, 208.0, 12.0, 2000), Some(HeaderAction::Move));
    }

    #[test]
    fn a_bar_too_narrow_for_controls_only_moves() {
        let mut bar = bar(100);
        assert_eq!(press(&mut bar, 95.0, 10.0, 0), Some(HeaderAction::Move));
    }

    #[test]
    fn touch_uses_the_same_zones() {
        let mut bar = bar(800);
        bar.pointer_enter(Point::from((790.0, 10.0)));
        assert_eq!(bar.touch_down(), None);
        assert_eq!(bar.touch_up(), Some(HeaderAction::Close));
        bar.pointer_enter(Point::from((100.0, 10.0)));
        assert_eq!(bar.touch_down(), Some(HeaderAction::Move));
        assert_eq!(bar.touch_up(), None);
    }

    #[test]
    fn glyphs_are_antialiased_strokes_on_a_transparent_button() {
        let (pixels, width, height) = rasterize(CLOSE_GLYPH);
        assert_eq!(
            (width, height),
            (BUTTON_WIDTH * GLYPH_SCALE, HEADER_BAR_HEIGHT * GLYPH_SCALE)
        );
        assert_eq!(pixels.len(), (width * height * 4) as usize);
        let alpha_at = |x: i32, y: i32| pixels[((y * width + x) * 4 + 3) as usize];
        // The centre of the cross is fully covered; the corners are untouched.
        assert_eq!(alpha_at(38, 36), 255);
        assert_eq!(alpha_at(0, 0), 0);
        assert_eq!(alpha_at(width - 1, height - 1), 0);
        // Premultiplied: no colour channel exceeds alpha.
        for pixel in pixels.chunks_exact(4) {
            assert!(pixel[0] <= pixel[3] && pixel[1] <= pixel[3] && pixel[2] <= pixel[3]);
        }
        // Edges are antialiased: some pixels are partly covered.
        assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] > 0 && pixel[3] < 255));
    }

    #[test]
    fn stroke_coverage_falls_off_across_the_stroke_edge() {
        let segments = MINIMIZE_GLYPH;
        assert_eq!(stroke_coverage((19.0, 18.0), segments, 2.0), 1.0);
        assert_eq!(stroke_coverage((19.0, 30.0), segments, 2.0), 0.0);
        let edge = stroke_coverage((19.0, 18.0 + GLYPH_STROKE / 2.0), segments, 2.0);
        assert!((edge - 0.5).abs() < 1e-3);
    }
}
