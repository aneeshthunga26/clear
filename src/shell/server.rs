//! Bounded, nonblocking Unix transport for the optional shell protocol.
//!
//! Call `poll` on each event-loop turn, including turns without shell requests:
//! replies and publications are queued and flushed only by `poll`. JSON parsing,
//! snapshot generation, and compositor reconciliation belong to the caller.

use std::{
    collections::VecDeque,
    fs::{self, DirBuilder, Metadata, Permissions},
    io::{self, Read, Write},
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Component, Path, PathBuf},
};

const MAX_CLIENTS: usize = 32;
const MAX_REQUEST: usize = 16 * 1024;
const MAX_OUTGOING: usize = 1024 * 1024;
const ACCEPT_ATTEMPTS: usize = 8;
const IO_ATTEMPTS_PER_CLIENT: usize = 8;
const REQUESTS_PER_CLIENT: usize = 8;
const BYTES_PER_CLIENT: usize = 32 * 1024;
const REQUESTS_PER_POLL: usize = 64;
const READ_BYTES_PER_POLL: usize = 128 * 1024;
const WRITE_BYTES_PER_POLL: usize = 256 * 1024;

/// An opaque connection identity, never reused during a server's lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClientId(u64);

/// A single-threaded shell transport with bounded work and storage per connection.
pub struct Server {
    listener: UnixListener,
    paths: OwnedPaths,
    clients: VecDeque<Client>,
    next_id: u64,
}

impl Server {
    /// Bind `<runtime_dir>/clear-<wayland_socket>/shell.sock` without replacing files.
    ///
    /// The runtime directory must be absolute, nonsymlinked, mode 0700, and owned
    /// by the effective user. The Wayland socket must be a single filename. An
    /// existing shell directory (even a stale one) is an error, not removed.
    pub fn bind(runtime_dir: &Path, wayland_socket: &str) -> io::Result<Self> {
        if wayland_socket.is_empty()
            || wayland_socket == "."
            || wayland_socket == ".."
            || wayland_socket.contains(['/', '\0'])
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Wayland socket must be a single filename",
            ));
        }
        let runtime = validate_runtime_dir(runtime_dir)?;
        let directory = runtime_dir.join(format!("clear-{wayland_socket}"));
        DirBuilder::new().mode(0o700).create(&directory)?;
        // Compare with the newly created directory's UID: std has no geteuid,
        // and environment variables are not authoritative ownership information.
        let directory_metadata = fs::symlink_metadata(&directory)?;
        let mut paths = OwnedPaths {
            runtime: runtime_dir.to_owned(),
            runtime_identity: Identity::of(&runtime),
            socket: directory.join("shell.sock"),
            directory,
            directory_identity: Identity::of(&directory_metadata),
            socket_identity: None,
        };
        if !directory_metadata.is_dir()
            || directory_metadata.uid() != runtime.uid()
            || directory_metadata.mode() & 0o777 != 0o700
            || !paths.runtime_matches()
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "runtime directory must be private and owned by the effective user",
            ));
        }
        let listener = UnixListener::bind(&paths.socket)?;
        let socket_metadata = fs::symlink_metadata(&paths.socket)?;
        paths.socket_identity = Some(Identity::of(&socket_metadata));
        if !socket_metadata.file_type().is_socket() {
            return Err(io::Error::other("shell socket was replaced during binding"));
        }
        // The enclosing 0700 directory protects the socket before chmod, without
        // changing the process-wide umask (other threads may be creating files).
        fs::set_permissions(&paths.socket, Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            paths,
            clients: VecDeque::new(),
            next_id: 1,
        })
    }

    /// Filesystem address for shell clients to connect to.
    pub fn path(&self) -> &Path {
        &self.paths.socket
    }

    /// Accept, read, and flush a bounded amount of work, never waiting for IO.
    ///
    /// Returns at most 64 frames, each at most 16 KiB excluding the stripped LF.
    /// Empty frames and non-UTF-8 bytes are passed to the protocol parser. Oversize
    /// requests, IO failures, and overflowing output queues disconnect that peer.
    /// An unterminated frame at EOF is discarded. A write-half-closed peer can
    /// still receive replies to its complete requests before being disconnected.
    pub fn poll(&mut self) -> Vec<(ClientId, Vec<u8>)> {
        for _ in 0..ACCEPT_ATTEMPTS {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if self.clients.len() == MAX_CLIENTS
                        || self.next_id == u64::MAX
                        || stream.set_nonblocking(true).is_err()
                    {
                        continue;
                    }
                    let id = ClientId(self.next_id);
                    self.next_id += 1;
                    self.clients.push_back(Client::new(id, stream));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }

        let mut requests = Vec::new();
        let mut read_budget = READ_BYTES_PER_POLL;
        let mut write_budget = WRITE_BYTES_PER_POLL;
        for _ in 0..self.clients.len() {
            let mut client = self.clients.pop_front().expect("known client count");
            let before = requests.len();
            if !client.receive(&mut requests, &mut read_budget) || !client.flush(&mut write_budget)
            {
                // Do not execute commands accumulated from a malformed peer in
                // this turn, or commands whose connection has already failed.
                requests.truncate(before);
                continue;
            }
            if !client.read_closed || !client.outgoing.is_empty() || requests.len() != before {
                self.clients.push_back(client);
            }
        }
        // Global budgets must not let a busy prefix starve later connections.
        if !self.clients.is_empty() {
            self.clients.rotate_left(1);
        }
        requests
    }

    /// Queue an already encoded response, including its newline, for one peer.
    /// Unknown/disconnected identities are ignored; queue overflow disconnects.
    pub fn reply(&mut self, id: ClientId, frame: Vec<u8>) {
        self.clients
            .retain_mut(|client| client.id != id || client.enqueue(&frame));
    }

    /// Mark a peer as subscribed. This is idempotent and sends no initial snapshot.
    pub fn subscribe(&mut self, id: ClientId) {
        if let Some(client) = self.clients.iter_mut().find(|client| client.id == id) {
            client.subscribed = true;
        }
    }

    /// Queue an encoded state event only for subscribed peers.
    pub fn publish(&mut self, frame: Vec<u8>) {
        self.clients
            .retain_mut(|client| !client.subscribed || client.enqueue(&frame));
    }

    /// Whether a connected peer has requested state publications.
    pub fn has_subscribers(&self) -> bool {
        self.clients.iter().any(|client| client.subscribed)
    }
}

