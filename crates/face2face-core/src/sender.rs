//! The Sender, assembled: a deck source -> session -> WebSocket server.
//!
//! The desktop shell (`app/src-tauri`) starts one of these when the user picks
//! "Link up" (two-way) or "Share only", with the source they chose: mock
//! decks, or the reader for their DJ software (`sources`).
//!
//! Two-way is two of these, one per laptop, each viewed by the other: the
//! protocol stays one-directional per connection (docs/DECISIONS.md, D21).
//!
//! A Sender is reachable from other computers only with a pairing code
//! ([`Access::Paired`]); without one it listens on this computer alone.

use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::net::pairing::{Pairing, MIN_CODE_LENGTH};
use crate::net::transport::{serve, SenderIdentity};
use crate::session::Session;
use crate::sources::mock::{MockHandle, MockSource, DEMO_SESSION};
use crate::sources::{DeckSource, SourceEvent, SourceKind};

const READER_RESTART_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// The mock the desktop app offers: two decks with the scripted demo running.
const MOCK_DECKS: u8 = 2;

/// What the UI shows about a running Sender. Mirrored in `app/src/tauri.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SenderInfo {
    pub port: u16,
    pub source: SourceKind,
    /// From the source's latest status: how many decks it has, whether the DJ
    /// software is there, and the reader's own words for its state.
    pub decks: u8,
    pub connected: bool,
    pub reader_status: String,
    pub viewers: usize,
    /// Mock only: whether the scripted demo is running.
    pub demo: bool,
    /// Linked (two-way) only: the port on which the partner's Sender listens.
    pub partner_port: Option<u16>,
}

pub struct Sender {
    port: u16,
    partner_port: Option<u16>,
    session: Arc<Session>,
    source: SourceKind,
    /// The handle for mock commands, when the source is the mock.
    mock: Option<MockHandle>,
    tasks: Vec<JoinHandle<()>>,
}

/// Who may connect to a Sender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    /// Programs on this computer only, without a code: tests, the `sender`
    /// example, and a browser during development.
    ThisComputer,
    /// Anyone on this computer's networks who presents this pairing code
    /// (`net::pairing`). The one way to listen beyond this computer.
    Paired { code: String },
}

impl Access {
    fn listen_on(&self, port: u16) -> anyhow::Result<SocketAddrV4> {
        match self {
            Access::ThisComputer => Ok(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)),
            Access::Paired { code } => {
                anyhow::ensure!(code.len() >= MIN_CODE_LENGTH, "the pairing code needs at least {MIN_CODE_LENGTH} characters");
                Ok(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port))
            }
        }
    }

    fn pairing(self) -> Option<Arc<Pairing>> {
        match self {
            Access::ThisComputer => None,
            Access::Paired { code } => Some(Arc::new(Pairing::new(code))),
        }
    }
}

/// What a Sender reads its decks from.
enum Decks {
    Mock { deck_count: u8, demo: bool },
    Reader(SourceKind, Box<dyn DeckSource>),
}

impl Decks {
    fn of(kind: SourceKind) -> Self {
        match kind.reader() {
            Some(reader) => Decks::Reader(kind, reader),
            None => Decks::Mock { deck_count: MOCK_DECKS, demo: true },
        }
    }

    /// For the name a Viewer sees, e.g. "Laptop A (Traktor)".
    fn label(&self) -> &'static str {
        match self {
            Decks::Mock { .. } | Decks::Reader(SourceKind::Mock, _) => "mock decks",
            Decks::Reader(SourceKind::Traktor, _) => "Traktor",
            Decks::Reader(SourceKind::Rekordbox, _) => "rekordbox",
            Decks::Reader(SourceKind::ProDjLink, _) => "Pro DJ Link",
            Decks::Reader(SourceKind::Mixxx, _) => "Mixxx",
        }
    }
}

/// Aborts the task it holds when dropped, so stopping the supervisor stops the reader.
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Watches the running reader. A reader that ends by itself (it panicked)
/// would leave the decks frozen while heartbeats still say "connected": the
/// Viewers are told it stopped, and a reader for real software is started again.
async fn supervise(
    kind: SourceKind,
    reader: JoinHandle<()>,
    events: mpsc::UnboundedSender<SourceEvent>,
    session: Arc<Session>,
) {
    let mut running = AbortOnDrop(reader);
    loop {
        let ended = (&mut running.0).await;
        tracing::error!("the {kind:?} reader stopped: {ended:?}");
        if let Some(mut status) = session.source_status() {
            status.connected = false;
            status.reader_status = "the reader stopped; restarting".to_string();
            session.apply(SourceEvent::Status(status));
        }
        tokio::time::sleep(READER_RESTART_DELAY).await;
        // The mock cannot be rebuilt here (its command handle is already out).
        let Some(reader) = kind.reader() else { return };
        running = AbortOnDrop(reader.spawn(events.clone()));
    }
}

fn identity(name: &str) -> SenderIdentity {
    // A random, persistent device id arrives with pairing in Milestone 1.
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    SenderIdentity {
        device_id: format!("sender-{:x}-{:x}", std::process::id(), nanos),
        name: name.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
    }
}

