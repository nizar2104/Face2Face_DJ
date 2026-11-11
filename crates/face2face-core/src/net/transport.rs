//! Sender side of the transport: a WebSocket server that Viewers connect to
//! (brief 7.6). JSON text frames carry control and state, binary frames carry
//! waveform chunks. See `docs/PROTOCOL.md` for the handshake.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{Sink, SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinSet;
use tokio_tungstenite::tungstenite::{Error as WsError, Message as WsMessage};

use crate::protocol::{Message, Role, PROTOCOL_VERSION};
use crate::session::{Frame, Session};

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
/// A frame that cannot be written for this long means the Viewer is gone.
const SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// How this Sender introduces itself in `hello`.
#[derive(Debug, Clone)]
pub struct SenderIdentity {
    pub device_id: String,
    pub name: String,
    pub app_version: String,
    pub os: String,
}

/// Accepts Viewers until the task is aborted. Aborting it also closes every
/// Viewer connection, because they live in the `JoinSet` owned by this future.
pub async fn serve(listener: TcpListener, session: Arc<Session>, identity: Arc<SenderIdentity>) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    let session = Arc::clone(&session);
                    let identity = Arc::clone(&identity);
                    connections.spawn(async move {
                        if let Err(error) = handle_viewer(stream, session, identity).await {
                            tracing::debug!("viewer {peer} dropped: {error:#}");
                        }
                    });
                }
                Err(error) => {
                    tracing::warn!("accept failed: {error}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            // Reap finished connections so the set does not grow without bound.
            Some(_) = connections.join_next() => {}
        }
    }
}

async fn send<S>(sink: &mut S, message: WsMessage) -> anyhow::Result<()>
where
    S: Sink<WsMessage, Error = WsError> + Unpin,
{
    match tokio::time::timeout(SEND_TIMEOUT, sink.send(message)).await {
        Ok(result) => Ok(result?),
        Err(_) => anyhow::bail!("send timed out"),
    }
}

// The `.into()` calls below are identities with tungstenite 0.24 (String and
// Vec<u8> payloads) and real conversions with newer versions (Utf8Bytes, Bytes).
#[allow(clippy::useless_conversion)]
async fn send_json<S>(sink: &mut S, message: &Message) -> anyhow::Result<()>
where
    S: Sink<WsMessage, Error = WsError> + Unpin,
{
    send(sink, WsMessage::Text(message.to_json().into())).await
}

#[allow(clippy::useless_conversion)]
async fn send_frame<S>(sink: &mut S, frame: Frame) -> anyhow::Result<()>
where
    S: Sink<WsMessage, Error = WsError> + Unpin,
{
    let message = match frame {
        Frame::Text(text) => WsMessage::Text(text.to_string().into()),
        Frame::Binary(bytes) => WsMessage::Binary(bytes.to_vec().into()),
    };
    send(sink, message).await
}

async fn handle_viewer(
    stream: TcpStream,
    session: Arc<Session>,
    identity: Arc<SenderIdentity>,
) -> anyhow::Result<()> {
    stream.set_nodelay(true)?;
    let websocket = tokio_tungstenite::accept_async(stream).await?;
    let (mut sink, mut incoming) = websocket.split();

    // 1. Wait for the Viewer's hello. Pings are answered even before it.
    let deadline = tokio::time::sleep(HELLO_TIMEOUT);
    tokio::pin!(deadline);
    let viewer_name = loop {
        tokio::select! {
            _ = &mut deadline => anyhow::bail!("no hello within {HELLO_TIMEOUT:?}"),
            next = incoming.next() => {
                let Some(next) = next else { return Ok(()) };
                match next? {
                    WsMessage::Text(text) => match Message::parse(text.as_str()) {
                        Some(Message::Ping { t_viewer }) => {
                            send_json(&mut sink, &Message::Pong { t_viewer, t_sender: session.now() }).await?;
                        }
                        Some(Message::Hello { protocol, role: Role::Viewer, name, .. }) => {
                            if protocol != PROTOCOL_VERSION {
                                let refusal = Message::Error {
                                    code: "protocol_mismatch".to_string(),
                                    message: format!("Sender speaks protocol {PROTOCOL_VERSION}, Viewer sent {protocol}."),
                                };
                                send_json(&mut sink, &refusal).await?;
                                let _ = sink.close().await;
                                return Ok(());
                            }
                            break name;
                        }
                        _ => {} // malformed, unknown or out of place: ignore
                    },
                    WsMessage::Close(_) => return Ok(()),
                    _ => {}
                }
            }
        }
    };

    // 2. Introduce ourselves and bring the Viewer up to date.
    let hello = Message::Hello {
        protocol: PROTOCOL_VERSION,
        device_id: identity.device_id.clone(),
        name: identity.name.clone(),
        role: Role::Sender,
        app_version: identity.app_version.clone(),
        os: identity.os.clone(),
    };
    send_json(&mut sink, &hello).await?;
    let (snapshot, mut live) = session.subscribe();
    let _counted = session.viewer_guard();
    tracing::info!("viewer connected: {viewer_name}");
    for frame in snapshot {
        send_frame(&mut sink, frame).await?;
    }

    // 3. Stream: live frames out, pings in, a heartbeat every second.
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    loop {
        tokio::select! {
            frame = live.recv() => match frame {
                Ok(frame) => send_frame(&mut sink, frame).await?,
                Err(RecvError::Lagged(missed)) => anyhow::bail!("viewer fell {missed} frames behind"),
                Err(RecvError::Closed) => return Ok(()),
            },
            next = incoming.next() => {
                let Some(next) = next else { return Ok(()) };
                match next? {
                    WsMessage::Text(text) => {
                        if let Some(Message::Ping { t_viewer }) = Message::parse(text.as_str()) {
                            send_json(&mut sink, &Message::Pong { t_viewer, t_sender: session.now() }).await?;
                        }
                    }
                    WsMessage::Close(_) => return Ok(()),
                    _ => {}
                }
            }
            _ = heartbeat.tick() => {
                send_json(&mut sink, &Message::Heartbeat { ts: session.now() }).await?;
            }
        }
    }
}
