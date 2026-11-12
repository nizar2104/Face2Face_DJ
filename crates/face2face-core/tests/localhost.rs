//! Integration: a WebSocket client playing the Viewer against the real Sender
//! stack (mock decks -> session -> transport) on localhost.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use face2face_core::protocol::chunk::{decode_chunk, WaveformKind};
use face2face_core::protocol::{Message, Role, PROTOCOL_VERSION};
use face2face_core::sender::{Access, Sender};
use face2face_core::sources::SourceKind;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

const WAIT: Duration = Duration::from_secs(5);

async fn connect(port: u16) -> Client {
    let (client, _response) = connect_async(format!("ws://127.0.0.1:{port}")).await.expect("connects");
    client
}

#[allow(clippy::useless_conversion)] // identity with tungstenite 0.24, a conversion with newer versions
async fn send(client: &mut Client, message: &Message) {
    client.send(WsMessage::Text(message.to_json().into())).await.expect("sends");
}

/// The next frame, or a panic after `WAIT`. Text frames are parsed.
async fn next(client: &mut Client) -> Result<Message, Vec<u8>> {
    loop {
        let frame = tokio::time::timeout(WAIT, client.next())
            .await
            .expect("a frame arrives in time")
            .expect("the connection is open")
            .expect("the frame is valid");
        match frame {
            WsMessage::Text(text) => return Ok(Message::parse(text.as_str()).expect("the Sender sends valid messages")),
            WsMessage::Binary(bytes) => return Err(bytes.to_vec()),
            _ => continue,
        }
    }
}

/// Reads frames until `wanted` returns something, skipping the rest.
async fn until<T>(client: &mut Client, mut wanted: impl FnMut(&Result<Message, Vec<u8>>) -> Option<T>) -> T {
    loop {
        let frame = next(client).await;
        if let Some(found) = wanted(&frame) {
            return found;
        }
    }
}

fn viewer_hello(protocol: u32) -> Message {
    Message::Hello {
        protocol,
        device_id: "test-viewer".to_string(),
        name: "Test Viewer".to_string(),
        role: Role::Viewer,
        app_version: "0.1.0".to_string(),
        os: "test".to_string(),
        pair_code: None,
    }
}

#[tokio::test]
async fn a_viewer_gets_snapshot_waveform_live_state_and_pongs() {
    let sender = Sender::start_mock(0, 2, false).await.expect("sender starts");
    sender.command("load 1 1").expect("load");

    let mut client = connect(sender.port()).await;
    send(&mut client, &viewer_hello(PROTOCOL_VERSION)).await;

    // Handshake and snapshot, in the documented order.
    assert!(matches!(next(&mut client).await, Ok(Message::Hello { role: Role::Sender, protocol: PROTOCOL_VERSION, .. })));
    assert!(matches!(next(&mut client).await, Ok(Message::SourceStatus { decks: 2, connected: true, .. })));
    let (frames, waveform_id) = match next(&mut client).await {
        Ok(Message::DeckLoaded { deck: 1, track, waveform: Some(info) }) => {
            assert_eq!(track.title, "Neon Tide");
            assert_eq!(track.bpm_original, 124.0);
            (info.frames as usize, info.waveform_id)
        }
        other => panic!("expected deck_loaded, got {other:?}"),
    };
    assert_eq!(frames, 55_860);

    // Waveform: overview first, then the full detail.
    let mut overview = 0usize;
    let mut detail = 0usize;
    while detail < frames * 3 {
        let bytes = next(&mut client).await.expect_err("binary frames follow deck_loaded");
        let chunk = decode_chunk(&bytes).expect("valid chunk");
        assert_eq!(chunk.waveform_id, waveform_id);
        match chunk.kind {
            WaveformKind::Overview => {
                assert_eq!(detail, 0, "overview must come first");
                assert_eq!(chunk.offset as usize, overview);
                overview += chunk.payload.len();
            }
            WaveformKind::Detail => {
                assert_eq!(chunk.offset as usize, detail);
                detail += chunk.payload.len();
            }
        }
    }
    assert_eq!(overview, 3600);
    assert!(matches!(next(&mut client).await, Ok(Message::DeckState { deck: 1, playing: false, .. })));
    assert_eq!(sender.info().viewers, 1);

    // Live: play, then positions that advance and timestamps that increase.
    sender.command("play 1").expect("play");
    let mut playing = |frame: &Result<Message, Vec<u8>>| match frame {
        Ok(Message::DeckState { deck: 1, playing: true, is_master: true, ts, pos_s, .. }) => Some((*ts, *pos_s)),
        _ => None,
    };
    let (ts_a, pos_a) = until(&mut client, &mut playing).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    sender.command("pitch 1 10").expect("pitch");
    let (ts_b, pos_b, bpm_b) = until(&mut client, |frame| match frame {
        Ok(Message::DeckState { deck: 1, ts, pos_s, bpm_current, .. }) if (*bpm_current - 136.4).abs() < 1e-6 => {
            Some((*ts, *pos_s, *bpm_current))
        }
        _ => None,
    })
    .await;
    assert!(ts_b > ts_a && pos_b > pos_a, "{ts_a}->{ts_b}, {pos_a}->{pos_b}");
    assert!((bpm_b - 136.4).abs() < 1e-6);

    // Clock sync: a ping is answered with our timestamp echoed.
    send(&mut client, &Message::Ping { t_viewer: 12.5 }).await;
    let t_sender = until(&mut client, |frame| match frame {
        Ok(Message::Pong { t_viewer, t_sender }) if *t_viewer == 12.5 => Some(*t_sender),
        _ => None,
    })
    .await;
    assert!(t_sender >= ts_b);

    // Eject reaches the Viewer; an invalid command is an error, not a crash.
    sender.command("eject 1").expect("eject");
    until(&mut client, |frame| matches!(frame, Ok(Message::DeckUnloaded { deck: 1 })).then_some(())).await;
    assert!(sender.command("play 1").is_err());
    assert!(sender.command("warp 9").is_err());

    // Heartbeats keep coming while nothing plays.
    until(&mut client, |frame| matches!(frame, Ok(Message::Heartbeat { .. })).then_some(())).await;

    sender.stop().await;
}