struct Client {
    id: ClientId,
    stream: UnixStream,
    incoming: Vec<u8>,
    scanned: usize,
    outgoing: VecDeque<u8>,
    subscribed: bool,
    read_closed: bool,
}

impl Client {
    fn new(id: ClientId, stream: UnixStream) -> Self {
        Self {
            id,
            stream,
            incoming: Vec::new(),
            scanned: 0,
            outgoing: VecDeque::new(),
            subscribed: false,
            read_closed: false,
        }
    }

    fn enqueue(&mut self, frame: &[u8]) -> bool {
        if frame.len() > MAX_OUTGOING - self.outgoing.len() {
            return false;
        }
        // A byte queue also bounds metadata: millions of tiny/empty responses
        // must not allocate millions of per-message queue entries.
        self.outgoing.extend(frame);
        true
    }

    fn receive(
        &mut self,
        requests: &mut Vec<(ClientId, Vec<u8>)>,
        read_budget: &mut usize,
    ) -> bool {
        let mut bytes_left = BYTES_PER_CLIENT;
        let mut attempts = 0;
        let mut emitted = 0;
        loop {
            if let Some(offset) = self.incoming[self.scanned..]
                .iter()
                .position(|byte| *byte == b'\n')
            {
                let end = self.scanned + offset;
                if end > MAX_REQUEST {
                    return false;
                }
                if emitted == REQUESTS_PER_CLIENT || requests.len() == REQUESTS_PER_POLL {
                    return true;
                }
                requests.push((self.id, self.incoming[..end].to_vec()));
                self.incoming.drain(..=end);
                self.scanned = 0;
                emitted += 1;
                continue;
            }
            self.scanned = self.incoming.len();
            if self.incoming.len() > MAX_REQUEST {
                return false;
            }
            if self.read_closed
                || emitted == REQUESTS_PER_CLIENT
                || requests.len() == REQUESTS_PER_POLL
                || attempts == IO_ATTEMPTS_PER_CLIENT
                || bytes_left == 0
                || *read_budget == 0
            {
                return true;
            }
            let mut buffer = [0; 4096];
            let length = buffer
                .len()
                .min(MAX_REQUEST + 1 - self.incoming.len())
                .min(bytes_left)
                .min(*read_budget);
            attempts += 1;
            match self.stream.read(&mut buffer[..length]) {
                Ok(0) => {
                    self.read_closed = true;
                    self.incoming.clear();
                    self.scanned = 0;
                    return true;
                }
                Ok(count) => {
                    self.incoming.extend_from_slice(&buffer[..count]);
                    bytes_left -= count;
                    *read_budget -= count;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return true,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return false,
            }
        }
    }

    fn flush(&mut self, write_budget: &mut usize) -> bool {
        let mut bytes_left = BYTES_PER_CLIENT;
        for _ in 0..IO_ATTEMPTS_PER_CLIENT {
            if self.outgoing.is_empty() || bytes_left == 0 || *write_budget == 0 {
                break;
            }
            let (bytes, _) = self.outgoing.as_slices();
            let length = bytes.len().min(bytes_left).min(*write_budget);
            match self.stream.write(&bytes[..length]) {
                Ok(0) => return false,
                Ok(count) => {
                    self.outgoing.drain(..count);
                    bytes_left -= count;
                    *write_budget -= count;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return false,
            }
        }
        true
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
}

impl Identity {
    fn of(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }

    fn matches(self, path: &Path) -> bool {
        fs::symlink_metadata(path).is_ok_and(|metadata| Self::of(&metadata) == self)
    }
}

struct OwnedPaths {
    runtime: PathBuf,
    runtime_identity: Identity,
    directory: PathBuf,
    directory_identity: Identity,
    socket: PathBuf,
    socket_identity: Option<Identity>,
}

impl OwnedPaths {
    fn runtime_matches(&self) -> bool {
        self.runtime_identity.matches(&self.runtime)
    }
}

impl Drop for OwnedPaths {
    fn drop(&mut self) {
        // Never recurse, and never unlink an entry just because its name matches.
        // As with the Wayland socket, the runtime tree must remain controlled by
        // the caller's UID; std's path APIs cannot defeat hostile same-UID races.
        if !self.runtime_matches() || !self.directory_identity.matches(&self.directory) {
            return;
        }
        if self
            .socket_identity
            .is_some_and(|identity| identity.matches(&self.socket))
        {
            let _ = fs::remove_file(&self.socket);
        }
        let _ = fs::remove_dir(&self.directory);
    }
}

fn validate_runtime_dir(path: &Path) -> io::Result<Metadata> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "runtime directory must be absolute",
        ));
    }
    let mut prefix = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Normal(_) => prefix.push(component),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "runtime directory must not contain parent traversal",
                ));
            }
        }
        if !fs::symlink_metadata(&prefix)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "runtime directory and its ancestors must be nonsymlink directories",
            ));
        }
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.mode() & 0o777 != 0o700 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime directory must have mode 0700",
        ));
    }
    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::Shutdown,
        os::unix::fs::symlink,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            loop {
                let path = std::env::temp_dir().join(format!(
                    "clear-shell-{}-{}",
                    std::process::id(),
                    NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
                ));
                match DirBuilder::new().mode(0o700).create(&path) {
                    Ok(()) => {
                        fs::set_permissions(&path, Permissions::from_mode(0o700)).unwrap();
                        return Self(path);
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("creating test directory: {error}"),
                }
            }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn connect(server: &mut Server) -> (UnixStream, ClientId) {
        let stream = UnixStream::connect(server.path()).unwrap();
        stream.set_nonblocking(true).unwrap();
        let previous = server.next_id;
        assert!(server.poll().is_empty());
        let id = ClientId(previous);
        assert!(server.clients.iter().any(|client| client.id == id));
        (stream, id)
    }

    fn available(stream: &mut UnixStream) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut buffer = [0; 8192];
        for _ in 0..512 {
            match stream.read(&mut buffer) {
                Ok(0) => return bytes,
                Ok(count) => bytes.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return bytes,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => panic!("reading test stream: {error}"),
            }
        }
        panic!("test read budget exceeded");
    }

    #[test]
    fn private_permissions_conflicts_and_cleanup() {
        let temp = TempDir::new();
        let server = Server::bind(&temp.0, "wayland-1").unwrap();
        let path = server.path().to_owned();
        assert_eq!(path, temp.0.join("clear-wayland-1/shell.sock"));
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
            0o700
        );
        assert!(Server::bind(&temp.0, "wayland-1").is_err());
        assert!(UnixStream::connect(&path).is_ok());
        let other = Server::bind(&temp.0, "wayland-2").unwrap();
        drop(server);
        assert!(!path.exists());
        assert!(!path.parent().unwrap().exists());
        assert!(UnixStream::connect(other.path()).is_ok());
    }

    #[test]
    fn rejects_unsafe_paths_and_preserves_existing_entries() {
        let temp = TempDir::new();
        assert!(Server::bind(Path::new("relative"), "wayland-1").is_err());
        for name in ["", ".", "..", "../escape", "nested/name", "bad\0name"] {
            assert!(Server::bind(&temp.0, name).is_err());
        }
        for mode in [0o755, 0o770, 0o777] {
            fs::set_permissions(&temp.0, Permissions::from_mode(mode)).unwrap();
            assert!(Server::bind(&temp.0, "wayland-1").is_err());
        }
        fs::set_permissions(&temp.0, Permissions::from_mode(0o700)).unwrap();
        let link = temp.0.join("link");
        symlink(&temp.0, &link).unwrap();
        assert!(Server::bind(&link, "wayland-1").is_err());
        assert!(Server::bind(&link.join("link"), "wayland-1").is_err());
        let existing = temp.0.join("clear-wayland-1");
        fs::write(&existing, b"do not remove").unwrap();
        assert!(Server::bind(&temp.0, "wayland-1").is_err());
        assert_eq!(fs::read(&existing).unwrap(), b"do not remove");
        fs::remove_file(&existing).unwrap();
        symlink(&temp.0, &existing).unwrap();
        assert!(Server::bind(&temp.0, "wayland-1").is_err());
        assert!(fs::symlink_metadata(&existing).unwrap().is_symlink());
    }

    #[test]
    fn bind_failure_removes_created_directory() {
        let temp = TempDir::new();
        let name = "w".repeat(100);
        assert!(Server::bind(&temp.0, &name).is_err());
        assert!(!temp.0.join(format!("clear-{name}")).exists());
    }

    #[test]
    fn cleanup_preserves_replaced_socket() {
        let temp = TempDir::new();
        let server = Server::bind(&temp.0, "wayland-1").unwrap();
        let path = server.path().to_owned();
        // Keep the old inode alive so the replacement cannot reuse it.
        let old = path.with_extension("old");
        fs::rename(&path, &old).unwrap();
        let replacement = UnixListener::bind(&path).unwrap();
        drop(server);
        assert!(UnixStream::connect(&path).is_ok());
        assert!(old.exists());
        drop(replacement);
    }

    #[test]
    fn cleanup_preserves_replaced_directory() {
        let temp = TempDir::new();
        let server = Server::bind(&temp.0, "wayland-1").unwrap();
        let directory = server.path().parent().unwrap().to_owned();
        fs::rename(&directory, temp.0.join("moved")).unwrap();
        fs::create_dir(&directory).unwrap();
        let replacement = directory.join("shell.sock");
        fs::write(&replacement, b"replacement").unwrap();
        drop(server);
        assert_eq!(fs::read(&replacement).unwrap(), b"replacement");
        assert!(temp.0.join("moved/shell.sock").exists());
    }

    #[test]
    fn fragmented_and_multiple_frames_preserve_bytes() {
        let temp = TempDir::new();
        let mut server = Server::bind(&temp.0, "wayland-1").unwrap();
        let (mut stream, id) = connect(&mut server);
        stream.write_all(b"part").unwrap();
        assert!(server.poll().is_empty());
        stream.write_all(b"ial\nsecond\n\n\xff\r\nlast").unwrap();
        assert_eq!(
            server.poll(),
            vec![
                (id, b"partial".to_vec()),
                (id, b"second".to_vec()),
                (id, Vec::new()),
                (id, b"\xff\r".to_vec()),
            ]
        );
        stream.write_all(b"\n").unwrap();
        assert_eq!(server.poll(), vec![(id, b"last".to_vec())]);
        assert!(server.poll().is_empty());
    }

    #[test]
    fn request_limit_includes_exact_boundary_but_not_newline() {
        let temp = TempDir::new();
        let mut server = Server::bind(&temp.0, "wayland-1").unwrap();
        let (mut stream, id) = connect(&mut server);
        let frame = vec![b'x'; MAX_REQUEST];
        stream.write_all(&frame).unwrap();
        assert!(server.poll().is_empty());
        stream.write_all(b"\n").unwrap();
        assert_eq!(server.poll(), vec![(id, frame)]);
    }

    #[test]
    fn oversize_disconnects_only_offending_peer_with_or_without_newline() {
        let temp = TempDir::new();
        let mut server = Server::bind(&temp.0, "wayland-1").unwrap();
        let (mut healthy, healthy_id) = connect(&mut server);
        for newline in [false, true] {
            let (mut stream, id) = connect(&mut server);
            let mut frame = vec![b'x'; MAX_REQUEST + 1];
            if newline {
                frame.push(b'\n');
            }
            stream.write_all(&frame).unwrap();
            healthy.write_all(b"ok\n").unwrap();
            assert_eq!(server.poll(), vec![(healthy_id, b"ok".to_vec())]);
            assert!(!server.clients.iter().any(|client| client.id == id));
        }
    }

    #[test]
    fn subscriptions_are_idempotent_and_do_not_send_snapshots() {
        let temp = TempDir::new();
        let mut server = Server::bind(&temp.0, "wayland-1").unwrap();
        let (mut first, id) = connect(&mut server);
        let (mut second, other_id) = connect(&mut server);
        assert!(!server.has_subscribers());
        server.subscribe(id);
        server.subscribe(id);
        assert!(server.has_subscribers());
        server.poll();
        assert!(available(&mut first).is_empty());
        server.reply(other_id, b"reply\n".to_vec());
        server.publish(b"state\n".to_vec());
        server.poll();
        assert_eq!(available(&mut first), b"state\n");
        assert_eq!(available(&mut second), b"reply\n");
        drop(first);
        server.poll();
        assert!(!server.has_subscribers());
        server.reply(id, b"stale\n".to_vec());
        server.subscribe(id);
        let (mut third, new_id) = connect(&mut server);
        assert_ne!(id, new_id);
        server.publish(b"ignored\n".to_vec());
        server.poll();
        assert!(available(&mut third).is_empty());
    }

    #[test]
    fn eof_discards_fragment_and_half_close_allows_reply() {
        let temp = TempDir::new();
        let mut server = Server::bind(&temp.0, "wayland-1").unwrap();
        let (mut stream, id) = connect(&mut server);
        stream.write_all(b"complete\npartial").unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        assert_eq!(server.poll(), vec![(id, b"complete".to_vec())]);
        server.reply(id, b"response\n".to_vec());
        server.poll();
        assert_eq!(available(&mut stream), b"response\n");
        assert!(server.clients.is_empty());
    }

    #[test]
    fn accepts_and_requests_are_bounded_and_fair() {
        let temp = TempDir::new();
        let mut server = Server::bind(&temp.0, "wayland-1").unwrap();
        let mut streams = Vec::new();
        for _ in 0..ACCEPT_ATTEMPTS + 2 {
            let mut stream = UnixStream::connect(server.path()).unwrap();
            stream.set_nonblocking(true).unwrap();
            stream.write_all(&b"x\n".repeat(100)).unwrap();
            streams.push(stream);
        }
        let first = server.poll();
        assert_eq!(server.clients.len(), ACCEPT_ATTEMPTS);
        assert_eq!(first.len(), REQUESTS_PER_POLL);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..20 {
            let frames = server.poll();
            assert!(frames.len() <= REQUESTS_PER_POLL);
            for (id, _) in frames {
                seen.insert(id);
            }
        }
        assert_eq!(seen.len(), streams.len());
        assert_eq!(server.clients.len(), streams.len());
    }

    #[test]
    fn connection_limit_and_disconnected_slots_are_reusable() {
        let temp = TempDir::new();
        let mut server = Server::bind(&temp.0, "wayland-1").unwrap();
        let mut streams = Vec::new();
        for _ in 0..MAX_CLIENTS {
            streams.push(connect(&mut server).0);
        }
        let mut excess = UnixStream::connect(server.path()).unwrap();
        excess.set_nonblocking(true).unwrap();
        server.poll();
        assert_eq!(server.clients.len(), MAX_CLIENTS);
        assert_eq!(excess.read(&mut [0]).unwrap(), 0);
        streams.pop();
        server.poll();
        assert_eq!(server.clients.len(), MAX_CLIENTS - 1);
        let (_replacement, _) = connect(&mut server);
        assert_eq!(server.clients.len(), MAX_CLIENTS);
    }

    #[test]
    fn oversized_replies_and_publications_disconnect_without_buffering() {
        let temp = TempDir::new();
        let mut server = Server::bind(&temp.0, "wayland-1").unwrap();
        let (_first, id) = connect(&mut server);
        let (_second, other_id) = connect(&mut server);
        server.reply(id, vec![0; MAX_OUTGOING + 1]);
        assert_eq!(server.clients.len(), 1);
        server.publish(vec![0; MAX_OUTGOING + 1]);
        assert_eq!(server.clients.len(), 1);
        server.subscribe(other_id);
        server.publish(vec![0; MAX_OUTGOING + 1]);
        assert!(server.clients.is_empty());
    }

    #[test]
    fn slow_reader_hits_backpressure_then_recovers_with_exact_bytes() {
        let temp = TempDir::new();
        let mut server = Server::bind(&temp.0, "wayland-1").unwrap();
        let (mut stream, id) = connect(&mut server);
        let frame: Vec<_> = (0..MAX_OUTGOING).map(|i| (i % 251) as u8).collect();
        server.reply(id, frame.clone());
        server.poll();
        assert!(server.clients[0].outgoing.len() >= MAX_OUTGOING - BYTES_PER_CLIENT);
        let mut stalled = false;
        for _ in 0..128 {
            let before = server.clients[0].outgoing.len();
            server.poll();
            let after = server.clients[0].outgoing.len();
            if after > 0 && before == after {
                stalled = true;
                break;
            }
        }
        assert!(
            stalled,
            "test socket must encounter real write backpressure"
        );
        let mut received = Vec::new();
        for _ in 0..128 {
            received.extend(available(&mut stream));
            server.poll();
            if received.len() == frame.len() {
                break;
            }
        }
        assert_eq!(received, frame);
        assert!(server.clients[0].outgoing.is_empty());
        // Reuse the ring buffer after partial draining and wraparound.
        server.reply(id, b"next\n".to_vec());
        server.poll();
        assert_eq!(available(&mut stream), b"next\n");
    }

    #[test]
    fn slow_subscriber_overflow_does_not_hurt_healthy_peer() {
        let temp = TempDir::new();
        let mut server = Server::bind(&temp.0, "wayland-1").unwrap();
        let (_slow, id) = connect(&mut server);
        let (mut healthy, healthy_id) = connect(&mut server);
        server.subscribe(id);
        for _ in 0..128 {
            server.publish(vec![b'x'; BYTES_PER_CLIENT]);
            healthy.write_all(b"ping\n").unwrap();
            assert_eq!(server.poll(), vec![(healthy_id, b"ping".to_vec())]);
            if !server.has_subscribers() {
                break;
            }
        }
        assert!(!server.has_subscribers());
        assert_eq!(server.clients.len(), 1);
        assert_eq!(server.clients[0].id, healthy_id);
        server.reply(healthy_id, b"pong\n".to_vec());
        server.poll();
        assert_eq!(available(&mut healthy), b"pong\n");
    }
}
