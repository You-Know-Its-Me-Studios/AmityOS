use std::{borrow::Cow, time::Duration};

use smithay::{
    backend::renderer::{
        element::{
            memory::MemoryRenderBufferRenderElement, solid::SolidColorRenderElement,
            surface::WaylandSurfaceRenderElement, AsRenderElements,
        },
        ImportAll, ImportMem, Renderer, Texture,
    },
    desktop::{
        space::SpaceElement, utils::OutputPresentationFeedback, Window, WindowSurface, WindowSurfaceType,
    },
    input::{
        pointer::{
            AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
            GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
            GestureSwipeUpdateEvent, MotionEvent, PointerTarget, RelativeMotionEvent,
        },
        touch::TouchTarget,
        Seat,
    },
    output::Output,
    reexports::{
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    render_elements,
    utils::{user_data::UserDataMap, IsAlive, Logical, Physical, Point, Rectangle, Scale, Serial},
    wayland::{compositor::SurfaceData as WlSurfaceData, dmabuf::DmabufFeedback, seat::WaylandFocus},
};

use super::ssd::{HeaderAction, HEADER_BAR_HEIGHT};
use crate::{focus::PointerFocusTarget, layout::Tiling, state::Backend, AmityState};

#[derive(Debug, Clone, PartialEq)]
pub struct WindowElement(pub Window);

impl WindowElement {
    pub fn surface_under(
        &self,
        location: Point<f64, Logical>,
        window_type: WindowSurfaceType,
    ) -> Option<(PointerFocusTarget, Point<i32, Logical>)> {
        let header = self.has_header();
        let offset = if header {
            Point::from((0, HEADER_BAR_HEIGHT))
        } else {
            Point::default()
        };
        if header {
            // One of the window's own menus may overlap its frame; the menu comes first.
            let on_popup = self
                .0
                .surface_under(location - offset.to_f64(), WindowSurfaceType::POPUP)
                .is_some();
            if !on_popup && (location.y < HEADER_BAR_HEIGHT as f64 || self.resize_edges_at(location).any()) {
                // The title bar, or the band inside the frame that resizes the window.
                return Some((PointerFocusTarget::SSD(SSD(self.clone())), Point::default()));
            }
        }

        let surface_under = self.0.surface_under(location - offset.to_f64(), window_type);
        let (under, loc) = match self.0.underlying_surface() {
            WindowSurface::Wayland(_) => {
                surface_under.map(|(surface, loc)| (PointerFocusTarget::WlSurface(surface), loc))
            }
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(s) => {
                surface_under.map(|(_, loc)| (PointerFocusTarget::X11Surface(s.clone()), loc))
            }
        }?;
        Some((under, loc + offset))
    }

    pub fn with_surfaces<F>(&self, processor: F)
    where
        F: FnMut(&WlSurface, &WlSurfaceData),
    {
        self.0.with_surfaces(processor);
    }

    pub fn send_frame<T, F>(
        &self,
        output: &Output,
        time: T,
        throttle: Option<Duration>,
        primary_scan_out_output: F,
    ) where
        T: Into<Duration>,
        F: FnMut(&WlSurface, &WlSurfaceData) -> Option<Output> + Copy,
    {
        self.0.send_frame(output, time, throttle, primary_scan_out_output)
    }

    pub fn send_dmabuf_feedback<'a, P, F>(
        &self,
        output: &Output,
        primary_scan_out_output: P,
        select_dmabuf_feedback: F,
    ) where
        P: FnMut(&WlSurface, &WlSurfaceData) -> Option<Output> + Copy,
        F: Fn(&WlSurface, &WlSurfaceData) -> &'a DmabufFeedback + Copy,
    {
        self.0
            .send_dmabuf_feedback(output, primary_scan_out_output, select_dmabuf_feedback)
    }

    pub fn take_presentation_feedback<F1, F2>(
        &self,
        output_feedback: &mut OutputPresentationFeedback,
        primary_scan_out_output: F1,
        presentation_feedback_flags: F2,
    ) where
        F1: FnMut(&WlSurface, &WlSurfaceData) -> Option<Output> + Copy,
        F2: FnMut(&WlSurface, &WlSurfaceData) -> wp_presentation_feedback::Kind + Copy,
    {
        self.0.take_presentation_feedback(
            output_feedback,
            primary_scan_out_output,
            presentation_feedback_flags,
        )
    }

    #[cfg(feature = "xwayland")]
    #[inline]
    pub fn is_x11(&self) -> bool {
        self.0.is_x11()
    }

    #[inline]
    pub fn is_wayland(&self) -> bool {
        self.0.is_wayland()
    }

    #[inline]
    pub fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        self.0.wl_surface()
    }

    #[inline]
    pub fn user_data(&self) -> &UserDataMap {
        self.0.user_data()
    }
}

