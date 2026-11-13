//! A Sender without a window, for trying a reader from a terminal:
//!
//! ```sh
//! cargo run --example sender -- traktor      # or rekordbox, pro_dj_link, mixxx, mock
//! ```
//!
//! It listens where the desktop app does (ws://127.0.0.1:47801), so
//! `npm run dev` plus "View only" in a browser shows what a partner would see.
//! Each change in what the reader reports is printed.
//!
//! A second argument is a pairing code, and makes the Sender reachable from
//! other computers that present it; a third is the port:
//!
//! ```sh
//! cargo run --example sender -- mock 483920 47811
//! ```

use std::time::Duration;

use face2face_core::protocol::DEFAULT_PORT;
use face2face_core::sender::{Access, Sender};
use face2face_core::sources::SourceKind;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let name = std::env::args().nth(1).unwrap_or_else(|| "mock".to_string());
    let Ok(source) = serde_json::from_value::<SourceKind>(name.clone().into()) else {
        anyhow::bail!("unknown source {name:?}: use mock, traktor, rekordbox, pro_dj_link or mixxx");
    };
    let access = match std::env::args().nth(2) {
        Some(code) => Access::Paired { code },
        None => Access::ThisComputer,
    };
    let port = match std::env::args().nth(3) {
        Some(port) => port.parse()?,
        None => DEFAULT_PORT,
    };
    let reach = if access == Access::ThisComputer { "this computer only" } else { "the network, with the pairing code" };
    let sender = Sender::start(port, source, access).await?;
    println!("Sender on port {}, open to {reach}, reading {name}. Ctrl+C stops it.", sender.port());
    if reach.starts_with("the network") {
        for local in face2face_core::net::addresses::local_addresses() {
            println!("This computer: {} ({})", local.address, local.interface);
        }
    }

    let mut last = None;
    loop {
        let info = sender.info();
        let line = format!(
            "{} · {} decks · {} · {} viewer(s)",
            if info.connected { "connected" } else { "not connected" },
            info.decks,
            info.reader_status,
            info.viewers
        );
        if last.as_ref() != Some(&line) {
            println!("{line}");
            last = Some(line);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
