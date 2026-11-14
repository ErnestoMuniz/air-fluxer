//! Sign-in state and the worker threads behind it. The UI thread never blocks: a job runs the
//! blocking REST calls off-thread, posts an [`Msg`] and wakes Rune, and `poll` drains them.

use fluxer_client::rest::{self, Credentials, Endpoints, LoginOutcome};
use rune::{TextEdit, Waker};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

pub const DEFAULT_INSTANCE: &str = "https://fluxer.com";

pub enum Step {
    Credentials,
    Code { ticket: String },
    /// Waiting for the user to open the e-mailed link; a poll thread signs in when they do.
    Waiting { ticket: String, email: String, resend_ready_at: Instant },
}

pub enum Msg {
    Signed { instance: String, ep: Endpoints, creds: Credentials },
    Mfa { instance: String, ep: Endpoints, ticket: String },
    Waiting { instance: String, ep: Endpoints, ticket: String, email: String, resend_in: u64 },
    Notice(String),
    Failed(String),
}

pub struct Auth {
    pub instance: TextEdit,
    pub identifier: TextEdit,
    pub password: TextEdit,
    pub code: TextEdit,
    pub step: Step,
    pub busy: bool,
    pub error: Option<String>,
    pub info: Option<String>,
    /// Stops the poll thread of the waiting step.
    cancel: Arc<AtomicBool>,
    /// Endpoints found for the instance in step one, reused by the code step.
    pub resolved: Option<(String, Endpoints)>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    wake: Option<Waker>,
}

impl Default for Auth {
    fn default() -> Self {
        let (tx, rx) = channel();
        Auth {
            instance: TextEdit::new(DEFAULT_INSTANCE),
            identifier: TextEdit::new(""),
            password: TextEdit::new(""),
            code: TextEdit::new(""),
            step: Step::Credentials,
            busy: false,
            error: None,
            info: None,
            cancel: Arc::new(AtomicBool::new(false)),
            resolved: None,
            tx,
            rx,
            wake: None,
        }
    }
}

fn describe(e: rest::Error) -> String {
    match e.code.as_deref() {
        Some("INVALID_CAPTCHA" | "CAPTCHA_REQUIRED") => "Não foi possível validar o captcha. Tente de novo.".into(),
        _ if e.status == Some(429) => "Muitas tentativas. Espere um pouco e tente de novo.".into(),
        _ => e.message,
    }
}

impl Auth {
    pub fn set_waker(&mut self, wake: Waker) { self.wake = Some(wake) }

    /// Stops polling for an IP authorization, if it is running.
    pub fn cancel_polling(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
    }

    pub fn back_to_credentials(&mut self) {
        self.cancel_polling();
        self.info = None;
        self.step = Step::Credentials;
        self.code = TextEdit::new("");
        self.error = None;
    }

    /// Starts the job for the current step. Returns false when a field is empty or one is running.
    pub fn submit(&mut self) -> bool {
        if self.busy { return false }
        let tx = self.tx.clone();
        let wake = self.wake.clone();
        let send = move |m: Msg| { let _ = tx.send(m); if let Some(w) = &wake { w() } };
        match &self.step {
            Step::Credentials => {
                let (instance, id, pw) = (self.instance.text.trim().to_string(), self.identifier.text.trim().to_string(), self.password.text.clone());
                if instance.is_empty() || id.is_empty() || pw.is_empty() {
                    self.error = Some("Preencha instância, usuário e senha.".into());
                    return false;
                }
                std::thread::spawn(move || {
                    let res = rest::discover(&instance).and_then(|ep| {
                        let out = rest::login(&ep, &id, &pw)?;
                        Ok((ep, out))
                    });
                    send(finish(instance, res));
                });
            }
            // The poll thread is already working on it.
            Step::Waiting { .. } => return false,
            Step::Code { ticket } => {
                let (ticket, code) = (ticket.clone(), self.code.text.trim().to_string());
                let Some((instance, ep)) = self.resolved.clone() else { return false };
                if code.is_empty() { self.error = Some("Digite o código de 6 dígitos.".into()); return false }
                std::thread::spawn(move || {
                    let res = rest::login_mfa_totp(&ep, &ticket, &code).map(|o| (ep, o));
                    send(finish(instance, res));
                });
            }
        }
        self.busy = true;
        self.error = None;
        true
    }

