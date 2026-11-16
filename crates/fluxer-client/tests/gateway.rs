//! Drives the real runner against an in-process mock gateway.

use fluxer_client::compression::{Decoder, Encoder, LEVEL};
use fluxer_client::connection::{Config, Event, run};
use fluxer_client::session::Session;
use fluxer_client::{Compression, Opcode, Payload};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

type Ws = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

/// The server side of one socket, with its own pair of zstd streams when compression is on.
struct Peer { ws: Ws, codec: Option<(Encoder, Decoder)> }

impl Peer {
    async fn accept(l: &TcpListener, zstd: bool) -> Peer {
        // Other local processes sometimes probe the port (a `HEAD /` health check, say): those
        // connections are not ours to serve, so keep listening for the client's handshake.
        loop {
            let (s, _) = l.accept().await.unwrap();
            if let Ok(ws) = accept_async(s).await {
                let codec = zstd.then(|| (Encoder::new(LEVEL).unwrap(), Decoder::new().unwrap()));
                return Peer { ws, codec };
            }
        }
    }
    async fn send(&mut self, v: serde_json::Value) {
        let text = v.to_string();
        let msg = match &mut self.codec {
            Some((e, _)) => Message::binary(e.compress(text.as_bytes()).unwrap()),
            None => Message::text(text),
        };
        self.ws.send(msg).await.unwrap()
    }
    async fn recv(&mut self) -> Payload {
        loop {
            match self.ws.next().await.unwrap().unwrap() {
                Message::Text(t) => return Payload::decode(&t).unwrap(),
                Message::Binary(b) => {
                    let (_, d) = self.codec.as_mut().expect("binary frame without compression");
                    return Payload::decode(std::str::from_utf8(&d.decompress(&b).unwrap()).unwrap()).unwrap();
                }
                _ => {}
            }
        }
    }
}

#[tokio::test]
async fn plain_json_identify_heartbeat_then_resume() { scenario(Compression::None).await }

#[tokio::test]
async fn zstd_stream_identify_heartbeat_then_resume() { scenario(Compression::ZstdStream).await }

async fn scenario(compression: Compression) {
    let zstd = compression == Compression::ZstdStream;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        // Connection 1: HELLO → IDENTIFY → READY → heartbeat/ACK → RECONNECT.
        let mut ws = Peer::accept(&listener, zstd).await;
        ws.send(json!({"op": 10, "d": {"heartbeat_interval": 1000}})).await;
        let id = ws.recv().await;
        assert_eq!(id.op, Opcode::IDENTIFY);
        assert_eq!(id.d.unwrap()["token"], "secret");
        ws.send(json!({"op": 0, "s": 1, "t": "READY", "d": {"session_id": "s1"}})).await;
        let hb = ws.recv().await;
        assert_eq!((hb.op, hb.d), (Opcode::HEARTBEAT, Some(json!(1))));
        ws.send(json!({"op": 11})).await;
        ws.send(json!({"op": 7})).await;
        drop(ws);

        // Connection 2: the client must RESUME with the session id and last sequence.
        let mut ws = Peer::accept(&listener, zstd).await;
        ws.send(json!({"op": 10, "d": {"heartbeat_interval": 1000}})).await;
        let resume = ws.recv().await;
        assert_eq!(resume.op, Opcode::RESUME);
        let d = resume.d.unwrap();
        assert_eq!((d["session_id"].as_str(), d["seq"].as_u64()), (Some("s1"), Some(1)));
        ws.send(json!({"op": 0, "s": 2, "t": "RESUMED", "d": {}})).await;
        // Keep the socket open until the test finishes.
        let _ = ws.ws.next().await;
    });

    let (ev_tx, mut ev_rx) = mpsc::channel(16);
    let (_out_tx, out_rx) = mpsc::channel(4);
    let cfg = Config { gateway: format!("ws://{addr}/"), api_version: 1, compression };
    let client = tokio::spawn(run(cfg, Session::new("secret", json!({"os": "linux"})), ev_tx, out_rx));

    let mut seen = Vec::new();
    while let Ok(Some(e)) = timeout(Duration::from_secs(10), ev_rx.recv()).await {
        if let Event::Payload(p) = &e { seen.push(p.t.clone().unwrap_or_default()) }
        if seen.last().map(String::as_str) == Some("RESUMED") { break }
    }
    assert_eq!(seen, ["READY", "RESUMED"]);
    client.abort();
    server.abort();
}
