//! The Milestone 0 Sender, assembled: mock decks -> session -> WebSocket server.
//!
//! The desktop shell (`app/src-tauri`) starts one of these when the user picks
//! "Link up" (two-way) or "Share only". Real sources replace the mock in
//! Milestones 3 and 4.
//!
//! Two-way is two of these, one per laptop, each viewed by the other: the
//! protocol stays one-directional per connection (docs/DECISIONS.md, D21).

use std::io::ErrorKind;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::net::transport::{serve, SenderIdentity};
use crate::session::Session;
use crate::sources::mock::{MockHandle, MockSource, DEMO_SESSION};
use crate::sources::DeckSource;

/// What the UI shows about a running Sender. Mirrored in `app/src/tauri.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SenderInfo {
    pub port: u16,
    pub decks: u8,
    pub viewers: usize,
    pub demo: bool,
    /// Linked (two-way) only: the port on which the partner's Sender listens.
    pub partner_port: Option<u16>,
}

pub struct MockSender {
    port: u16,
    partner_port: Option<u16>,
    session: Arc<Session>,
    mock: MockHandle,
    tasks: Vec<JoinHandle<()>>,
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

impl MockSender {
    /// Starts mock decks and listens on `127.0.0.1:port` (port 0 picks a free
    /// one). Loopback only: there is no pairing before Milestone 1, so the
    /// Sender must not be reachable from the network yet.
    ///
    /// Must be called from within a Tokio runtime.
    pub async fn start(port: u16, deck_count: u8, demo: bool) -> anyhow::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .with_context(|| format!("could not listen on 127.0.0.1:{port} (is another Face2Face DJ Sender running?)"))?;
        Self::run(listener, None, "Face2Face DJ Sender (mock decks)", deck_count, demo, 0)
    }

    /// Two-way on one computer (Milestone 0): listens on the first of `ports`
    /// that is free and names the other one as the partner's. So the first
    /// instance to link becomes "Laptop A", the second "Laptop B", and each
    /// views the other. The two sides load different mock tracks.
    ///
    /// Must be called from within a Tokio runtime.
    pub async fn start_link(ports: [u16; 2], deck_count: u8, demo: bool) -> anyhow::Result<Self> {
        for (side, port) in ports.into_iter().enumerate() {
            match TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
                Ok(listener) => {
                    let name = format!("Laptop {} (mock decks)", if side == 0 { 'A' } else { 'B' });
                    let first_track = side * usize::from(deck_count.clamp(1, 4));
                    return Self::run(listener, Some(ports[1 - side]), &name, deck_count, demo, first_track);
                }
                Err(error) if error.kind() == ErrorKind::AddrInUse => continue,
                Err(error) => return Err(error).with_context(|| format!("could not listen on 127.0.0.1:{port}")),
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
        deck_count: u8,
        demo: bool,
        first_track: usize,
    ) -> anyhow::Result<Self> {
        let port = listener.local_addr()?.port();

        let session = Session::new();
        let source =
            MockSource::starting_at(deck_count, Some(DEMO_SESSION), demo, first_track).map_err(anyhow::Error::msg)?;
        let mock = source.handle();
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();

        let pump_session = Arc::clone(&session);
        let tasks = vec![
            Box::new(source).spawn(events_tx),
            tokio::spawn(async move {
                while let Some(event) = events_rx.recv().await {
                    pump_session.apply(event);
                }
            }),
            tokio::spawn(serve(listener, Arc::clone(&session), Arc::new(identity(name)))),
        ];
        tracing::info!("mock sender listening on 127.0.0.1:{port}");
        Ok(Self { port, partner_port, session, mock, tasks })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn info(&self) -> SenderInfo {
        SenderInfo {
            port: self.port,
            decks: self.mock.deck_count(),
            viewers: self.session.viewer_count(),
            demo: self.mock.demo(),
            partner_port: self.partner_port,
        }
    }

    /// Runs one mock command line, e.g. `play 1`. See `sources::mock`.
    pub fn command(&self, line: &str) -> Result<(), String> {
        self.mock.command(line)
    }

    /// Stops everything and waits until the port is free again.
    pub async fn stop(mut self) {
        for task in std::mem::take(&mut self.tasks) {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for MockSender {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
