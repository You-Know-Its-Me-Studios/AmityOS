//! Window management policy: which windows are managed, their most-recently-used
//! order, keyboard focus, placement, and the minimized, maximized and snapped
//! states. Behaviour reference: docs/specs/shell-behaviour.md §4 and §6.

use std::cell::{Cell, RefCell};

use amity_ipc::{bounded_text, SwitcherState, WindowInfo, MAX_WINDOWS};
use smithay::{
    desktop::{layer_map_for_output, space::SpaceElement, WindowSurface},
    output::Output,
    reexports::{wayland_protocols::xdg::shell::server::xdg_toplevel, wayland_server::Resource},
    utils::{IsAlive, Logical, Point, Rectangle, Size, SERIAL_COUNTER},
    wayland::{compositor::with_states, shell::xdg::XdgToplevelSurfaceData},
};

use crate::{
    focus::KeyboardFocusTarget,
    layout::{self, Relocation, Tiling},
    shell::{ssd::HEADER_BAR_HEIGHT, FullscreenSurface, WindowElement},
    state::{AmityState, Backend},
};

/// Width of a dragged window that must stay on screen, in logical pixels.
const DRAG_GRIP: i32 = 120;

/// Compositor-side record attached to every managed window.
#[derive(Debug)]
pub struct WindowRecord {
    id: u64,
    /// Cascade slot used for the initial position.
    slot: u32,
    tiling: Cell<Tiling>,
    /// Outer rectangle (decoration included) to return to when the window
    /// stops being maximized or snapped.
    restore: Cell<Option<Rectangle<i32, Logical>>>,
    /// Where the window was when it was minimized; `Some` while minimized.
    minimized_at: Cell<Option<Point<i32, Logical>>>,
    /// False until the first content arrives and the position has been fitted.
    placed: Cell<bool>,
    /// Set while the window is fullscreen.
    fullscreen: RefCell<Option<Fullscreen>>,
}

/// Where a fullscreen window is shown and what it returns to.
#[derive(Debug)]
struct Fullscreen {
    output: Output,
    /// Outer rectangle before the window went fullscreen, if it had one.
    restore: Option<Rectangle<i32, Logical>>,
}

impl WindowElement {
    /// The management record; `None` for unmanaged windows such as X11 menus.
    pub fn record(&self) -> Option<&WindowRecord> {
        self.user_data().get::<WindowRecord>()
    }

    pub fn id(&self) -> Option<u64> {
        self.record().map(|record| record.id)
    }

    pub fn tiling(&self) -> Tiling {
        self.record()
            .map(|record| record.tiling.get())
            .unwrap_or_default()
    }

    pub fn is_minimized(&self) -> bool {
        self.record()
            .is_some_and(|record| record.minimized_at.get().is_some())
    }

    /// Geometry of the client's content, without the compositor-drawn title bar.
    pub fn content_geometry(&self) -> Rectangle<i32, Logical> {
        SpaceElement::geometry(&self.0)
    }

    pub fn is_fullscreen(&self) -> bool {
        self.record()
            .is_some_and(|record| record.fullscreen.borrow().is_some())
    }

    /// Whether the compositor draws a title bar for this window right now: it
    /// does for windows that do not draw their own frame, except in fullscreen.
    pub fn has_header(&self) -> bool {
        self.decoration_state().is_ssd && !self.is_fullscreen()
    }

    /// Height of the compositor-drawn title bar, 0 when there is none.
    pub fn header_height(&self) -> i32 {
        if self.has_header() {
            HEADER_BAR_HEIGHT
        } else {
            0
        }
    }

    pub fn title(&self) -> String {
        match self.0.underlying_surface() {
            WindowSurface::Wayland(toplevel) => with_states(toplevel.wl_surface(), |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .and_then(|data| data.lock().ok()?.title.clone())
            })
            .unwrap_or_default(),
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(surface) => surface.title(),
        }
    }

    /// Application identity: the Wayland `app_id` or the X11 window class.
    pub fn app_id(&self) -> String {
        match self.0.underlying_surface() {
            WindowSurface::Wayland(toplevel) => with_states(toplevel.wl_surface(), |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .and_then(|data| data.lock().ok()?.app_id.clone())
            })
            .unwrap_or_default(),
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(surface) => {
                let class = surface.class();
                if class.is_empty() {
                    surface.instance()
                } else {
                    class
                }
            }
        }
    }
}

/// Window-manager state that is not tied to a single window.
#[derive(Debug, Default)]
pub struct Wm {
    next_id: u64,
    /// Managed windows, most recently used first; includes minimized windows.
    mru: Vec<WindowElement>,
    /// The window that has (or last had) keyboard focus. Shell surfaces can
    /// hold the keyboard for a while without changing it.
    active: Option<WindowElement>,
    cascade: u32,
    switcher: Option<SwitcherState>,
}