impl Sender {
    /// Starts reading `source` and listens on `port` (0 picks a free one) for
    /// the Viewers that `access` allows.
    ///
    /// Must be called from within a Tokio runtime.
    pub async fn start(port: u16, source: SourceKind, access: Access) -> anyhow::Result<Self> {
        Self::listen(port, Decks::of(source), access).await
    }

    /// A Sender for this computer only, with mock decks of your choosing.
    pub async fn start_mock(port: u16, deck_count: u8, demo: bool) -> anyhow::Result<Self> {
        Self::listen(port, Decks::Mock { deck_count, demo }, Access::ThisComputer).await
    }

    async fn listen(port: u16, decks: Decks, access: Access) -> anyhow::Result<Self> {
        let address = access.listen_on(port)?;
        let listener = TcpListener::bind(address)
            .await
            .with_context(|| format!("could not listen on {address} (is another Face2Face DJ Sender running?)"))?;
        let name = format!("Face2Face DJ Sender ({})", decks.label());
        Self::run(listener, None, &name, decks, 0, access)
    }

    /// Two-way ("Link up"): listens on the first of `ports` that is free and
    /// names the other one as the port of a partner *on this computer*.
    ///
    /// On two laptops both get the first port, and each DJ enters the other's
    /// address. On one computer (for trying it out) the second window gets
    /// the second port, so the first to link is "Laptop A", the second
    /// "Laptop B", and each can view the other. Two mock sides load different
    /// mock tracks.
    ///
    /// Must be called from within a Tokio runtime.
    pub async fn start_link(ports: [u16; 2], source: SourceKind, access: Access) -> anyhow::Result<Self> {
        for (side, port) in ports.into_iter().enumerate() {
            let address = access.listen_on(port)?;
            match TcpListener::bind(address).await {
                Ok(listener) => {
                    let decks = Decks::of(source);
                    let name = format!("Laptop {} ({})", if side == 0 { 'A' } else { 'B' }, decks.label());
                    let first_track = side * usize::from(MOCK_DECKS);
                    return Self::run(listener, Some(ports[1 - side]), &name, decks, first_track, access);
                }
                Err(error) if error.kind() == ErrorKind::AddrInUse => continue,
                Err(error) => return Err(error).with_context(|| format!("could not listen on {address}")),
            }
        }
        anyhow::bail!(
            "ports {} and {} are both in use (are two Face2Face DJ windows already linked on this computer?)",
            ports[0],
            ports[1]
        )
    }

    fn run(
        listener: TcpListener,
        partner_port: Option<u16>,
        name: &str,
        decks: Decks,
        first_mock_track: usize,
        access: Access,
    ) -> anyhow::Result<Self> {
        let listener_address = listener.local_addr()?;
        let port = listener_address.port();
        let session = Session::new();
        let (source, mock, reader): (SourceKind, Option<MockHandle>, Box<dyn DeckSource>) = match decks {
            Decks::Mock { deck_count, demo } => {
                let mock = MockSource::starting_at(deck_count, Some(DEMO_SESSION), demo, first_mock_track)
                    .map_err(anyhow::Error::msg)?;
                (SourceKind::Mock, Some(mock.handle()), Box::new(mock))
            }
            Decks::Reader(kind, reader) => (kind, None, reader),
        };
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();

        let pump_session = Arc::clone(&session);
        let reader_name = reader.name();
        let tasks = vec![
            // Started here, not inside the supervisor's task: a mock command
            // issued right after `start` must find the reader already wired up.
            tokio::spawn(supervise(source, reader.spawn(events_tx.clone()), events_tx, Arc::clone(&session))),
            tokio::spawn(async move {
                while let Some(event) = events_rx.recv().await {
                    pump_session.apply(event);
                }
            }),
            tokio::spawn(serve(listener, Arc::clone(&session), Arc::new(identity(name)), access.pairing())),
        ];
        tracing::info!("sender listening on {}, reading {reader_name}", listener_address);
        Ok(Self { port, partner_port, session, source, mock, tasks })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn info(&self) -> SenderInfo {
        let status = self.session.source_status();
        SenderInfo {
            port: self.port,
            source: self.source,
            decks: status.as_ref().map_or(2, |status| status.decks),
            connected: status.as_ref().is_some_and(|status| status.connected),
            reader_status: status.map(|status| status.reader_status).unwrap_or_default(),
            viewers: self.session.viewer_count(),
            demo: self.mock.as_ref().is_some_and(MockHandle::demo),
            partner_port: self.partner_port,
        }
    }

    /// Runs one mock command line, e.g. `play 1`. See `sources::mock`.
    pub fn command(&self, line: &str) -> Result<(), String> {
        match &self.mock {
            Some(mock) => mock.command(line),
            None => Err("Only mock decks take commands.".to_string()),
        }
    }

    /// Stops everything and waits until the port is free again.
    pub async fn stop(mut self) {
        for task in std::mem::take(&mut self.tasks) {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
