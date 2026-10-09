//! Optional shell transport integration; policy and wire types remain backend-neutral.

use super::state::Compositor;
use crate::shell::{self, RequestKind, server::Server};
use std::path::PathBuf;

impl Compositor {
    pub fn init_shell(&mut self, enabled: bool) {
        if !enabled {
            return;
        }
        let server = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| std::io::Error::other("XDG_RUNTIME_DIR is unset"))
            .and_then(|directory| Server::bind(&directory, &self.socket_name.to_string_lossy()));
        match server {
            Ok(server) => {
                eprintln!("clear: shell socket {}", server.path().display());
                self.shell_server = Some(server);
            }
            Err(error) => {
                // An optional shell must never prevent the window manager starting.
                eprintln!("clear: shell IPC unavailable: {error}; continuing without it");
            }
        }
    }

    pub fn dispatch_shell(&mut self) {
        let Some(mut server) = self.shell_server.take() else {
            return;
        };
        for (client, bytes) in server.poll() {
            let request = match shell::parse_request(&bytes) {
                Ok(request) => request,
                Err(error) => {
                    server.reply(client, shell::encode_error(None, &error));
                    continue;
                }
            };
            if let Err(error) = shell::execute(&mut self.runtime, &request) {
                server.reply(client, shell::encode_error(Some(request.id), &error));
                continue;
            }
            self.service_overview();
            match request.request {
                RequestKind::Snapshot | RequestKind::Subscribe => {
                    if matches!(request.request, RequestKind::Subscribe) {
                        server.subscribe(client);
                    }
                    server.reply(
                        client,
                        shell::encode_snapshot(request.id, &shell::snapshot(&self.runtime)),
                    );
                }
                _ => {
                    // Reconcile before acknowledging so a subsequent snapshot
                    // includes settled geometry/reservations and policy focus.
                    self.dirty = true;
                    self.reconcile();
                    server.reply(client, shell::encode_ok(request.id));
                }
            }
        }
        if server.has_subscribers() && (self.shell_dirty || self.shell_snapshot.is_none()) {
            let snapshot = shell::snapshot(&self.runtime);
            if self.shell_snapshot.as_ref() != Some(&snapshot) {
                server.publish(shell::encode_update(&snapshot));
                self.shell_snapshot = Some(snapshot);
            }
            self.shell_dirty = false;
        } else if !server.has_subscribers() {
            self.shell_snapshot = None;
        }
        self.shell_server = Some(server);
    }
}