impl Wm {
    pub fn switcher(&self) -> Option<&SwitcherState> {
        self.switcher.as_ref()
    }

    pub fn windows(&self) -> impl Iterator<Item = &WindowElement> {
        self.mru.iter()
    }

    fn touch(&mut self, window: &WindowElement) {
        self.mru.retain(|other| other != window);
        self.mru.insert(0, window.clone());
    }
}

impl<BackendData: Backend> AmityState<BackendData> {
    /// Usable area of an output in global coordinates: the output without the
    /// zones shell surfaces reserve for themselves, such as the taskbar.
    pub fn work_area(&self, output: &Output) -> Rectangle<i32, Logical> {
        let geometry = self.space.output_geometry(output).unwrap_or_default();
        let zone = layer_map_for_output(output).non_exclusive_zone();
        Rectangle::new(geometry.loc + zone.loc, zone.size)
    }

    fn fallback_area(&self) -> Rectangle<i32, Logical> {
        Rectangle::from_size(Size::from((1280, 800)))
    }

    /// Work area of the output under the pointer, where new windows open.
    fn pointer_work_area(&self) -> Rectangle<i32, Logical> {
        self.space
            .output_under(self.pointer.current_location())
            .next()
            .or_else(|| self.space.outputs().next())
            .map(|output| self.work_area(output))
            .unwrap_or_else(|| self.fallback_area())
    }

    /// The output a window is on, judged by its centre.
    pub fn output_of(&self, window: &WindowElement) -> Option<Output> {
        let by_centre = self.space.element_geometry(window).and_then(|geometry| {
            let centre = Point::<f64, Logical>::from((
                f64::from(geometry.loc.x) + f64::from(geometry.size.w) / 2.0,
                f64::from(geometry.loc.y) + f64::from(geometry.size.h) / 2.0,
            ));
            self.space.output_under(centre).next().cloned()
        });
        by_centre
            .or_else(|| self.space.outputs_for_element(window).first().cloned())
            .or_else(|| self.space.outputs().next().cloned())
    }

    fn window_work_area(&self, window: &WindowElement) -> Rectangle<i32, Logical> {
        self.output_of(window)
            .map(|output| self.work_area(&output))
            .unwrap_or_else(|| self.fallback_area())
    }

    pub fn window_by_id(&self, id: u64) -> Option<WindowElement> {
        self.wm.mru.iter().find(|window| window.id() == Some(id)).cloned()
    }

    /// The window that has keyboard focus, or had it before a shell surface
    /// borrowed the keyboard.
    pub fn active_window(&self) -> Option<WindowElement> {
        self.wm.active.clone().filter(|window| window.alive())
    }

    /// The outer rectangle the compositor last asked a window to take. Unlike
    /// the geometry of what the client has drawn so far, this is already right
    /// when a request such as "restore" has been sent but not yet answered.
    fn intended_geometry(&self, window: &WindowElement) -> Option<Rectangle<i32, Logical>> {
        let location = self.space.element_location(window)?;
        let drawn = window.geometry().size;
        let header = window.header_height();
        let size = match window.0.underlying_surface() {
            WindowSurface::Wayland(toplevel) => toplevel
                .with_pending_state(|state| state.size)
                .map(|content| Size::from((content.w, content.h + header)))
                .unwrap_or(drawn),
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(surface) => {
                let content = surface.geometry().size;
                Size::from((content.w, content.h + header))
            }
        };
        (size.w > 0 && size.h > 0).then(|| Rectangle::new(location, size))
    }

    /// `wanted` if a window placed there would be on a monitor, otherwise a
    /// place for it where new windows open. The monitor a minimized window
    /// was on may be gone by the time it is restored.
    fn visible_location(&self, window: &WindowElement, wanted: Point<i32, Logical>) -> Point<i32, Logical> {
        let size = window.geometry().size;
        let centre = Point::<f64, Logical>::from((
            f64::from(wanted.x) + f64::from(size.w) / 2.0,
            f64::from(wanted.y) + f64::from(size.h) / 2.0,
        ));
        if self.space.output_under(centre).next().is_some() {
            wanted
        } else {
            layout::fit_into(wanted, size, self.pointer_work_area())
        }
    }