#[tokio::test]
async fn garbage_and_wrong_versions_are_handled() {
    let sender = Sender::start_mock(0, 2, false).await.expect("sender starts");
    let mut client = connect(sender.port()).await;

    // Garbage is ignored; a ping before hello is still answered.
    client.send(WsMessage::Text("not json".to_string().into())).await.expect("sends");
    client.send(WsMessage::Binary(vec![1u8, 2, 3].into())).await.expect("sends");
    send(&mut client, &Message::Ping { t_viewer: 1.0 }).await;
    assert!(matches!(next(&mut client).await, Ok(Message::Pong { .. })));

    // A Viewer from the future is told why it is refused.
    send(&mut client, &viewer_hello(99)).await;
    match next(&mut client).await {
        Ok(Message::Error { code, .. }) => assert_eq!(code, "protocol_mismatch"),
        other => panic!("expected an error, got {other:?}"),
    }
    assert_eq!(sender.info().viewers, 0);

    // The Sender is still fine for the next Viewer.
    let mut second = connect(sender.port()).await;
    send(&mut second, &viewer_hello(PROTOCOL_VERSION)).await;
    assert!(matches!(next(&mut second).await, Ok(Message::Hello { .. })));

    sender.stop().await;
}

#[tokio::test]
async fn the_port_is_free_again_after_stop() {
    let sender = Sender::start_mock(0, 2, false).await.expect("sender starts");
    let port = sender.port();
    assert!(Sender::start_mock(port, 2, false).await.is_err(), "port is taken while running");
    sender.stop().await;
    let again = Sender::start_mock(port, 2, false).await.expect("port is free after stop");
    again.stop().await;
}

/// Two free loopback ports, found by binding and releasing them.
async fn two_free_ports() -> [u16; 2] {
    let a = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let b = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("binds");
    [a.local_addr().expect("has an address").port(), b.local_addr().expect("has an address").port()]
}

/// The title of the track on deck 1, as a newly connected Viewer is told it.
async fn deck_1_title(port: u16) -> (String, String) {
    let mut client = connect(port).await;
    send(&mut client, &viewer_hello(PROTOCOL_VERSION)).await;
    let sender_name = until(&mut client, |frame| match frame {
        Ok(Message::Hello { name, .. }) => Some(name.clone()),
        _ => None,
    })
    .await;
    let title = until(&mut client, |frame| match frame {
        Ok(Message::DeckLoaded { deck: 1, track, .. }) => Some(track.title.clone()),
        _ => None,
    })
    .await;
    (sender_name, title)
}

