// Face2Face DJ desktop shell. All logic lives in `face2face-core`; this file only
// exposes it to the UI as Tauri commands (see app/src/tauri.ts).

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use face2face_core::protocol::DEFAULT_PORT;
use face2face_core::sender::{MockSender, SenderInfo};
use tauri::State;
use tokio::sync::Mutex;

/// Milestone 0 decks: a 2-deck mock, with the scripted demo running.
const MOCK_DECKS: u8 = 2;

#[derive(Default)]
struct AppState {
    sender: Mutex<Option<MockSender>>,
}

/// "Share my decks": starts the Sender, or reports the one already running.
#[tauri::command]
async fn start_sender(state: State<'_, AppState>) -> Result<SenderInfo, String> {
    let mut sender = state.sender.lock().await;
    if let Some(running) = sender.as_ref() {
        return Ok(running.info());
    }
    let started = MockSender::start(DEFAULT_PORT, MOCK_DECKS, true)
        .await
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
    Ok(sender.as_ref().map(MockSender::info))
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

fn main() {
    tracing_subscriber::fmt::init();
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![start_sender, stop_sender, sender_status, mock_command])
        .run(tauri::generate_context!())
        .expect("error while running Face2Face DJ");
}