    /// Starts managing a new top-level window and maps it. A window whose size
    /// is not known yet is positioned again when its first content arrives.
    pub fn manage_window(&mut self, window: &WindowElement) {
        if window.record().is_some() {
            return;
        }
        self.wm.next_id += 1;
        let id = self.wm.next_id;
        let slot = self.wm.cascade;
        self.wm.cascade = self.wm.cascade.wrapping_add(1);
        window.user_data().insert_if_missing(|| WindowRecord {
            id,
            slot,
            tiling: Cell::new(Tiling::Floating),
            restore: Cell::new(None),
            minimized_at: Cell::new(None),
            placed: Cell::new(false),
            fullscreen: RefCell::new(None),
        });

        let area = self.pointer_work_area();
        #[allow(irrefutable_let_patterns)]
        if let Some(toplevel) = window.0.toplevel() {
            toplevel.with_pending_state(|state| {
                state.bounds = Some(area.size);
            });
        }
        let location = area.loc + layout::cascade_offset(slot, area.size);
        self.space.map_element(window.clone(), location, false);
        // Behind the other windows in the use order until it receives focus.
        self.wm.mru.push(window.clone());

        let size = window.geometry().size;
        if size.w > 0 && size.h > 0 {
            self.finish_placement(window);
        }
    }

    /// Called on every commit of a managed window's main surface.
    pub fn window_committed(&mut self, window: &WindowElement) {
        let Some(record) = window.record() else {
            return;
        };
        if record.placed.get() {
            return;
        }
        let size = window.geometry().size;
        if size.w > 0 && size.h > 0 {
            self.finish_placement(window);
        }
    }

    /// Fits the initial position to the window's real size and focuses it.
    fn finish_placement(&mut self, window: &WindowElement) {
        let Some(record) = window.record() else {
            return;
        };
        if record.placed.replace(true) {
            return;
        }
        if record.tiling.get() == Tiling::Floating && !window.is_fullscreen() {
            let area = self.window_work_area(window);
            let asked = window.geometry().size;
            let size = layout::fit_new_size(asked, area.size);
            let location = layout::place_new(record.slot, size, area);
            if size == asked {
                self.space.map_element(window.clone(), location, false);
                self.sync_x11_geometry(window);
            } else {
                // It opened larger than the work area: ask it for a size that fits.
                self.configure_window(window, Rectangle::new(location, size), Tiling::Floating);
            }
        }
        self.focus_window(window);
    }

    /// Stops managing a window that was closed or withdrawn.
    pub fn unmanage_window(&mut self, window: &WindowElement) {
        let was_managed = self.wm.mru.iter().any(|other| other == window);
        self.wm.mru.retain(|other| other != window);
        if let Some(id) = window.id() {
            self.switcher_remove(id);
        }
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        for output in outputs {
            let cleared = output
                .user_data()
                .get::<FullscreenSurface>()
                .is_some_and(|slot| slot.get().as_ref() == Some(window) && slot.clear().is_some());
            if cleared {
                self.backend_data.reset_buffers(&output);
            }
        }
        let was_active = self.wm.active.as_ref() == Some(window);
        if was_active {
            self.wm.active = None;
        }
        if was_managed && was_active {
            self.focus_top_window();
        }
    }

    /// Drops windows whose client went away without a destroy notification.
    pub fn prune_windows(&mut self) {
        let dead: Vec<WindowElement> = self
            .wm
            .mru
            .iter()
            .filter(|window| !window.alive())
            .cloned()
            .collect();
        for window in dead {
            self.unmanage_window(&window);
        }
    }

    /// Sends state changes (activated, maximized, size) that are still pending.
    fn flush_window_states(&self) {
        for window in self.space.elements() {
            #[allow(irrefutable_let_patterns)]
            if let Some(toplevel) = window.0.toplevel() {
                if toplevel.is_initial_configure_sent() {
                    toplevel.send_pending_configure();
                }
            }
        }
    }

    /// Gives a window keyboard focus: restores it if minimized, raises it and
    /// moves it to the front of the use order.
    pub fn focus_window(&mut self, window: &WindowElement) {
        if !window.alive() {
            return;
        }
        if let Some(record) = window.record() {
            if let Some(location) = record.minimized_at.take() {
                #[cfg(feature = "xwayland")]
                if let Some(surface) = window.0.x11_surface() {
                    let _ = surface.set_mapped(true);
                }
                let location = self.visible_location(window, location);
                self.space.map_element(window.clone(), location, false);
                self.sync_x11_geometry(window);
                // The monitor may have changed while the window was minimized.
                self.rearrange(window);
            }
            self.wm.touch(window);
        }
        self.space.raise_element(window, true);
        #[cfg(feature = "xwayland")]
        if let Some(surface) = window.0.x11_surface() {
            if let Some(xwm) = self.xwm.as_mut() {
                let _ = xwm.raise_window(surface);
            }
        }
        self.wm.active = Some(window.clone());
        self.flush_window_states();
        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.set_focus(self, Some(window.clone().into()), SERIAL_COUNTER.next_serial());
    }

