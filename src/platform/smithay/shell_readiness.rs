//! Calloop ownership for the backend-neutral bounded Unix shell transport.

use super::super::state::Compositor;
use crate::shell::server::ClientId;
use smithay::reexports::calloop::{
    Interest, LoopHandle, Mode, PostAction, RegistrationToken,
    generic::Generic,
    ping::{Ping, make_ping},
};
use std::{cell::Cell, io, rc::Rc};

struct ClientRegistration {
    id: ClientId,
    readable: bool,
    writable: bool,
    token: RegistrationToken,
}

/// Adapter-owned descriptor registrations and one coalesced next-turn continuation.
/// Cloned descriptors never perform IO and are removed on disconnect or interest change.
pub(in crate::platform::smithay) struct ShellReadiness {
    handle: LoopHandle<'static, Compositor>,
    listener: Option<RegistrationToken>,
    clients: Vec<ClientRegistration>,
    continuation: Ping,
    continuation_token: RegistrationToken,
    continuation_pending: Rc<Cell<bool>>,
}

impl ShellReadiness {
    pub(in crate::platform::smithay) fn new(
        handle: &LoopHandle<'static, Compositor>,
        state: &mut Compositor,
    ) -> io::Result<Self> {
        let (continuation, source) = make_ping()?;
        let continuation_pending = Rc::new(Cell::new(false));
        let pending = continuation_pending.clone();
        let continuation_token = handle
            .insert_source(source, move |_, _, state| {
                pending.set(false);
                state.render_demand.wake();
            })
            .map_err(|error| io::Error::other(error.to_string()))?;
        let mut registrations = Self {
            handle: handle.clone(),
            listener: None,
            clients: Vec::new(),
            continuation,
            continuation_token,
            continuation_pending,
        };
        registrations.sync(state)?;
        Ok(registrations)
    }

    /// Call after dispatch and publications, including changes caused by ordinary Wayland IO.
    pub(in crate::platform::smithay) fn sync(&mut self, state: &mut Compositor) -> io::Result<()> {
        let Some(server) = state.shell_server.as_ref() else {
            self.clear_descriptors();
            return Ok(());
        };
        if self.listener.is_none() {
            let listener = server.listener().try_clone()?;
            self.listener = Some(
                self.handle
                    .insert_source(
                        Generic::new(listener, Interest::READ, Mode::Level),
                        |_, _, state| {
                            if let Some(server) = state.shell_server.as_mut() {
                                server.mark_listener_ready();
                            }
                            state.render_demand.wake();
                            Ok(PostAction::Continue)
                        },
                    )
                    .map_err(|error| io::Error::other(error.to_string()))?,
            );
        }
        self.clients.retain(|registration| {
            let retain = server.interests().any(|(id, _, readable, writable)| {
                id == registration.id
                    && readable == registration.readable
                    && writable == registration.writable
                    && (readable || writable)
            });
            if !retain {
                self.handle.remove(registration.token);
            }
            retain
        });
        for (id, stream, readable, writable) in server.interests() {
            if !(readable || writable) || self.clients.iter().any(|entry| entry.id == id) {
                continue;
            }
            let interest = Interest { readable, writable };
            let token = self
                .handle
                .insert_source(
                    Generic::new(stream.try_clone()?, interest, Mode::Level),
                    move |event, _, state| {
                        if let Some(server) = state.shell_server.as_mut() {
                            server.mark_ready(id, event.readable, event.writable);
                        }
                        state.render_demand.wake();
                        Ok(PostAction::Continue)
                    },
                )
                .map_err(|error| io::Error::other(error.to_string()))?;
            self.clients.push(ClientRegistration {
                id,
                readable,
                writable,
                token,
            });
        }
        if server.needs_continuation() && !self.continuation_pending.replace(true) {
            self.continuation.ping();
        }
        Ok(())
    }

    fn clear_descriptors(&mut self) {
        if let Some(listener) = self.listener.take() {
            self.handle.remove(listener);
        }
        for registration in self.clients.drain(..) {
            self.handle.remove(registration.token);
        }
    }
}

impl Drop for ShellReadiness {
    fn drop(&mut self) {
        self.clear_descriptors();
        self.handle.remove(self.continuation_token);
    }
}
