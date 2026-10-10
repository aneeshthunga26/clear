//! Surface-local pointer delivery through a sampled presentation scale.

use super::state::Compositor;
use smithay::{
    input::{
        Seat,
        dnd::{DndFocus, Source},
        pointer::*,
    },
    reexports::wayland_server::{
        DisplayHandle, Resource, backend::ObjectId, protocol::wl_surface::WlSurface,
    },
    utils::{IsAlive, Logical, Point, Serial},
    wayland::seat::WaylandFocus,
};
use std::{borrow::Cow, sync::Arc};

/// Surface identity stays stable while the displayed scale changes between frames.
#[derive(Debug, Clone)]
pub(super) struct PointerFocus {
    pub surface: WlSurface,
    inverse_scale: Point<f64, Logical>,
}

impl PointerFocus {
    pub fn new(surface: WlSurface, scale: Point<f64, Logical>) -> Self {
        debug_assert!(scale.x.is_finite() && scale.y.is_finite() && scale.x > 0.0 && scale.y > 0.0);
        Self {
            surface,
            inverse_scale: (1.0 / scale.x, 1.0 / scale.y).into(),
        }
    }

    pub fn id(&self) -> ObjectId {
        self.surface.id()
    }

    fn vector(&self, value: Point<f64, Logical>) -> Point<f64, Logical> {
        inverse_vector(value, self.inverse_scale)
    }

    fn motion_event(&self, event: &MotionEvent) -> MotionEvent {
        MotionEvent {
            location: self.vector(event.location),
            ..event.clone()
        }
    }
}

fn inverse_vector(value: Point<f64, Logical>, inverse: Point<f64, Logical>) -> Point<f64, Logical> {
    (value.x * inverse.x, value.y * inverse.y).into()
}

impl PartialEq for PointerFocus {
    fn eq(&self, other: &Self) -> bool {
        self.surface == other.surface
    }
}
impl IsAlive for PointerFocus {
    fn alive(&self) -> bool {
        Resource::is_alive(&self.surface)
    }
}
impl WaylandFocus for PointerFocus {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        Some(Cow::Borrowed(&self.surface))
    }
}

