//! Geometry rules for placing and fitting windows.
//!
//! The fitting and cascade rules follow docs/specs/shell-behaviour.md §6; the
//! test vectors at the end of that document are the tests here.

use smithay::utils::{Logical, Point, Rectangle, Size};

/// How a window is arranged on its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tiling {
    /// Free position and size.
    #[default]
    Floating,
    /// Fills the work area.
    Maximized,
    /// Fills the left half of the work area.
    Left,
    /// Fills the right half of the work area.
    Right,
}

/// Smallest size given to a window that is dragged off its maximized or
/// snapped state without a floating size to return to, in logical pixels.
pub const MINIMUM_WINDOW: (i32, i32) = (320, 200);

/// Fits a requested length between a minimum and the available length. The
/// minimum never exceeds the available length and the result is never below 1.
pub fn fit_size(requested: i32, minimum: i32, available: i32) -> i32 {
    let available = available.max(1);
    requested.clamp(minimum.max(1).min(available), available)
}

/// Fits a position so that `size` stays inside `available`, measured from 0.
/// Something larger than the available length is placed at 0.
pub fn fit_position(requested: i32, available: i32, size: i32) -> i32 {
    requested.clamp(0, available.saturating_sub(size.max(1)).max(0))
}

/// Cascade offset inside the work area for the `slot`th new window.
pub fn cascade_offset(slot: u32, area: Size<i32, Logical>) -> Point<i32, Logical> {
    let slot = i64::from(slot);
    let x = 130 + (38 * slot) % i64::from(area.w.saturating_sub(480).max(1));
    let y = 68 + (34 * slot) % 180;
    Point::from((x as i32, y as i32))
}

/// Margin kept between a new window and the edges of the work area, in total
/// per axis, when the window asked for more room than there is.
pub const NEW_WINDOW_MARGIN: i32 = 16;

/// The size a new window gets: what it asked for, unless that is more than the
/// work area minus the margin. Small windows are never enlarged.
pub fn fit_new_size(requested: Size<i32, Logical>, area: Size<i32, Logical>) -> Size<i32, Logical> {
    Size::from((
        requested.w.min((area.w - NEW_WINDOW_MARGIN).max(1)),
        requested.h.min((area.h - NEW_WINDOW_MARGIN).max(1)),
    ))
}

/// Initial position of a new window of `size`: the cascade position, pulled
/// back so the window lies inside the work area.
pub fn place_new(slot: u32, size: Size<i32, Logical>, area: Rectangle<i32, Logical>) -> Point<i32, Logical> {
    let offset = cascade_offset(slot, area.size);
    Point::from((
        area.loc.x + fit_position(offset.x, area.size.w, size.w),
        area.loc.y + fit_position(offset.y, area.size.h.saturating_sub(8), size.h),
    ))
}

/// Moves `loc` so that a window of `size` lies inside `area`.
pub fn fit_into(
    loc: Point<i32, Logical>,
    size: Size<i32, Logical>,
    area: Rectangle<i32, Logical>,
) -> Point<i32, Logical> {
    Point::from((
        area.loc.x + fit_position(loc.x.saturating_sub(area.loc.x), area.size.w, size.w),
        area.loc.y + fit_position(loc.y.saturating_sub(area.loc.y), area.size.h, size.h),
    ))
}

/// Shrinks and moves a rectangle so that it lies inside `area`. A rectangle
/// that fits keeps its size: small windows such as dialogs are never enlarged.
pub fn fit_rect(rect: Rectangle<i32, Logical>, area: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    let size = Size::from((
        rect.size.w.min(area.size.w).max(1),
        rect.size.h.min(area.size.h).max(1),
    ));
    Rectangle::new(fit_into(rect.loc, size, area), size)
}

/// The rectangle a tiled window occupies in `area`. `None` for floating windows.
pub fn tiled_rect(tiling: Tiling, area: Rectangle<i32, Logical>) -> Option<Rectangle<i32, Logical>> {
    let half = area.size.w / 2;
    match tiling {
        Tiling::Floating => None,
        Tiling::Maximized => Some(area),
        Tiling::Left => Some(Rectangle::new(area.loc, Size::from((half, area.size.h)))),
        Tiling::Right => Some(Rectangle::new(
            Point::from((area.loc.x + half, area.loc.y)),
            Size::from((area.size.w - half, area.size.h)),
        )),
    }
}