    /// Removes keyboard focus from windows (a click on the empty desktop).
    pub fn clear_window_focus(&mut self) {
        if let Some(previous) = self.wm.active.take() {
            previous.set_activate(false);
            self.flush_window_states();
        }
        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.set_focus(
            self,
            Option::<KeyboardFocusTarget>::None,
            SERIAL_COUNTER.next_serial(),
        );
    }

    /// Focuses the most recently used window that is not minimized, if any.
    pub fn focus_top_window(&mut self) {
        let next = self
            .wm
            .mru
            .iter()
            .find(|window| window.alive() && !window.is_minimized())
            .cloned();
        match next {
            Some(window) => self.focus_window(&window),
            None => self.clear_window_focus(),
        }
    }

    /// Returns the keyboard to the active window after a shell surface that
    /// held it went away.
    pub fn restore_window_focus(&mut self) {
        match self.active_window().filter(|window| !window.is_minimized()) {
            Some(window) => self.focus_window(&window),
            None => self.focus_top_window(),
        }
    }

    pub fn minimize_window(&mut self, window: &WindowElement) {
        let Some(record) = window.record() else {
            return;
        };
        if record.minimized_at.get().is_some() {
            return;
        }
        let Some(location) = self.space.element_location(window) else {
            return;
        };
        record.minimized_at.set(Some(location));
        window.set_activate(false);
        #[allow(irrefutable_let_patterns)]
        if let Some(toplevel) = window.0.toplevel() {
            if toplevel.is_initial_configure_sent() {
                toplevel.send_pending_configure();
            }
        }
        self.space.unmap_elem(window);
        #[cfg(feature = "xwayland")]
        if let Some(surface) = window.0.x11_surface() {
            let _ = surface.set_mapped(false);
        }
        if self.wm.active.as_ref() == Some(window) {
            self.wm.active = None;
            self.focus_top_window();
        }
    }

    pub fn close_window(&mut self, window: &WindowElement) {
        match window.0.underlying_surface() {
            WindowSurface::Wayland(toplevel) => toplevel.send_close(),
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(surface) => {
                let _ = surface.close();
            }
        }
    }

    /// Tells an X11 client where its window is. X11 clients position their
    /// menus and dialogs from this, so it must follow every move.
    pub fn sync_x11_geometry(&self, window: &WindowElement) {
        #[cfg(feature = "xwayland")]
        if let Some(surface) = window.0.x11_surface() {
            if surface.is_override_redirect() {
                return;
            }
            if let Some(location) = self.space.element_location(window) {
                let content = Point::from((location.x, location.y + window.header_height()));
                let _ = surface.configure(Rectangle::new(content, surface.geometry().size));
            }
        }
        #[cfg(not(feature = "xwayland"))]
        let _ = window;
    }

    /// Asks the client for the outer rectangle `outer` (title bar included)
    /// in the given arrangement and moves the window there.
    fn configure_window(&mut self, window: &WindowElement, outer: Rectangle<i32, Logical>, tiling: Tiling) {
        let header = window.header_height();
        let content = Size::<i32, Logical>::from((outer.size.w.max(1), (outer.size.h - header).max(1)));
        match window.0.underlying_surface() {
            WindowSurface::Wayland(toplevel) => {
                use xdg_toplevel::State;
                toplevel.with_pending_state(|state| {
                    for tiled in [
                        State::Maximized,
                        State::TiledLeft,
                        State::TiledRight,
                        State::TiledTop,
                        State::TiledBottom,
                    ] {
                        state.states.unset(tiled);
                    }
                    match tiling {
                        Tiling::Floating => {}
                        Tiling::Maximized => {
                            state.states.set(State::Maximized);
                        }
                        Tiling::Left => {
                            for tiled in [State::TiledLeft, State::TiledTop, State::TiledBottom] {
                                state.states.set(tiled);
                            }
                        }
                        Tiling::Right => {
                            for tiled in [State::TiledRight, State::TiledTop, State::TiledBottom] {
                                state.states.set(tiled);
                            }
                        }
                    }
                    state.size = Some(content);
                });
                if toplevel.is_initial_configure_sent() {
                    toplevel.send_pending_configure();
                }
            }
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(surface) => {
                let _ = surface.set_maximized(tiling == Tiling::Maximized);
                let location = Point::from((outer.loc.x, outer.loc.y + header));
                let _ = surface.configure(Rectangle::new(location, content));
            }
        }
        if !window.is_minimized() {
            self.space.map_element(window.clone(), outer.loc, false);
        }
    }