impl IsAlive for WindowElement {
    #[inline]
    fn alive(&self) -> bool {
        self.0.alive()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SSD(WindowElement);

impl IsAlive for SSD {
    #[inline]
    fn alive(&self) -> bool {
        self.0.alive()
    }
}

impl SSD {
    /// Schedules a title-bar action. It runs from an idle callback because it
    /// may start a pointer grab or change the window's decoration state, which
    /// must not happen inside the input callback that produced it.
    fn run<BackendData: Backend>(
        &self,
        seat: &Seat<AmityState<BackendData>>,
        data: &mut AmityState<BackendData>,
        action: Option<HeaderAction>,
        serial: Serial,
    ) {
        if let Some(action) = action {
            let (window, seat) = (self.0.clone(), seat.clone());
            data.handle
                .insert_idle(move |data| data.header_action(&window, action, &seat, serial));
        }
    }
}

impl WaylandFocus for SSD {
    #[inline]
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        self.0.wl_surface()
    }
}

impl<BackendData: Backend> PointerTarget<AmityState<BackendData>> for SSD {
    fn enter(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        event: &MotionEvent,
    ) {
        let mut state = self.0.decoration_state();
        if state.is_ssd {
            state.header_bar.pointer_enter(event.location);
        }
    }
    fn motion(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        event: &MotionEvent,
    ) {
        let mut state = self.0.decoration_state();
        if state.is_ssd {
            state.header_bar.pointer_enter(event.location);
        }
    }
    fn relative_motion(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        _event: &RelativeMotionEvent,
    ) {
    }
    fn button(
        &self,
        seat: &Seat<AmityState<BackendData>>,
        data: &mut AmityState<BackendData>,
        event: &ButtonEvent,
    ) {
        let pointer = self.0.decoration_state().header_bar.pointer_location();
        let edges = pointer
            .map(|location| self.0.resize_edges_at(location))
            .unwrap_or_default();
        let action = {
            let mut state = self.0.decoration_state();
            if state.is_ssd {
                state.header_bar.pointer_button(
                    event.button,
                    event.state == smithay::backend::input::ButtonState::Pressed,
                    event.time,
                    edges,
                )
            } else {
                None
            }
        };
        self.run(seat, data, action, event.serial);
    }
    fn axis(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        _frame: AxisFrame,
    ) {
    }
    fn frame(&self, _seat: &Seat<AmityState<BackendData>>, _data: &mut AmityState<BackendData>) {}
    fn leave(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        _serial: Serial,
        _time: u32,
    ) {
        let mut state = self.0.decoration_state();
        if state.is_ssd {
            state.header_bar.pointer_leave();
        }
    }
    fn gesture_swipe_begin(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        _event: &GestureSwipeBeginEvent,
    ) {
    }
    fn gesture_swipe_update(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        _event: &GestureSwipeUpdateEvent,
    ) {
    }
    fn gesture_swipe_end(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        _event: &GestureSwipeEndEvent,
    ) {
    }
    fn gesture_pinch_begin(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        _event: &GesturePinchBeginEvent,
    ) {
    }
    fn gesture_pinch_update(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        _event: &GesturePinchUpdateEvent,
    ) {
    }
    fn gesture_pinch_end(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        _event: &GesturePinchEndEvent,
    ) {
    }
    fn gesture_hold_begin(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        _event: &GestureHoldBeginEvent,
    ) {
    }
    fn gesture_hold_end(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        _event: &GestureHoldEndEvent,
    ) {
    }
}

impl<BackendData: Backend> TouchTarget<AmityState<BackendData>> for SSD {
    fn down(
        &self,
        seat: &Seat<AmityState<BackendData>>,
        data: &mut AmityState<BackendData>,
        event: &smithay::input::touch::DownEvent,
        _seq: Serial,
    ) {
        let action = {
            let mut state = self.0.decoration_state();
            if state.is_ssd {
                state.header_bar.pointer_enter(event.location);
                state.header_bar.touch_down()
            } else {
                None
            }
        };
        self.run(seat, data, action, event.serial);
    }

