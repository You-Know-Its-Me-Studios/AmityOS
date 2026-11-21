use std::{cell::RefCell, os::unix::io::OwnedFd};

use smithay::{
    desktop::Window,
    input::pointer::Focus,
    utils::{Logical, Rectangle, SERIAL_COUNTER},
    wayland::{
        compositor::with_states,
        selection::{
            data_device::{
                clear_data_device_selection, current_data_device_selection_userdata,
                request_data_device_client_selection, set_data_device_selection,
            },
            primary_selection::{
                clear_primary_selection, current_primary_selection_userdata,
                request_primary_client_selection, set_primary_selection,
            },
            SelectionTarget,
        },
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        xwm::{Reorder, ResizeEdge as X11ResizeEdge, XwmId},
        X11Surface, X11Wm, XwmHandler,
    },
};
use tracing::{error, trace};

use crate::{focus::KeyboardFocusTarget, state::Backend, AmityState};

use super::{
    PointerMoveSurfaceGrab, PointerResizeSurfaceGrab, ResizeData, ResizeState, SurfaceData,
    TouchMoveSurfaceGrab, WindowElement,
};
use crate::layout::Tiling;

impl<BackendData: Backend> XWaylandShellHandler for AmityState<BackendData> {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

impl<BackendData: Backend> XwmHandler for AmityState<BackendData> {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().unwrap()
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Err(err) = window.set_mapped(true) {
            error!(?err, "could not map an X11 window");
            return;
        }
        let decorated = window.is_decorated();
        let window = WindowElement(Window::new_x11_window(window));
        // Windows that do not draw their own frame get the compositor's title bar.
        window.set_ssd(!decorated);
        self.manage_window(&window);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let location = window.geometry().loc;
        let window = WindowElement(Window::new_x11_window(window));
        self.space.map_element(window, location, true);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elem) = self.x11_element(&window) {
            self.space.unmap_elem(&elem);
            self.unmanage_window(&elem);
        }
        if !window.is_override_redirect() {
            let _ = window.set_mapped(false);
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elem) = self.x11_element(&window) {
            self.space.unmap_elem(&elem);
            self.unmanage_window(&elem);
        }
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _x: Option<i32>,
        _y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        // we just set the new size, but don't let windows move themselves around freely
        let mut geo = window.geometry();
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }
        let _ = window.configure(geo);
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        let Some(elem) = self.x11_element(&window) else {
            return;
        };
        // Managed windows are positioned by the compositor. Only menus and
        // other override-redirect windows place themselves.
        if elem.record().is_some() {
            return;
        }
        self.space.map_element(elem, geometry.loc, false);
        // TODO: We don't properly handle the order of override-redirect windows here,
        //       they are always mapped top and then never reordered.
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elem) = self.x11_element(&window) {
            self.set_tiling(&elem, Tiling::Maximized);
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elem) = self.x11_element(&window) {
            if elem.tiling() == Tiling::Maximized {
                self.set_tiling(&elem, Tiling::Floating);
            }
        }
    }

    fn minimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elem) = self.x11_element(&window) {
            self.minimize_window(&elem);
        }
    }

    fn unminimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elem) = self.x11_element(&window) {
            self.focus_window(&elem);
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elem) = self.x11_element(&window) {
            self.set_fullscreen(&elem, None);
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elem) = self.x11_element(&window) {
            self.unset_fullscreen(&elem);
        }
    }

    fn resize_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32, edges: X11ResizeEdge) {
        let Some(start_data) = self.pointer.grab_start_data() else {
            return;
        };
        let Some(element) = self.x11_element(&window) else {
            return;
        };
        if element.is_fullscreen() {
            return;
        }
        self.float_in_place(&element);
        let element = &element;

        let geometry = element.content_geometry();
        let Some(loc) = self.space.element_location(element) else {
            return;
        };
        let (initial_window_location, initial_window_size) = (loc, geometry.size);

        let Some(surface) = element.wl_surface() else {
            return;
        };
        with_states(&surface, move |states| {
            states
                .data_map
                .get::<RefCell<SurfaceData>>()
                .unwrap()
                .borrow_mut()
                .resize_state = ResizeState::Resizing(ResizeData {
                edges: edges.into(),
                initial_window_location,
                initial_window_size,
            });
        });

        let grab = PointerResizeSurfaceGrab {
            start_data,
            window: element.clone(),
            edges: edges.into(),
            initial_window_location,
            initial_window_size,
            last_window_size: initial_window_size,
        };

        let pointer = self.pointer.clone();
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32) {
        self.move_request_x11(&window)
    }

    fn allow_selection_access(&mut self, xwm: XwmId, _selection: SelectionTarget) -> bool {
        if let Some(keyboard) = self.seat.get_keyboard() {
            // check that an X11 window is focused
            if let Some(KeyboardFocusTarget::Window(w)) = keyboard.current_focus() {
                if let Some(surface) = w.x11_surface() {
                    if surface.xwm_id().unwrap() == xwm {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn send_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_type: String, fd: OwnedFd) {
        match selection {
            SelectionTarget::Clipboard => {
                if let Err(err) = request_data_device_client_selection(&self.seat, mime_type, fd) {
                    error!(?err, "Failed to request current wayland clipboard for Xwayland",);
                }
            }
            SelectionTarget::Primary => {
                if let Err(err) = request_primary_client_selection(&self.seat, mime_type, fd) {
                    error!(
                        ?err,
                        "Failed to request current wayland primary selection for Xwayland",
                    );
                }
            }
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        trace!(?selection, ?mime_types, "Got Selection from X11",);
        // TODO check, that focused windows is X11 window before doing this
        match selection {
            SelectionTarget::Clipboard => {
                set_data_device_selection(&self.display_handle, &self.seat, mime_types, ())
            }
            SelectionTarget::Primary => {
                set_primary_selection(&self.display_handle, &self.seat, mime_types, ())
            }
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&self.seat).is_some() {
                    clear_data_device_selection(&self.display_handle, &self.seat)
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&self.seat).is_some() {
                    clear_primary_selection(&self.display_handle, &self.seat)
                }
            }
        }
    }
}

impl<BackendData: Backend> AmityState<BackendData> {
    /// The window element of an X11 surface, including minimized windows.
    fn x11_element(&self, surface: &X11Surface) -> Option<WindowElement> {
        self.space
            .elements()
            .chain(self.wm.windows())
            .find(|element| matches!(element.0.x11_surface(), Some(other) if other == surface))
            .cloned()
    }

    pub fn move_request_x11(&mut self, window: &X11Surface) {
        let Some(element) = self.x11_element(window) else {
            return;
        };
        // A fullscreen window stays where it is.
        if element.is_fullscreen() {
            return;
        }

        if let Some(touch) = self.seat.get_touch() {
            if let Some(start_data) = touch.grab_start_data() {
                let Some(mut initial_window_location) = self.space.element_location(&element) else {
                    return;
                };
                // A maximized or snapped window returns to its floating size under the finger.
                if element.tiling() != Tiling::Floating {
                    initial_window_location = self.untile_for_drag(&element, start_data.location);
                }
                let grab = TouchMoveSurfaceGrab {
                    start_data,
                    window: element,
                    initial_window_location,
                };
                touch.set_grab(self, grab, SERIAL_COUNTER.next_serial());
                return;
            }
        }

        let Some(start_data) = self.pointer.grab_start_data() else {
            return;
        };
        let Some(initial_window_location) = self.space.element_location(&element) else {
            return;
        };
        let grab = PointerMoveSurfaceGrab {
            start_data,
            window: element,
            initial_window_location,
        };
        let pointer = self.pointer.clone();
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }
}