    /// Maximizes, snaps or restores a window.
    pub fn set_tiling(&mut self, window: &WindowElement, tiling: Tiling) {
        let Some(record) = window.record() else {
            return;
        };
        // A window that started fullscreen has never had a size of its own to return to.
        let mut sizeless = false;
        if window.is_fullscreen() {
            sizeless = record
                .fullscreen
                .borrow()
                .as_ref()
                .is_some_and(|fullscreen| fullscreen.restore.is_none());
            // Leaving fullscreen comes first; the window then takes the new arrangement.
            self.unset_fullscreen(window);
        }
        let current = record.tiling.get();
        if current == tiling {
            return;
        }
        let area = self.window_work_area(window);
        if current == Tiling::Floating {
            // Remember where to come back to, if the window has a size yet.
            let restore = if sizeless {
                None
            } else {
                self.intended_geometry(window)
            };
            record.restore.set(restore);
        }
        record.tiling.set(tiling);
        match layout::tiled_rect(tiling, area) {
            Some(target) => self.configure_window(window, target, tiling),
            None => match record.restore.take() {
                Some(restore) => self.configure_window(window, layout::fit_rect(restore, area), tiling),
                None => self.release_tiling(window, area),
            },
        }
    }

    /// Returns a window that was tiled before it ever had a floating size to
    /// floating, letting the client choose its size.
    fn release_tiling(&mut self, window: &WindowElement, area: Rectangle<i32, Logical>) {
        let Some(record) = window.record() else {
            return;
        };
        match window.0.underlying_surface() {
            WindowSurface::Wayland(toplevel) => {
                use xdg_toplevel::State;
                toplevel.with_pending_state(|state| {
                    for tiled in [
                        State::Maximized,
                        State::TiledLeft,
                        State::TiledRight,
                        State::TiledTop,
                        State::TiledBottom,
                    ] {
                        state.states.unset(tiled);
                    }
                    state.size = None;
                });
                if toplevel.is_initial_configure_sent() {
                    toplevel.send_pending_configure();
                }
            }
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(surface) => {
                let _ = surface.set_maximized(false);
            }
        }
        if !window.is_minimized() {
            let location = area.loc + layout::cascade_offset(record.slot, area.size);
            self.space.map_element(window.clone(), location, false);
            self.sync_x11_geometry(window);
        }
    }

    pub fn toggle_maximize(&mut self, window: &WindowElement) {
        let target = if window.tiling() == Tiling::Maximized {
            Tiling::Floating
        } else {
            Tiling::Maximized
        };
        self.set_tiling(window, target);
    }

    /// Marks a tiled window as floating without changing its geometry, for an
    /// interactive resize of a maximized or snapped window.
    pub fn float_in_place(&mut self, window: &WindowElement) {
        let Some(record) = window.record() else {
            return;
        };
        if record.tiling.replace(Tiling::Floating) == Tiling::Floating {
            return;
        }
        record.restore.set(None);
        match window.0.underlying_surface() {
            WindowSurface::Wayland(toplevel) => {
                use xdg_toplevel::State;
                toplevel.with_pending_state(|state| {
                    for tiled in [
                        State::Maximized,
                        State::TiledLeft,
                        State::TiledRight,
                        State::TiledTop,
                        State::TiledBottom,
                    ] {
                        state.states.unset(tiled);
                    }
                });
            }
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(surface) => {
                let _ = surface.set_maximized(false);
            }
        }
    }

    /// A tiled window is being dragged by its title bar: give it back its
    /// floating size under the pointer. Returns the window's new location as
    /// it would be with the pointer still at `press`.
    pub fn untile_for_drag(
        &mut self,
        window: &WindowElement,
        press: Point<f64, Logical>,
    ) -> Point<i32, Logical> {
        let current = self
            .space
            .element_geometry(window)
            .unwrap_or_else(|| self.fallback_area());
        let Some(record) = window.record() else {
            return current.loc;
        };
        let size = record
            .restore
            .take()
            .map(|restore| restore.size)
            .unwrap_or_else(|| Size::from((current.size.w * 2 / 3, current.size.h * 2 / 3)));
        let size = Size::from((
            size.w.max(layout::MINIMUM_WINDOW.0),
            size.h.max(layout::MINIMUM_WINDOW.1),
        ));
        let fraction =
            ((press.x - f64::from(current.loc.x)) / f64::from(current.size.w.max(1))).clamp(0.0, 1.0);
        let location = Point::from((
            (press.x - fraction * f64::from(size.w)).round() as i32,
            current.loc.y,
        ));
        record.tiling.set(Tiling::Floating);
        self.configure_window(window, Rectangle::new(location, size), Tiling::Floating);
        location
    }

    /// Limits where a dragged window can go so that its title bar stays reachable.
    pub fn clamp_dragged(
        &self,
        window: &WindowElement,
        location: Point<i32, Logical>,
        pointer: Point<f64, Logical>,
    ) -> Point<i32, Logical> {
        let area = self
            .space
            .output_under(pointer)
            .next()
            .map(|output| self.work_area(output))
            .unwrap_or_else(|| self.window_work_area(window));
        layout::clamp_dragged(
            location,
            window.geometry().size,
            area,
            HEADER_BAR_HEIGHT,
            DRAG_GRIP,
        )
    }

