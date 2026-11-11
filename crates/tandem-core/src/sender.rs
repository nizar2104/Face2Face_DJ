//! The Milestone 0 Sender, assembled: mock decks -> session -> WebSocket server.
//!
//! The desktop shell (`app/src-tauri`) starts one of these when the user picks
//! "Share my decks". Real sources replace the mock in Milestones 3 and 4.

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
}

pub struct MockSender {
    port: u16,
    session: Arc<Session>,
    mock: MockHandle,
    tasks: Vec<JoinHandle<()>>,
}

fn identity() -> SenderIdentity {
    // A random, persistent device id arrives with pairing in Milestone 1.
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    SenderIdentity {
        device_id: format!("sender-{:x}-{:x}", std::process::id(), nanos),
        name: "Tandem Sender (mock decks)".to_string(),
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
            .with_context(|| format!("could not listen on 127.0.0.1:{port} (is another Tandem Sender running?)"))?;
        let port = listener.local_addr()?.port();

        let session = Session::new();
        let source = MockSource::new(deck_count, Some(DEMO_SESSION), demo).map_err(anyhow::Error::msg)?;
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
            tokio::spawn(serve(listener, Arc::clone(&session), Arc::new(identity()))),
        ];
        tracing::info!("mock sender listening on 127.0.0.1:{port}");
        Ok(Self { port, session, mock, tasks })
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