#[tokio::test]
async fn two_linked_senders_take_one_port_each_and_point_at_each_other() {
    let ports = two_free_ports().await;
    let a = Sender::start_link(ports, SourceKind::Mock, Access::ThisComputer).await.expect("the first side starts");
    let b = Sender::start_link(ports, SourceKind::Mock, Access::ThisComputer).await.expect("the second side starts");
    assert_eq!((a.port(), a.info().partner_port), (ports[0], Some(ports[1])));
    assert_eq!((b.port(), b.info().partner_port), (ports[1], Some(ports[0])));
    assert!(Sender::start_link(ports, SourceKind::Mock, Access::ThisComputer).await.is_err(), "there is no third side");

    // Each side is a full Sender, and the two show different tracks.
    let (name_a, title_a) = deck_1_title(a.port()).await;
    let (name_b, title_b) = deck_1_title(b.port()).await;
    assert_eq!(name_a, "Laptop A (mock decks)");
    assert_eq!(name_b, "Laptop B (mock decks)");
    assert_ne!(title_a, title_b);

    // When one side leaves, its port is free for the next one to link.
    a.stop().await;
    let again = Sender::start_link(ports, SourceKind::Mock, Access::ThisComputer).await.expect("the freed side can be taken again");
    assert_eq!((again.port(), again.info().partner_port), (ports[0], Some(ports[1])));
    again.stop().await;
    b.stop().await;
}

#[tokio::test]
async fn a_plain_sender_has_no_partner() {
    let sender = Sender::start_mock(0, 2, false).await.expect("sender starts");
    assert_eq!(sender.info().partner_port, None);
    sender.stop().await;
}

/// The first text frame, whatever it is.
async fn first_reply(port: u16, pair_code: Option<&str>) -> Message {
    let mut client = connect(port).await;
    let mut hello = viewer_hello(PROTOCOL_VERSION);
    if let Message::Hello { pair_code: code, .. } = &mut hello {
        *code = pair_code.map(str::to_string);
    }
    send(&mut client, &hello).await;
    next(&mut client).await.expect("a text frame")
}

#[tokio::test]
async fn a_paired_sender_admits_only_viewers_with_its_code() {
    let access = Access::Paired { code: "483920".to_string() };
    let sender = Sender::start(0, SourceKind::Mock, access).await.expect("sender starts");

    assert!(matches!(first_reply(sender.port(), Some("483920")).await, Message::Hello { role: Role::Sender, .. }));
    for wrong in [None, Some("000000")] {
        match first_reply(sender.port(), wrong).await {
            Message::Error { code, .. } => assert_eq!(code, "pairing_failed"),
            other => panic!("a Viewer with code {wrong:?} got {other:?}"),
        }
    }
    // Wrong guesses did not lock the partner out.
    assert!(matches!(first_reply(sender.port(), Some("483920")).await, Message::Hello { .. }));
    sender.stop().await;
}

#[tokio::test]
async fn a_sender_on_the_network_needs_a_real_code() {
    let short = Access::Paired { code: "123".to_string() };
    assert!(Sender::start(0, SourceKind::Mock, short).await.is_err());
}

#[tokio::test]
async fn a_viewer_that_goes_silent_is_dropped() {
    let sender = Sender::start_mock(0, 2, false).await.expect("sender starts");
    let mut client = connect(sender.port()).await;
    send(&mut client, &viewer_hello(PROTOCOL_VERSION)).await;
    assert!(matches!(next(&mut client).await, Ok(Message::Hello { .. })));
    assert_eq!(sender.info().viewers, 1);

    // The connection stays open, but the Viewer never pings: a laptop that
    // went to sleep. A real Viewer pings every 2 s.
    let dropped = tokio::time::timeout(Duration::from_secs(12), async {
        while sender.info().viewers > 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(dropped.is_ok(), "the Sender still counts the silent Viewer");
    drop(client);
    sender.stop().await;
}

#[tokio::test]
async fn a_connection_that_never_speaks_websocket_is_closed() {
    use tokio::io::AsyncReadExt;
    let sender = Sender::start_mock(0, 2, false).await.expect("sender starts");
    let mut raw = TcpStream::connect(("127.0.0.1", sender.port())).await.expect("connects");
    let mut byte = [0u8; 1];
    // Says nothing; the Sender hangs up (read returns 0 bytes or an error).
    let closed = tokio::time::timeout(Duration::from_secs(10), raw.read(&mut byte)).await;
    assert!(matches!(closed, Ok(Ok(0)) | Ok(Err(_))), "still open: {closed:?}");
    sender.stop().await;
}