    /// Re-fits the windows of an output after its work area changed (the
    /// taskbar appeared, the mode changed).
    pub fn reflow_output(&mut self, output: &Output) {
        let area = self.work_area(output);
        let windows: Vec<WindowElement> = self.space.elements().cloned().collect();
        for window in windows {
            let Some(record) = window.record() else {
                continue;
            };
            if !record.placed.get() {
                continue;
            }
            let fullscreen_here = record
                .fullscreen
                .borrow()
                .as_ref()
                .is_some_and(|fullscreen| &fullscreen.output == output);
            if fullscreen_here {
                // The monitor's mode may have changed.
                self.apply_fullscreen(&window, output);
                continue;
            }
            if window.is_fullscreen() || self.output_of(&window).as_ref() != Some(output) {
                continue;
            }
            let tiling = record.tiling.get();
            match layout::tiled_rect(tiling, area) {
                Some(target) => self.configure_window(&window, target, tiling),
                None => {
                    let Some(location) = self.space.element_location(&window) else {
                        continue;
                    };
                    let fitted = layout::clamp_dragged(
                        location,
                        window.geometry().size,
                        area,
                        HEADER_BAR_HEIGHT,
                        DRAG_GRIP,
                    );
                    if fitted != location {
                        self.space.map_element(window.clone(), fitted, false);
                        self.sync_x11_geometry(&window);
                    }
                }
            }
        }
    }

    /// The managed windows in use order, and the id of the active one.
    ///
    /// Titles and application ids come from applications; they are bounded
    /// here, and so is the length of the list, so that no application can make
    /// the message to the shell arbitrarily large.
    pub fn window_list(&self) -> (Vec<WindowInfo>, Option<u64>) {
        let windows = self
            .wm
            .mru
            .iter()
            .filter(|window| window.alive())
            .take(MAX_WINDOWS)
            .filter_map(|window| {
                let record = window.record()?;
                Some(WindowInfo {
                    id: record.id,
                    title: bounded_text(&window.title()),
                    app_id: bounded_text(&window.app_id()),
                    minimized: record.minimized_at.get().is_some(),
                    maximized: record.tiling.get() == Tiling::Maximized,
                    fullscreen: window.is_fullscreen(),
                    output: if record.minimized_at.get().is_some() {
                        None
                    } else {
                        self.output_of(window).map(|output| output.name())
                    },
                })
            })
            .collect();
        (windows, self.active_window().and_then(|window| window.id()))
    }

    /// Re-applies a window's arrangement (fullscreen, maximized, snapped) to
    /// the monitor it is on now.
    fn rearrange(&mut self, window: &WindowElement) {
        let Some(record) = window.record() else {
            return;
        };
        let fullscreen_output = record
            .fullscreen
            .borrow()
            .as_ref()
            .map(|fullscreen| fullscreen.output.clone());
        if let Some(output) = fullscreen_output {
            self.apply_fullscreen(window, &output);
            return;
        }
        let tiling = record.tiling.get();
        if let Some(target) = layout::tiled_rect(tiling, self.window_work_area(window)) {
            self.configure_window(window, target, tiling);
        }
    }

    /// The fullscreen window of a monitor, whether or not it covers it right now.
    fn fullscreen_on(&self, output: &Output) -> Option<WindowElement> {
        output
            .user_data()
            .get::<FullscreenSurface>()
            .and_then(|slot| slot.get())
    }

    /// Makes a window fullscreen on `requested`, or on the monitor it is on.
    pub fn set_fullscreen(&mut self, window: &WindowElement, requested: Option<Output>) {
        let Some(record) = window.record() else {
            return;
        };
        let requested = requested.filter(|output| self.space.outputs().any(|other| other == output));
        let Some(output) = requested.or_else(|| self.output_of(window)) else {
            return;
        };
        // One fullscreen window per monitor.
        if let Some(other) = self.fullscreen_on(&output).filter(|other| other != window) {
            self.unset_fullscreen(&other);
        }
        let previous = record
            .fullscreen
            .borrow()
            .as_ref()
            .map(|fullscreen| fullscreen.output.clone());
        match previous {
            Some(previous) if previous == output => return,
            Some(previous) => {
                // Moving to another monitor.
                if let Some(slot) = previous.user_data().get::<FullscreenSurface>() {
                    slot.clear();
                }
                self.backend_data.reset_buffers(&previous);
                if let Some(fullscreen) = record.fullscreen.borrow_mut().as_mut() {
                    fullscreen.output = output.clone();
                }
            }
            None => {
                let restore = if record.placed.get() && !window.is_minimized() {
                    self.intended_geometry(window)
                } else {
                    None
                };
                *record.fullscreen.borrow_mut() = Some(Fullscreen {
                    output: output.clone(),
                    restore,
                });
            }
        }
        self.apply_fullscreen(window, &output);
    }

