// Face2Face DJ desktop shell. All logic lives in `face2face-core`; this file only
// exposes it to the UI as Tauri commands (see app/src/tauri.ts).

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use face2face_core::protocol::{DEFAULT_PORT, LINK_PORTS};
use face2face_core::net::addresses::{local_addresses, LocalAddress};
use face2face_core::sender::{Access, Sender, SenderInfo};
use face2face_core::sources::SourceKind;
use tauri::State;
use tokio::sync::Mutex;

#[derive(Default)]
struct AppState {
    sender: Mutex<Option<Sender>>,
}

/// Starts the Sender reading `source`, or reports the one already running.
/// A Sender running with a different source is stopped first: that is how the
/// UI switches source.
/// `link` is "Link up" (two-way): take whichever of the two link ports is free
/// and tell the UI where a partner on this computer would be. Without it, "Share only".
/// `code` is this laptop's pairing code: the Sender is reachable from the
/// network, by Viewers that present it.
#[tauri::command]
async fn start_sender(
    state: State<'_, AppState>,
    link: bool,
    source: SourceKind,
    code: String,
) -> Result<SenderInfo, String> {
    let mut sender = state.sender.lock().await;
    if let Some(running) = sender.take() {
        let info = running.info();
        if info.source == source {
            *sender = Some(running);
            return Ok(info);
        }
        running.stop().await;
    }
    let access = Access::Paired { code };
    let started = if link {
        Sender::start_link(LINK_PORTS, source, access).await
    } else {
        Sender::start(DEFAULT_PORT, source, access).await
    }
    .map_err(|error| format!("{error:#}"))?;
    let info = started.info();
    *sender = Some(started);
    Ok(info)
}

#[tauri::command]
async fn stop_sender(state: State<'_, AppState>) -> Result<(), String> {
    let running = state.sender.lock().await.take();
    if let Some(running) = running {
        running.stop().await;
    }
    Ok(())
}

#[tauri::command]
async fn sender_status(state: State<'_, AppState>) -> Result<Option<SenderInfo>, String> {
    let sender = state.sender.lock().await;
    Ok(sender.as_ref().map(Sender::info))
}

/// Runs one mock-deck command, e.g. "play 1" (vocabulary in face2face-core's `sources::mock`).
#[tauri::command]
async fn mock_command(state: State<'_, AppState>, command: String) -> Result<(), String> {
    let sender = state.sender.lock().await;
    match sender.as_ref() {
        Some(sender) => sender.command(&command),
        None => Err("The Sender is not running.".to_string()),
    }
}

/// This laptop's addresses, for the partner to type in.
#[tauri::command]
fn network_addresses() -> Vec<LocalAddress> {
    local_addresses()
}

fn main() {
    tracing_subscriber::fmt::init();
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![start_sender, stop_sender, sender_status, mock_command, network_addresses])
        .run(tauri::generate_context!())
        .expect("error while running Face2Face DJ");
}