    fn sender(&self) -> impl Fn(Msg) + Send + 'static {
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        move |m| { let _ = tx.send(m); if let Some(w) = &wake { w() } }
    }

    /// Polls the ticket every two seconds and signs in as soon as the link was opened, so the user
    /// never has to type anything again (the web client does the same).
    pub fn start_polling(&mut self, ticket: String) {
        let Some((instance, ep)) = self.resolved.clone() else { return };
        self.cancel_polling();
        let (send, cancel) = (self.sender(), self.cancel.clone());
        std::thread::spawn(move || {
            let mut errors = 0;
            while !cancel.load(Ordering::Relaxed) {
                match rest::poll_ip_authorization(&ep, &ticket) {
                    Ok(Some(creds)) => {
                        if !cancel.load(Ordering::Relaxed) { send(Msg::Signed { instance, ep, creds }) }
                        return;
                    }
                    Ok(None) => errors = 0,
                    Err(_) => {
                        errors += 1;
                        if errors >= 3 {
                            send(Msg::Failed("Perdi a conexão enquanto esperava a autorização. Tente entrar de novo.".into()));
                            return;
                        }
                    }
                }
                for _ in 0..20 {
                    if cancel.load(Ordering::Relaxed) { return }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        });
    }

    pub fn resend(&mut self) {
        let Step::Waiting { ticket, resend_ready_at, .. } = &mut self.step else { return };
        let wait = resend_ready_at.saturating_duration_since(Instant::now()).as_secs();
        if wait > 0 { self.info = Some(format!("Aguarde {wait}s para reenviar.")); return }
        *resend_ready_at = Instant::now() + Duration::from_secs(30);
        let (ticket, send) = (ticket.clone(), self.sender());
        let Some((_, ep)) = self.resolved.clone() else { return };
        self.info = Some("Reenviando o e-mail…".into());
        std::thread::spawn(move || send(match rest::resend_ip_authorization(&ep, &ticket) {
            Ok(()) => Msg::Notice("E-mail reenviado.".into()),
            Err(e) => Msg::Failed(describe(e)),
        }));
    }

    pub fn drain(&mut self) -> Vec<Msg> {
        let msgs: Vec<Msg> = self.rx.try_iter().collect();
        if !msgs.is_empty() { self.busy = false }
        msgs
    }
}

fn finish(instance: String, res: Result<(Endpoints, LoginOutcome), rest::Error>) -> Msg {
    match res {
        Err(e) => Msg::Failed(describe(e)),
        Ok((ep, LoginOutcome::Signed(creds))) => Msg::Signed { instance, ep, creds },
        Ok((ep, LoginOutcome::Mfa { ticket, totp })) => {
            if totp { Msg::Mfa { instance, ep, ticket } }
            else { Msg::Failed("Esta conta usa uma chave de segurança (WebAuthn), que ainda não é suportada aqui.".into()) }
        }
        Ok((ep, LoginOutcome::IpAuthorization { ticket, email, resend_available_in })) =>
            Msg::Waiting { instance, ep, ticket, email, resend_in: resend_available_in },
    }
}

impl Drop for Auth {
    fn drop(&mut self) { self.cancel.store(true, Ordering::Relaxed) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// A server that answers each GET with the next canned JSON body (the last one again when they
    /// run out). A connection that closes without sending a request, as an HTTP client's spare
    /// connection may, takes no body.
    fn serve(bodies: Vec<&'static str>) -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            let mut next = 0;
            while let Ok((mut c, _)) = l.accept() {
                let mut buf = [0u8; 4096];
                if c.read(&mut buf).unwrap_or(0) == 0 { continue }
                let b = bodies[next.min(bodies.len() - 1)];
                next += 1;
                let _ = write!(c, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len());
            }
        });
        url
    }

    #[test]
    fn polling_signs_in_by_itself_once_the_link_is_opened() {
        let url = serve(vec![r#"{"completed":false}"#, r#"{"completed":true,"token":"tok","user_id":"9"}"#]);
        let mut a = Auth::default();
        a.resolved = Some(("https://i".into(), Endpoints { api_client: url, gateway: "wss://g".into(), media: None, origin: None }));
        a.start_polling("tk".into());
        let started = Instant::now();
        let msg = loop {
            if let Some(m) = a.drain().into_iter().next() { break m }
            assert!(started.elapsed() < Duration::from_secs(10), "no sign-in after the link was opened");
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(matches!(msg, Msg::Signed { creds, .. } if creds.token == "tok"));
    }

    #[test]
    fn cancelling_stops_the_poll() {
        let url = serve(vec![r#"{"completed":false}"#; 1]);
        let mut a = Auth::default();
        a.resolved = Some(("https://i".into(), Endpoints { api_client: url, gateway: "wss://g".into(), media: None, origin: None }));
        a.start_polling("tk".into());
        a.cancel_polling();
        std::thread::sleep(Duration::from_millis(400));
        assert!(a.drain().is_empty());
    }
}