    /// Gives a fullscreen window the geometry of its monitor.
    fn apply_fullscreen(&mut self, window: &WindowElement, output: &Output) {
        let geometry = self
            .space
            .output_geometry(output)
            .unwrap_or_else(|| self.fallback_area());
        match window.0.underlying_surface() {
            WindowSurface::Wayland(toplevel) => {
                let wl_output = self
                    .display_handle
                    .get_client(toplevel.wl_surface().id())
                    .ok()
                    .and_then(|client| output.client_outputs(&client).into_iter().next());
                toplevel.with_pending_state(|state| {
                    state.states.set(xdg_toplevel::State::Fullscreen);
                    state.size = Some(geometry.size);
                    state.fullscreen_output = wl_output;
                });
                if toplevel.is_initial_configure_sent() {
                    toplevel.send_pending_configure();
                }
            }
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(surface) => {
                let _ = surface.set_fullscreen(true);
                let _ = surface.configure(geometry);
            }
        }
        output.user_data().insert_if_missing(FullscreenSurface::default);
        if let Some(slot) = output.user_data().get::<FullscreenSurface>() {
            slot.set(window.clone());
        }
        if !window.is_minimized() {
            self.space.map_element(window.clone(), geometry.loc, false);
        }
        self.backend_data.reset_buffers(output);
    }

    /// Takes a window out of fullscreen, back to what it was before.
    pub fn unset_fullscreen(&mut self, window: &WindowElement) {
        let Some(record) = window.record() else {
            return;
        };
        let Some(fullscreen) = record.fullscreen.borrow_mut().take() else {
            return;
        };
        if let Some(slot) = fullscreen.output.user_data().get::<FullscreenSurface>() {
            if slot.get().as_ref() == Some(window) {
                slot.clear();
            }
        }
        self.backend_data.reset_buffers(&fullscreen.output);
        match window.0.underlying_surface() {
            WindowSurface::Wayland(toplevel) => {
                toplevel.with_pending_state(|state| {
                    state.states.unset(xdg_toplevel::State::Fullscreen);
                    state.fullscreen_output = None;
                });
            }
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(surface) => {
                let _ = surface.set_fullscreen(false);
            }
        }
        // From here on the window has its title bar again, if it has one.
        let area = self.window_work_area(window);
        let tiling = record.tiling.get();
        let target = layout::tiled_rect(tiling, area)
            .or_else(|| fullscreen.restore.map(|restore| layout::fit_rect(restore, area)));
        match target {
            Some(target) => self.configure_window(window, target, tiling),
            // It started fullscreen and never had a size of its own: the client chooses.
            None => self.release_tiling(window, area),
        }
    }

    /// A fullscreen window covers its monitor, hiding the taskbar and the
    /// other windows, only while it is the active window and no shell overlay
    /// (launcher, switcher) wants the screen. Otherwise the monitor is drawn
    /// normally, with the fullscreen window as one window among the others.
    pub fn sync_fullscreen(&mut self) {
        let overlay = self.wm.switcher.is_some() || self.exclusive_keyboard_layer().is_some();
        let active = self.active_window();
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        for output in outputs {
            let Some(slot) = output.user_data().get::<FullscreenSurface>() else {
                continue;
            };
            let covering = !overlay
                && slot
                    .get()
                    .is_some_and(|window| Some(&window) == active.as_ref() && !window.is_minimized());
            if slot.set_covering(covering) {
                self.backend_data.reset_buffers(&output);
            }
        }
    }

    /// Follows a change of the monitor layout: windows keep their place on a
    /// monitor that moved, and go to a remaining monitor when theirs is gone.
    /// `before` is the layout the windows were arranged for.
    pub fn outputs_changed(&mut self, before: &[(String, Rectangle<i32, Logical>)]) {
        let after = self.output_layout.clone();
        let fallback = self.space.outputs().next().cloned();
        let windows: Vec<WindowElement> = self.wm.mru.clone();
        for window in windows {
            let Some(record) = window.record() else {
                continue;
            };
            let minimized = record.minimized_at.get();
            let Some(location) = minimized.or_else(|| self.space.element_location(&window)) else {
                continue;
            };
            let size = window.geometry().size;
            let centre = Point::from((location.x + size.w / 2, location.y + size.h / 2));
            // A fullscreen window belongs to its monitor even while it is minimized
            // somewhere else in the layout.
            let fullscreen_output_gone =
                record.fullscreen.borrow().as_ref().is_some_and(|fullscreen| {
                    !self.space.outputs().any(|output| output == &fullscreen.output)
                });
            let relocation = if fullscreen_output_gone {
                Relocation::Orphaned
            } else {
                layout::relocate(centre, before, &after)
            };
            match relocation {
                Relocation::Stay => {}
                Relocation::Shift(delta) => {
                    if minimized.is_some() {
                        record.minimized_at.set(Some(location + delta));
                    } else {
                        self.space.map_element(window.clone(), location + delta, false);
                        self.sync_x11_geometry(&window);
                    }
                    let shifted = |rect: Rectangle<i32, Logical>| Rectangle::new(rect.loc + delta, rect.size);
                    record.restore.set(record.restore.get().map(shifted));
                    if let Some(fullscreen) = record.fullscreen.borrow_mut().as_mut() {
                        fullscreen.restore = fullscreen.restore.map(shifted);
                    }
                }
                Relocation::Orphaned => {
                    if let Some(fallback) = fallback.as_ref() {
                        self.rehome(&window, location, fallback);
                    }
                }
            }
        }
    }

