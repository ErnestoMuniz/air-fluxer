//! Async runner: connects, drives a [`Session`], reconnects with backoff.

use crate::compression::{Decoder, Encoder, LEVEL};
use crate::session::{Action, Session, heartbeat_delay};
use crate::{Compression, Payload, gateway_url, is_fatal_close};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug)]
pub enum Event {
    /// A DISPATCH (READY, MESSAGE_CREATE, ...) or GATEWAY_ERROR payload.
    Payload(Payload),
    Disconnected { code: Option<u16>, fatal: bool },
    /// The socket could not be opened (DNS, TLS, handshake refused...); it will be retried.
    ConnectFailed(String),
}

pub struct Config {
    pub gateway: String,
    pub api_version: u32,
    pub compression: Compression,
}

/// Runs until the app drops `events` or the server closes with a fatal code. `outbound` carries
/// frames the app sends (presence, voice state, ...).
pub async fn run(
    cfg: Config,
    mut session: Session,
    events: mpsc::Sender<Event>,
    mut outbound: mpsc::Receiver<Payload>,
) {
    let url = gateway_url(&cfg.gateway, cfg.api_version, cfg.compression);
    let mut backoff = Duration::from_millis(500);
    loop {
        let ws = match connect_async(url.as_str()).await {
            Ok((ws, _)) => ws,
            Err(e) => {
                if events.send(Event::ConnectFailed(e.to_string())).await.is_err() { return }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };
        session.on_connect();
        // One zstd stream per direction per socket.
        let zstd = cfg.compression == Compression::ZstdStream;
        let mut codec = if zstd { Encoder::new(LEVEL).ok().zip(Decoder::new().ok()) } else { None };
        if zstd && codec.is_none() { return }
        let (mut tx, mut rx) = ws.split();
        let mut next_beat: Option<Instant> = None;
        let mut close_code = None;
        'socket: loop {
            let beat = async {
                match next_beat { Some(t) => sleep_until(t).await, None => std::future::pending().await }
            };
            let mut actions = Vec::new();
            tokio::select! {
                msg = rx.next() => match msg {
                    Some(Ok(Message::Text(t))) => {
                        if let Ok(p) = Payload::decode(&t) { actions = session.on_payload(p) }
                    }
                    Some(Ok(Message::Binary(b))) => {
                        let Some((_, dec)) = codec.as_mut() else { continue };
                        match dec.decompress(&b) {
                            Ok(chunk) if chunk.is_empty() => {}
                            Ok(chunk) => {
                                if let Some(p) = std::str::from_utf8(&chunk).ok().and_then(|t| Payload::decode(t).ok()) {
                                    actions = session.on_payload(p);
                                }
                            }
                            // A broken stream cannot be resynchronised: reconnect.
                            Err(_) => break 'socket,
                        }
                    }
                    Some(Ok(Message::Close(f))) => { close_code = f.map(|f| u16::from(f.code)); break 'socket }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => break 'socket,
                },
                _ = beat => actions = session.on_heartbeat_due(),
                Some(p) = outbound.recv() => actions = vec![Action::Send(p)],
            }
            for a in actions {
                match a {
                    Action::Send(p) => {
                        let text = p.encode();
                        let msg = match codec.as_mut() {
                            Some((enc, _)) => match enc.compress(text.as_bytes()) {
                                Ok(b) => Message::binary(b),
                                Err(_) => break 'socket,
                            },
                            None => Message::text(text),
                        };
                        if tx.send(msg).await.is_err() { break 'socket }
                    }
                    Action::Event(p) => {
                        backoff = Duration::from_millis(500);
                        if events.send(Event::Payload(p)).await.is_err() { return }
                    }
                    Action::Reconnect => break 'socket,
                }
            }
            if let Some(i) = session.heartbeat_interval() {
                // Rescheduled after every wake; fine because any frame only postpones by a little.
                next_beat.get_or_insert_with(|| Instant::now() + heartbeat_delay(i, jitter()));
                if next_beat.is_some_and(|t| t <= Instant::now()) {
                    next_beat = Some(Instant::now() + heartbeat_delay(i, jitter()));
                }
            }
        }
        let fatal = close_code.is_some_and(is_fatal_close);
        if events.send(Event::Disconnected { code: close_code, fatal }).await.is_err() || fatal { return }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

fn jitter() -> f64 {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    f64::from(n % 1000) / 1000.0
}