/// Keeps a dragged window reachable: its top edge stays inside the work area
/// (so the title bar never goes above the screen or under the taskbar) and at
/// least `grip` pixels of its width stay inside the area horizontally.
pub fn clamp_dragged(
    loc: Point<i32, Logical>,
    size: Size<i32, Logical>,
    area: Rectangle<i32, Logical>,
    title_height: i32,
    grip: i32,
) -> Point<i32, Logical> {
    let grip = grip.clamp(1, size.w.max(1));
    let min_x = area.loc.x.saturating_sub(size.w.max(1) - grip);
    let max_x = (area.loc.x + area.size.w - grip).max(min_x);
    let min_y = area.loc.y;
    let max_y = (area.loc.y + area.size.h - title_height.max(1)).max(min_y);
    Point::from((loc.x.clamp(min_x, max_x), loc.y.clamp(min_y, max_y)))
}

/// Which sides of a window a pointer position takes hold of for resizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl Edges {
    pub fn any(self) -> bool {
        self.left || self.right || self.top || self.bottom
    }
}

/// Width of the band just inside a window's outer frame that resizes it, in
/// logical pixels. Two edges meet in the corners, where the band is wider so
/// that a diagonal resize is easy to hit.
pub const RESIZE_BAND: f64 = 5.0;
const RESIZE_CORNER: f64 = 14.0;

/// The edges a pointer at `point` (relative to the window's outer top-left
/// corner) resizes, for a window of the outer `size`. None for a window too
/// small to tell its edges from its middle.
pub fn resize_edges(point: Point<f64, Logical>, size: Size<i32, Logical>) -> Edges {
    let (w, h) = (f64::from(size.w), f64::from(size.h));
    if w < 4.0 * RESIZE_CORNER || h < 4.0 * RESIZE_CORNER {
        return Edges::default();
    }
    let near = |distance: f64, band: f64| (0.0..band).contains(&distance);
    let mut edges = Edges {
        left: near(point.x, RESIZE_BAND),
        right: near(w - point.x, RESIZE_BAND) && point.x < w,
        top: near(point.y, RESIZE_BAND),
        bottom: near(h - point.y, RESIZE_BAND) && point.y < h,
    };
    // In a corner, being on one edge also takes the adjacent one.
    if edges.left || edges.right {
        edges.top |= near(point.y, RESIZE_CORNER);
        edges.bottom |= near(h - point.y, RESIZE_CORNER) && point.y < h;
    }
    if edges.top || edges.bottom {
        edges.left |= near(point.x, RESIZE_CORNER);
        edges.right |= near(w - point.x, RESIZE_CORNER) && point.x < w;
    }
    edges
}

/// What happens to a window when the monitor layout changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relocation {
    /// Its monitor did not move, or it was not on a monitor.
    Stay,
    /// Its monitor moved by this much, and the window moves along.
    Shift(Point<i32, Logical>),
    /// Its monitor is gone.
    Orphaned,
}

/// Decides how a window centred at `centre` follows a change of the monitor
/// layout from `before` to `after` (monitor name and geometry).
pub fn relocate(
    centre: Point<i32, Logical>,
    before: &[(String, Rectangle<i32, Logical>)],
    after: &[(String, Rectangle<i32, Logical>)],
) -> Relocation {
    let Some((name, old)) = before.iter().find(|(_, geometry)| geometry.contains(centre)) else {
        return Relocation::Stay;
    };
    match after.iter().find(|(other, _)| other == name) {
        None => Relocation::Orphaned,
        Some((_, new)) if new.loc == old.loc => Relocation::Stay,
        Some((_, new)) => Relocation::Shift(new.loc - old.loc),
    }
}