impl PointerTarget<Compositor> for PointerFocus {
    fn enter(&self, seat: &Seat<Compositor>, data: &mut Compositor, event: &MotionEvent) {
        PointerTarget::<Compositor>::enter(&self.surface, seat, data, &self.motion_event(event));
    }
    fn motion(&self, seat: &Seat<Compositor>, data: &mut Compositor, event: &MotionEvent) {
        PointerTarget::<Compositor>::motion(&self.surface, seat, data, &self.motion_event(event));
    }
    fn relative_motion(
        &self,
        seat: &Seat<Compositor>,
        data: &mut Compositor,
        event: &RelativeMotionEvent,
    ) {
        let event = RelativeMotionEvent {
            delta: self.vector(event.delta),
            delta_unaccel: self.vector(event.delta_unaccel),
            ..event.clone()
        };
        self.surface.relative_motion(seat, data, &event);
    }
    fn button(&self, seat: &Seat<Compositor>, data: &mut Compositor, event: &ButtonEvent) {
        self.surface.button(seat, data, event);
    }
    fn axis(&self, seat: &Seat<Compositor>, data: &mut Compositor, frame: AxisFrame) {
        self.surface.axis(seat, data, frame);
    }
    fn frame(&self, seat: &Seat<Compositor>, data: &mut Compositor) {
        self.surface.frame(seat, data);
    }
    fn leave(&self, seat: &Seat<Compositor>, data: &mut Compositor, serial: Serial, time: u32) {
        PointerTarget::<Compositor>::leave(&self.surface, seat, data, serial, time);
    }
    fn gesture_swipe_begin(
        &self,
        seat: &Seat<Compositor>,
        data: &mut Compositor,
        event: &GestureSwipeBeginEvent,
    ) {
        self.surface.gesture_swipe_begin(seat, data, event);
    }
    fn gesture_swipe_update(
        &self,
        seat: &Seat<Compositor>,
        data: &mut Compositor,
        event: &GestureSwipeUpdateEvent,
    ) {
        self.surface.gesture_swipe_update(seat, data, event);
    }
    fn gesture_swipe_end(
        &self,
        seat: &Seat<Compositor>,
        data: &mut Compositor,
        event: &GestureSwipeEndEvent,
    ) {
        self.surface.gesture_swipe_end(seat, data, event);
    }
    fn gesture_pinch_begin(
        &self,
        seat: &Seat<Compositor>,
        data: &mut Compositor,
        event: &GesturePinchBeginEvent,
    ) {
        self.surface.gesture_pinch_begin(seat, data, event);
    }
    fn gesture_pinch_update(
        &self,
        seat: &Seat<Compositor>,
        data: &mut Compositor,
        event: &GesturePinchUpdateEvent,
    ) {
        self.surface.gesture_pinch_update(seat, data, event);
    }
    fn gesture_pinch_end(
        &self,
        seat: &Seat<Compositor>,
        data: &mut Compositor,
        event: &GesturePinchEndEvent,
    ) {
        self.surface.gesture_pinch_end(seat, data, event);
    }
    fn gesture_hold_begin(
        &self,
        seat: &Seat<Compositor>,
        data: &mut Compositor,
        event: &GestureHoldBeginEvent,
    ) {
        self.surface.gesture_hold_begin(seat, data, event);
    }
    fn gesture_hold_end(
        &self,
        seat: &Seat<Compositor>,
        data: &mut Compositor,
        event: &GestureHoldEndEvent,
    ) {
        self.surface.gesture_hold_end(seat, data, event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inverse_pointer_coordinates_preserve_offsets_and_independent_axis_scales() {
        assert_eq!(
            inverse_vector((90.5, -36.0).into(), (0.5, 2.0).into()),
            (45.25, -72.0).into()
        );
        let local: Point<f64, Logical> = (12.25, 49.5).into();
        let displayed = (local.x * 1.04, local.y * 0.75).into();
        let recovered = inverse_vector(displayed, (1.0 / 1.04, 1.0 / 0.75).into());
        assert!((recovered.x - local.x).abs() < 1e-12);
        assert!((recovered.y - local.y).abs() < 1e-12);
        assert_eq!(inverse_vector(local, (1.0, 1.0).into()), local);
    }
}

impl From<WlSurface> for PointerFocus {
    fn from(surface: WlSurface) -> Self {
        Self::new(surface, (1.0, 1.0).into())
    }
}

impl DndFocus<Compositor> for PointerFocus {
    type OfferData<S: Source> = <WlSurface as DndFocus<Compositor>>::OfferData<S>;
    fn enter<S: Source>(
        &self,
        data: &mut Compositor,
        dh: &DisplayHandle,
        source: Arc<S>,
        seat: &Seat<Compositor>,
        location: Point<f64, Logical>,
        serial: &Serial,
    ) -> Option<Self::OfferData<S>> {
        DndFocus::<Compositor>::enter(
            &self.surface,
            data,
            dh,
            source,
            seat,
            self.vector(location),
            serial,
        )
    }
    fn motion<S: Source>(
        &self,
        data: &mut Compositor,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<Compositor>,
        location: Point<f64, Logical>,
        time: u32,
    ) {
        DndFocus::<Compositor>::motion(
            &self.surface,
            data,
            offer,
            seat,
            self.vector(location),
            time,
        );
    }
    fn leave<S: Source>(
        &self,
        data: &mut Compositor,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<Compositor>,
    ) {
        DndFocus::<Compositor>::leave(&self.surface, data, offer, seat);
    }
    fn drop<S: Source>(
        &self,
        data: &mut Compositor,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<Compositor>,
    ) {
        DndFocus::<Compositor>::drop(&self.surface, data, offer, seat);
    }
}
