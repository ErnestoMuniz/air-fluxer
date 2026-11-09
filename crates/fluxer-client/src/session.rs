//! Sans-IO gateway session: feed it frames and ticks, it says what to do. Mirrors the handshake in
//! `GatewaySocket.ts` (HELLO → IDENTIFY | RESUME, heartbeat with ACK tracking, RECONNECT,
//! INVALID_SESSION).

use crate::{Opcode, Payload, identify_flags};
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Send(Payload),
    /// Surface a dispatch (or gateway error) to the app.
    Event(Payload),
    /// Drop the socket and connect again; `resume` says whether the session is still valid.
    Reconnect,
}

pub struct Session {
    token: String,
    properties: Value,
    flags: u32,
    session_id: Option<String>,
    seq: Option<u64>,
    heartbeat_interval: Option<Duration>,
    awaiting_ack: bool,
}

impl Session {
    pub fn new(token: impl Into<String>, properties: Value) -> Self {
        Self {
            token: token.into(),
            properties,
            flags: identify_flags::DEBOUNCE_MESSAGE_REACTIONS | identify_flags::CHANNEL_THREADS,
            session_id: None,
            seq: None,
            heartbeat_interval: None,
            awaiting_ack: false,
        }
    }

    pub fn session_id(&self) -> Option<&str> { self.session_id.as_deref() }
    pub fn seq(&self) -> Option<u64> { self.seq }
    pub fn heartbeat_interval(&self) -> Option<Duration> { self.heartbeat_interval }

    /// Per-socket state resets on every new connection; the session id and sequence survive.
    pub fn on_connect(&mut self) {
        self.heartbeat_interval = None;
        self.awaiting_ack = false;
    }

    fn identify(&self) -> Payload {
        Payload {
            op: Opcode::IDENTIFY,
            d: Some(json!({ "token": self.token, "properties": self.properties, "flags": self.flags })),
            s: None,
            t: None,
        }
    }

    fn resume(&self, session_id: &str) -> Payload {
        Payload {
            op: Opcode::RESUME,
            d: Some(json!({ "token": self.token, "session_id": session_id, "seq": self.seq })),
            s: None,
            t: None,
        }
    }

    pub fn on_payload(&mut self, p: Payload) -> Vec<Action> {
        match p.op {
            Opcode::DISPATCH => {
                if let Some(s) = p.s { self.seq = Some(s) }
                match p.t.as_deref() {
                    Some("READY") => {
                        self.session_id = p.d.as_ref()
                            .and_then(|d| d.get("session_id")).and_then(Value::as_str).map(String::from);
                        self.awaiting_ack = false;
                    }
                    Some("RESUMED") => self.awaiting_ack = false,
                    _ => {}
                }
                vec![Action::Event(p)]
            }
            Opcode::HEARTBEAT => vec![Action::Send(Payload::heartbeat(self.seq))],
            Opcode::HEARTBEAT_ACK => { self.awaiting_ack = false; vec![] }
            Opcode::HELLO => {
                let ms = p.d.as_ref().and_then(|d| d.get("heartbeat_interval")).and_then(Value::as_u64);
                self.heartbeat_interval = ms.map(Duration::from_millis);
                let first = match &self.session_id {
                    Some(id) => self.resume(id),
                    None => self.identify(),
                };
                vec![Action::Send(first)]
            }
            Opcode::RECONNECT => vec![Action::Reconnect],
            Opcode::INVALID_SESSION => {
                if p.d.as_ref().and_then(Value::as_bool).unwrap_or(false) {
                    vec![Action::Reconnect]
                } else {
                    // The resume was rejected: forget the session and identify on this socket.
                    self.session_id = None;
                    self.seq = None;
                    vec![Action::Send(self.identify())]
                }
            }
            Opcode::GATEWAY_ERROR => vec![Action::Event(p)],
            _ => vec![],
        }
    }

    /// A heartbeat is due. Still waiting for the last ACK means the connection is a zombie.
    pub fn on_heartbeat_due(&mut self) -> Vec<Action> {
        if self.awaiting_ack { return vec![Action::Reconnect] }
        self.awaiting_ack = true;
        vec![Action::Send(Payload::heartbeat(self.seq))]
    }
}

/// Next heartbeat delay: 80% of the interval (at least 1 s) plus up to 5% jitter (at most 1.5 s).
pub fn heartbeat_delay(interval: Duration, jitter01: f64) -> Duration {
    let ms = interval.as_millis() as u64;
    let base = (ms * 8 / 10).max(1000);
    let jitter = (ms / 20).min(1500);
    Duration::from_millis(base + (jitter as f64 * jitter01.clamp(0.0, 1.0)) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(s: &str) -> Payload { Payload::decode(s).unwrap() }

    #[test]
    fn fresh_session_identifies_then_resumes_after_ready() {
        let mut s = Session::new("tok", json!({"os": "linux"}));
        let a = s.on_payload(frame(r#"{"op":10,"d":{"heartbeat_interval":40000}}"#));
        assert!(matches!(&a[0], Action::Send(p) if p.op == Opcode::IDENTIFY));
        s.on_payload(frame(r#"{"op":0,"s":5,"t":"READY","d":{"session_id":"abc"}}"#));
        assert_eq!((s.session_id(), s.seq()), (Some("abc"), Some(5)));
        s.on_connect();
        let a = s.on_payload(frame(r#"{"op":10,"d":{"heartbeat_interval":40000}}"#));
        let Action::Send(p) = &a[0] else { panic!() };
        assert_eq!(p.op, Opcode::RESUME);
        assert_eq!(p.d.as_ref().unwrap()["seq"], 5);
    }

    #[test]
    fn missing_ack_means_reconnect() {
        let mut s = Session::new("t", json!({}));
        assert!(matches!(&s.on_heartbeat_due()[0], Action::Send(p) if p.op == Opcode::HEARTBEAT));
        assert_eq!(s.on_heartbeat_due(), vec![Action::Reconnect]);
        s.on_payload(frame(r#"{"op":11}"#));
        assert!(matches!(&s.on_heartbeat_due()[0], Action::Send(_)));
    }

    #[test]
    fn rejected_resume_clears_the_session_and_identifies() {
        let mut s = Session::new("t", json!({}));
        s.on_payload(frame(r#"{"op":0,"s":2,"t":"READY","d":{"session_id":"x"}}"#));
        let a = s.on_payload(frame(r#"{"op":9,"d":false}"#));
        assert!(matches!(&a[0], Action::Send(p) if p.op == Opcode::IDENTIFY));
        assert_eq!(s.session_id(), None);
        assert_eq!(s.on_payload(frame(r#"{"op":9,"d":true}"#)), vec![Action::Reconnect]);
    }

    #[test]
    fn heartbeat_delay_follows_the_web_client() {
        let i = Duration::from_millis(41_250);
        assert_eq!(heartbeat_delay(i, 0.0), Duration::from_millis(33_000));
        assert_eq!(heartbeat_delay(Duration::from_millis(500), 0.0), Duration::from_millis(1000));
    }
}
