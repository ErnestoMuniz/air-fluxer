//! Bridges the async gateway runner into Rune's single-threaded state: events land in an
//! `std::sync::mpsc` inbox, the waker makes Rune run `poll`, and `poll` drains the inbox.

use fluxer_client::connection::{Config, Event, run};
use fluxer_client::session::Session;
use fluxer_client::{Compression, Payload};
use rune::Waker;
use serde_json::json;
use std::sync::mpsc;

pub struct Settings {
    pub gateway: String,
    pub token: String,
}

pub struct Handle {
    pub inbox: mpsc::Receiver<Event>,
    /// Frames for the server (here only subscriptions). Kept alive so the runner never sees a closed channel.
    pub outbound: tokio::sync::mpsc::Sender<Payload>,
}

impl Handle {
    /// Queues a frame; dropped when the queue is full (the next reconcile asks again).
    pub fn send(&self, p: Payload) -> bool { self.outbound.try_send(p).is_ok() }
}

pub fn start(settings: Settings, wake: Waker) -> Handle {
    let (tx, inbox) = mpsc::channel();
    let (out_tx, out_rx) = tokio::sync::mpsc::channel(16);
    std::thread::Builder::new().name("fluxer-gateway".into()).spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
        rt.block_on(async move {
            let (ev_tx, mut ev_rx) = tokio::sync::mpsc::channel(256);
            let cfg = Config { gateway: settings.gateway, api_version: 1, compression: Compression::ZstdStream };
            let props = json!({ "os": std::env::consts::OS, "browser": "fluxer-rune", "device": "fluxer-rune" });
            tokio::spawn(run(cfg, Session::new(settings.token, props), ev_tx, out_rx));
            while let Some(e) = ev_rx.recv().await {
                if tx.send(e).is_err() { return }
                wake();
            }
        });
    }).expect("spawn gateway thread");
    Handle { inbox, outbound: out_tx }
}