    fn up(
        &self,
        seat: &Seat<AmityState<BackendData>>,
        data: &mut AmityState<BackendData>,
        event: &smithay::input::touch::UpEvent,
        _seq: Serial,
    ) {
        let action = {
            let mut state = self.0.decoration_state();
            if state.is_ssd {
                state.header_bar.touch_up()
            } else {
                None
            }
        };
        self.run(seat, data, action, event.serial);
    }

    fn motion(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        event: &smithay::input::touch::MotionEvent,
        _seq: Serial,
    ) {
        let mut state = self.0.decoration_state();
        if state.is_ssd {
            state.header_bar.pointer_enter(event.location);
        }
    }

    fn frame(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        _seq: Serial,
    ) {
    }

    fn cancel(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        _seq: Serial,
    ) {
    }

    fn shape(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        _event: &smithay::input::touch::ShapeEvent,
        _seq: Serial,
    ) {
    }

    fn orientation(
        &self,
        _seat: &Seat<AmityState<BackendData>>,
        _data: &mut AmityState<BackendData>,
        _event: &smithay::input::touch::OrientationEvent,
        _seq: Serial,
    ) {
    }
}

impl SpaceElement for WindowElement {
    fn geometry(&self) -> Rectangle<i32, Logical> {
        let mut geo = SpaceElement::geometry(&self.0);
        if self.has_header() {
            geo.size.h += HEADER_BAR_HEIGHT;
        }
        geo
    }
    fn bbox(&self) -> Rectangle<i32, Logical> {
        let mut bbox = SpaceElement::bbox(&self.0);
        if self.has_header() {
            bbox.size.h += HEADER_BAR_HEIGHT;
        }
        bbox
    }
    fn is_in_input_region(&self, point: &Point<f64, Logical>) -> bool {
        if self.has_header() {
            point.y < HEADER_BAR_HEIGHT as f64
                || SpaceElement::is_in_input_region(
                    &self.0,
                    &(*point - Point::from((0.0, HEADER_BAR_HEIGHT as f64))),
                )
        } else {
            SpaceElement::is_in_input_region(&self.0, point)
        }
    }
    fn z_index(&self) -> u8 {
        SpaceElement::z_index(&self.0)
    }

    fn set_activate(&self, activated: bool) {
        SpaceElement::set_activate(&self.0, activated);
    }
    fn output_enter(&self, output: &Output, overlap: Rectangle<i32, Logical>) {
        SpaceElement::output_enter(&self.0, output, overlap);
    }
    fn output_leave(&self, output: &Output) {
        SpaceElement::output_leave(&self.0, output);
    }
    #[profiling::function]
    fn refresh(&self) {
        SpaceElement::refresh(&self.0);
    }
}

render_elements!(
    pub WindowRenderElement<R> where R: ImportAll + ImportMem;
    Window=WaylandSurfaceRenderElement<R>,
    Decoration=SolidColorRenderElement,
    Glyph=MemoryRenderBufferRenderElement<R>,
);

impl<R: Renderer> std::fmt::Debug for WindowRenderElement<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Window(arg0) => f.debug_tuple("Window").field(arg0).finish(),
            Self::Decoration(arg0) => f.debug_tuple("Decoration").field(arg0).finish(),
            Self::Glyph(arg0) => f.debug_tuple("Glyph").field(arg0).finish(),
            Self::_GenericCatcher(arg0) => f.debug_tuple("_GenericCatcher").field(arg0).finish(),
        }
    }
}

impl<R> AsRenderElements<R> for WindowElement
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + Texture + 'static,
{
    type RenderElement = WindowRenderElement<R>;

    fn render_elements<C: From<Self::RenderElement>>(
        &self,
        renderer: &mut R,
        mut location: Point<i32, Physical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> Vec<C> {
        let window_bbox = SpaceElement::bbox(&self.0);

        if self.has_header() && !window_bbox.is_empty() {
            let window_geo = SpaceElement::geometry(&self.0);

            let maximized = self.tiling() == Tiling::Maximized;
            let mut vec = {
                let mut state = self.decoration_state();
                state.header_bar.redraw(window_geo.size.w);
                state
                    .header_bar
                    .render_elements(renderer, location, scale, alpha, maximized)
            };

            location.y += (scale.y * HEADER_BAR_HEIGHT as f64) as i32;

            let window_elements =
                AsRenderElements::render_elements(&self.0, renderer, location, scale, alpha);
            vec.extend(window_elements);
            vec.into_iter().map(C::from).collect()
        } else {
            AsRenderElements::render_elements(&self.0, renderer, location, scale, alpha)
                .into_iter()
                .map(C::from)
                .collect()
        }
    }
}