/// Index selected after one switcher step, wrapping around `count` entries.
pub fn step_index(current: usize, count: usize, reverse: bool) -> usize {
    if count == 0 {
        0
    } else if reverse {
        (current + count - 1) % count
    } else {
        (current + 1) % count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new(Point::from((x, y)), Size::from((w, h)))
    }

    // TV-17
    #[test]
    fn size_and_position_fitting() {
        assert_eq!(fit_size(700, 600, 572), 572);
        assert_eq!(fit_size(100, 900, 1008), 900);
        assert_eq!(fit_size(100, 600, 620), 600);
        assert_eq!(fit_size(900, 600, 620), 620);
        assert_eq!(fit_size(i32::MAX, i32::MAX, 0), 1);
        assert_eq!(fit_size(i32::MIN, -1, -1), 1);
        assert_eq!(fit_position(i32::MIN, 100, 50), 0);
        assert_eq!(fit_position(i32::MAX, 100, 50), 50);
        assert_eq!(fit_position(20, 100, i32::MAX), 0);
    }

    // TV-18
    #[test]
    fn cascade_placement() {
        let area = rect(0, 0, 1024, 680);
        let size = Size::from((800, 600));
        assert_eq!(place_new(0, size, area), Point::from((130, 68)));
        assert_eq!(place_new(3, size, area), Point::from((224, 72)));
    }

    #[test]
    fn a_new_window_is_never_larger_than_the_work_area() {
        let area = Size::from((1280, 724));
        // Fits: unchanged (TV-18: 800 x 600 stays 800 x 600).
        assert_eq!(fit_new_size(Size::from((800, 600)), area), Size::from((800, 600)));
        // Taller than the work area: cut to the area minus the margin.
        assert_eq!(
            fit_new_size(Size::from((1018, 740)), area),
            Size::from((1018, 708))
        );
        assert_eq!(
            fit_new_size(Size::from((4000, 3000)), area),
            Size::from((1264, 708))
        );
        // Exactly the work area still gets the margin.
        assert_eq!(fit_new_size(area, area), Size::from((1264, 708)));
        // Small windows are not enlarged, and nothing degenerates.
        assert_eq!(fit_new_size(Size::from((200, 80)), area), Size::from((200, 80)));
        assert_eq!(
            fit_new_size(Size::from((500, 500)), Size::from((10, 10))),
            Size::from((1, 1))
        );
    }

    #[test]
    fn cascade_respects_the_work_area_origin_and_never_overflows() {
        // A second monitor to the right, with a panel taking the top 40 px.
        let area = rect(1920, 40, 1280, 680);
        let size = Size::from((640, 480));
        assert_eq!(place_new(0, size, area), Point::from((1920 + 130, 40 + 68)));
        for slot in [0, 1, 7, 100, u32::MAX] {
            let loc = place_new(slot, size, area);
            assert!(
                loc.x >= area.loc.x && loc.x + size.w <= area.loc.x + area.size.w,
                "slot {slot}"
            );
            assert!(
                loc.y >= area.loc.y && loc.y + size.h <= area.loc.y + area.size.h,
                "slot {slot}"
            );
        }
        // A window larger than the area is aligned to its corner.
        assert_eq!(place_new(5, Size::from((4000, 3000)), area), area.loc);
    }

    #[test]
    fn fitting_a_rectangle_into_a_smaller_area() {
        let area = rect(100, 50, 800, 600);
        assert_eq!(fit_rect(rect(120, 60, 400, 300), area), rect(120, 60, 400, 300));
        assert_eq!(fit_rect(rect(700, 500, 400, 300), area), rect(500, 350, 400, 300));
        assert_eq!(
            fit_rect(rect(-50, -50, 2000, 2000), area),
            rect(100, 50, 800, 600)
        );
        // A small window is moved if necessary but never enlarged.
        assert_eq!(fit_rect(rect(120, 60, 200, 100), area), rect(120, 60, 200, 100));
        assert_eq!(
            fit_rect(rect(5000, 5000, 200, 100), area),
            rect(700, 550, 200, 100)
        );
        // Degenerate input does not produce an empty or inverted rectangle.
        let fitted = fit_rect(rect(0, 0, 0, 0), area);
        assert_eq!(fitted.size, Size::from((1, 1)));
        assert!(area.contains_rect(fitted));
    }

    #[test]
    fn tiled_rectangles_cover_the_area_without_gaps() {
        let area = rect(0, 0, 1281, 738);
        assert_eq!(tiled_rect(Tiling::Floating, area), None);
        assert_eq!(tiled_rect(Tiling::Maximized, area), Some(area));
        let left = tiled_rect(Tiling::Left, area).unwrap();
        let right = tiled_rect(Tiling::Right, area).unwrap();
        assert_eq!(left, rect(0, 0, 640, 738));
        assert_eq!(right, rect(640, 0, 641, 738));
        assert_eq!(left.size.w + right.size.w, area.size.w);
    }

    #[test]
    fn dragged_windows_stay_reachable() {
        let area = rect(0, 0, 1280, 738);
        let size = Size::from((600, 400));
        // Above the screen: pulled back down.
        assert_eq!(
            clamp_dragged(Point::from((100, -80)), size, area, 36, 120),
            Point::from((100, 0))
        );
        // Under the taskbar: the title bar stays above it.
        assert_eq!(
            clamp_dragged(Point::from((100, 900)), size, area, 36, 120),
            Point::from((100, 702))
        );
        // Far left and right: a grip stays visible.
        assert_eq!(
            clamp_dragged(Point::from((-5000, 10)), size, area, 36, 120),
            Point::from((-480, 10))
        );
        assert_eq!(
            clamp_dragged(Point::from((5000, 10)), size, area, 36, 120),
            Point::from((1160, 10))
        );
        // Positions inside are left alone.
        assert_eq!(
            clamp_dragged(Point::from((300, 200)), size, area, 36, 120),
            Point::from((300, 200))
        );
    }

    #[test]
    fn resize_edges_are_a_band_inside_the_frame() {
        let size = Size::from((800, 600));
        let at = |x: f64, y: f64| resize_edges(Point::from((x, y)), size);
        let edges = |left, right, top, bottom| Edges {
            left,
            right,
            top,
            bottom,
        };
        // The middle of the window and of the title bar resize nothing.
        assert!(!at(400.0, 300.0).any());
        assert!(!at(400.0, 18.0).any());
        // The middle of each side.
        assert_eq!(at(2.0, 300.0), edges(true, false, false, false));
        assert_eq!(at(797.0, 300.0), edges(false, true, false, false));
        assert_eq!(at(400.0, 2.0), edges(false, false, true, false));
        assert_eq!(at(400.0, 597.0), edges(false, false, false, true));
        // The band is 5 px wide and lies inside the frame.
        assert!(at(4.9, 300.0).left && !at(5.0, 300.0).any());
        assert!(at(795.1, 300.0).right && !at(795.0, 300.0).any());
        // Corners take both edges, from a little further along either side.
        assert_eq!(at(2.0, 2.0), edges(true, false, true, false));
        assert_eq!(at(797.0, 597.0), edges(false, true, false, true));
        assert_eq!(at(2.0, 590.0), edges(true, false, false, true));
        assert_eq!(at(790.0, 2.0), edges(false, true, true, false));
        assert_eq!(at(2.0, 20.0), edges(true, false, false, false));
        // Positions outside the window resize nothing.
        assert!(!at(-1.0, 300.0).any());
        assert!(!at(800.0, 300.0).any());
        assert!(!at(400.0, 600.0).any());
        // A tiny window cannot be resized by its edges.
        assert!(!resize_edges(Point::from((1.0, 1.0)), Size::from((40, 40))).any());
    }

    #[test]
    fn windows_follow_their_monitor_when_the_layout_changes() {
        let left = ("DP-1".to_string(), rect(0, 0, 1920, 1080));
        let right = ("HDMI-1".to_string(), rect(1920, 0, 1280, 1024));
        let both = [left.clone(), right.clone()];
        let on_left = Point::from((400, 300));
        let on_right = Point::from((2500, 300));

        // Nothing changed.
        assert_eq!(relocate(on_left, &both, &both), Relocation::Stay);
        // The right monitor is unplugged.
        let only_left = [left.clone()];
        assert_eq!(relocate(on_left, &both, &only_left), Relocation::Stay);
        assert_eq!(relocate(on_right, &both, &only_left), Relocation::Orphaned);
        // The left monitor is unplugged: the right one becomes the origin and
        // its windows move with it.
        let only_right = [("HDMI-1".to_string(), rect(0, 0, 1280, 1024))];
        assert_eq!(relocate(on_left, &both, &only_right), Relocation::Orphaned);
        assert_eq!(
            relocate(on_right, &both, &only_right),
            Relocation::Shift(Point::from((-1920, 0)))
        );
        // A monitor is added to the left of an existing one.
        let swapped = [
            ("DP-2".to_string(), rect(0, 0, 2560, 1440)),
            ("DP-1".to_string(), rect(2560, 0, 1920, 1080)),
        ];
        assert_eq!(
            relocate(on_left, &only_left, &swapped),
            Relocation::Shift(Point::from((2560, 0)))
        );
        // A window that was on no monitor is left alone, as is the first layout.
        assert_eq!(
            relocate(Point::from((9000, 9000)), &both, &only_left),
            Relocation::Stay
        );
        assert_eq!(relocate(on_left, &[], &both), Relocation::Stay);
    }

    #[test]
    fn switcher_steps_wrap_in_both_directions() {
        assert_eq!(step_index(0, 3, false), 1);
        assert_eq!(step_index(2, 3, false), 0);
        assert_eq!(step_index(0, 3, true), 2);
        assert_eq!(step_index(1, 3, true), 0);
        assert_eq!(step_index(0, 1, false), 0);
        assert_eq!(step_index(0, 1, true), 0);
        assert_eq!(step_index(5, 0, false), 0);
    }
}