    /// Moves a window whose monitor is gone to `output`, keeping its
    /// arrangement: a maximized window is maximized there, and so on.
    fn rehome(&mut self, window: &WindowElement, location: Point<i32, Logical>, output: &Output) {
        let Some(record) = window.record() else {
            return;
        };
        let area = self.work_area(output);
        let minimized = record.minimized_at.get().is_some();
        record.restore.set(
            record
                .restore
                .get()
                .map(|restore| layout::fit_rect(restore, area)),
        );

        if window.is_fullscreen() {
            // One fullscreen window per monitor: the one already there gives way.
            if let Some(other) = self.fullscreen_on(output).filter(|other| other != window) {
                self.unset_fullscreen(&other);
            }
            if let Some(fullscreen) = record.fullscreen.borrow_mut().as_mut() {
                fullscreen.output = output.clone();
                fullscreen.restore = fullscreen.restore.map(|restore| layout::fit_rect(restore, area));
            }
            if minimized {
                let origin = self.space.output_geometry(output).unwrap_or(area).loc;
                record.minimized_at.set(Some(origin));
            }
            // Sets the size for the new monitor; maps the window unless it is minimized.
            self.apply_fullscreen(window, output);
            return;
        }

        let tiling = record.tiling.get();
        let size = window.geometry().size;
        let target = layout::tiled_rect(tiling, area)
            .unwrap_or_else(|| layout::fit_rect(Rectangle::new(location, size), area));
        // A window that has not drawn anything yet has no size to change.
        let has_size = size.w > 0 && size.h > 0;
        if minimized {
            // The size is applied when the window is restored.
            record.minimized_at.set(Some(target.loc));
        } else if tiling != Tiling::Floating || (has_size && target.size != size) {
            self.configure_window(window, target, tiling);
        } else {
            self.space.map_element(window.clone(), target.loc, false);
            self.sync_x11_geometry(window);
        }
    }

    /// Opens the switcher, or moves its selection when it is already open.
    pub fn switcher_step(&mut self, reverse: bool) {
        if let Some(switcher) = self.wm.switcher.as_mut() {
            switcher.selected = layout::step_index(switcher.selected, switcher.candidates.len(), reverse);
            return;
        }
        let candidates: Vec<u64> = self
            .wm
            .mru
            .iter()
            .filter(|window| window.alive())
            .filter_map(|window| window.id())
            .take(MAX_WINDOWS)
            .collect();
        if candidates.is_empty() {
            return;
        }
        let active = self.active_window().and_then(|window| window.id());
        let selected = match active.and_then(|id| candidates.iter().position(|&candidate| candidate == id)) {
            Some(index) => layout::step_index(index, candidates.len(), reverse),
            None => 0,
        };
        self.wm.switcher = Some(SwitcherState { candidates, selected });
    }

    /// Closes the switcher and focuses the selected window.
    pub fn switcher_commit(&mut self) {
        let Some(switcher) = self.wm.switcher.take() else {
            return;
        };
        let selected = switcher.candidates.get(switcher.selected).copied();
        if let Some(window) = selected.and_then(|id| self.window_by_id(id)) {
            self.focus_window(&window);
        }
    }

    /// Closes the switcher without changing anything.
    pub fn switcher_cancel(&mut self) {
        self.wm.switcher = None;
    }

    fn switcher_remove(&mut self, id: u64) {
        let Some(switcher) = self.wm.switcher.as_mut() else {
            return;
        };
        let Some(index) = switcher.candidates.iter().position(|&candidate| candidate == id) else {
            return;
        };
        switcher.candidates.remove(index);
        if switcher.candidates.is_empty() {
            self.wm.switcher = None;
        } else {
            if index < switcher.selected {
                switcher.selected -= 1;
            }
            switcher.selected = switcher.selected.min(switcher.candidates.len() - 1);
        }
    }
}
