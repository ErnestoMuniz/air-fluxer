//! Blocking REST for sign-in: instance discovery (`/.well-known/fluxer`), `/auth/login`, TOTP
//! second step and the altcha retry. Call from a worker thread, never from the UI thread.

use crate::altcha;
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    /// Where API requests go: `api_transport` when the instance publishes one (what the desktop
    /// client uses), otherwise `api_client`.
    pub api_client: String,
    pub gateway: String,
    /// Base URL of the media proxy (avatars, icons, attachments).
    pub media: Option<String>,
    /// Origin of the instance's web app. The API only accepts non-GET requests from its own web
    /// origins, so the desktop client sends this as `Origin` and so do we.
    pub origin: Option<String>,
}

/// `scheme://host[:port]` of a URL, or None when it is not http(s).
pub fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme != "https" && scheme != "http" { return None }
    let host = rest.split(['/', '?', '#']).next().filter(|h| !h.is_empty())?;
    Some(format!("{scheme}://{host}"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub token: String,
    pub user_id: String,
    pub username: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginOutcome {
    Signed(Credentials),
    /// The account has a second factor; finish with [`login_mfa_totp`].
    Mfa { ticket: String, totp: bool },
    /// The server wants this IP approved through an e-mailed link. Poll `ticket` with
    /// [`poll_ip_authorization`]; the answer carries the token once the link was opened.
    IpAuthorization { ticket: String, email: String, resend_available_in: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub status: Option<u16>,
    pub code: Option<String>,
    pub message: String,
}

impl Error {
    pub(crate) fn local(message: impl Into<String>) -> Self { Self { status: None, code: None, message: message.into() } }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.message) }
}
impl std::error::Error for Error {}

pub(crate) fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .user_agent("fluxer-rune")
        .build()
        .into()
}

/// `https://host` or `host`; the trailing slash goes.
pub fn normalize_instance(input: &str) -> String {
    let t = input.trim().trim_end_matches('/');
    if t.contains("://") { t.to_string() } else { format!("https://{t}") }
}

pub(crate) fn get_json(agent: &ureq::Agent, url: &str) -> Result<(u16, Value), Error> {
    let mut res = agent.get(url).header("Accept", "application/json").call()
        .map_err(|e| Error::local(format!("sem conexão com {url}: {e}")))?;
    let status = res.status().as_u16();
    Ok((status, res.body_mut().read_json().unwrap_or(Value::Null)))
}

/// Finds the API and gateway URLs of an instance. The document lives at the instance root for
/// API hosts and under `/api` for the web host.
pub fn discover(instance: &str) -> Result<Endpoints, Error> {
    let base = normalize_instance(instance);
    let agent = agent();
    let mut last = Error::local("instância não respondeu");
    for path in ["/.well-known/fluxer", "/api/.well-known/fluxer"] {
        match get_json(&agent, &format!("{base}{path}")) {
            Ok((200, v)) => {
                let ep = v.get("endpoints");
                let get = |k: &str| ep.and_then(|e| e.get(k)).and_then(Value::as_str).map(String::from);
                let api = get("api_transport").or_else(|| get("api_client")).or_else(|| get("api"));
                if let (Some(api), Some(gateway)) = (api, get("gateway")) {
                    let origin = get("webapp").and_then(|w| origin_of(&w)).or_else(|| origin_of(&base));
                    let media = get("media").map(|m| m.trim_end_matches('/').to_string());
                    return Ok(Endpoints { api_client: api.trim_end_matches('/').into(), gateway, media, origin });
                }
                last = Error::local("a instância não publicou os endpoints");
            }
            Ok((s, _)) => last = Error { status: Some(s), code: None, message: format!("a instância respondeu {s}") },
            Err(e) => last = e,
        }
    }
    Err(last)
}

pub(crate) struct Reply { pub status: u16, pub body: Value }

pub(crate) fn post(agent: &ureq::Agent, url: &str, origin: Option<&str>, body: &Value, captcha: Option<&str>) -> Result<Reply, Error> {
    let mut req = agent.post(url).header("Accept", "application/json");
    if let Some(o) = origin { req = req.header("Origin", o) }
    if let Some(t) = captcha { req = req.header("X-Captcha-Token", t) }
    let mut res = req.send_json(body).map_err(|e| Error::local(format!("falha de rede: {e}")))?;
    let status = res.status().as_u16();
    Ok(Reply { status, body: res.body_mut().read_json().unwrap_or(Value::Null) })
}

fn code_of(body: &Value) -> Option<&str> { body.get("code").and_then(Value::as_str) }

/// Posts, and while the server answers with an altcha challenge, solves it and retries (twice at
/// most, like `CaptchaInterceptor.ts`).
fn post_with_captcha(agent: &ureq::Agent, url: &str, origin: Option<&str>, body: &Value) -> Result<Reply, Error> {
    let mut reply = post(agent, url, origin, body, None)?;
    for _ in 0..2 {
        let wants = reply.status == 400 && matches!(code_of(&reply.body), Some("CAPTCHA_REQUIRED" | "INVALID_CAPTCHA"));
        let Some(challenge) = reply.body.get("altcha_challenge").filter(|_| wants) else { break };
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get().min(8));
        let token = altcha::solve(challenge, threads, Duration::from_secs(60)).map_err(|e| Error::local(e.to_string()))?;
        reply = post(agent, url, origin, body, Some(&token))?;
    }
    Ok(reply)
}

pub(crate) fn failure(r: &Reply) -> Error {
    Error {
        status: Some(r.status),
        code: code_of(&r.body).map(String::from),
        message: r.body.get("message").and_then(Value::as_str).map_or_else(|| format!("erro {}", r.status), String::from),
    }
}

fn credentials(body: &Value) -> Option<Credentials> {
    Some(Credentials {
        token: body.get("token")?.as_str()?.into(),
        user_id: body.get("user_id")?.as_str()?.into(),
        username: body.pointer("/user/username").and_then(Value::as_str).map(String::from),
    })
}

fn outcome(r: Reply) -> Result<LoginOutcome, Error> {
    if r.status == 403 && code_of(&r.body) == Some("IP_AUTHORIZATION_REQUIRED") {
        // The challenge sits in `details.context`, or flat in the body (`ResponseInspection.ts`).
        let ctx = r.body.pointer("/details/context").filter(|c| c.is_object()).unwrap_or(&r.body);
        let text = |k: &str| ctx.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(String::from);
        return match (text("ticket"), text("email")) {
            (Some(ticket), Some(email)) => Ok(LoginOutcome::IpAuthorization {
                ticket, email, resend_available_in: ctx.get("resend_available_in").and_then(Value::as_u64).unwrap_or(30),
            }),
            _ => Err(Error::local("o servidor pediu autorização de IP, mas a resposta veio incompleta")),
        };
    }
    if !(200..300).contains(&r.status) { return Err(failure(&r)) }
    if r.body.get("mfa").and_then(Value::as_bool) == Some(true) {
        let ticket = r.body.get("ticket").and_then(Value::as_str).ok_or_else(|| Error::local("resposta de MFA sem ticket"))?;
        return Ok(LoginOutcome::Mfa { ticket: ticket.into(), totp: r.body.get("totp").and_then(Value::as_bool).unwrap_or(false) });
    }
    credentials(&r.body).map(LoginOutcome::Signed).ok_or_else(|| Error::local("resposta de login inesperada"))
}

/// `identifier` is an e-mail when it has an `@`, otherwise a username.
pub fn login(ep: &Endpoints, identifier: &str, password: &str) -> Result<LoginOutcome, Error> {
    let key = if identifier.contains('@') { "email" } else { "login" };
    let body = json!({ key: identifier.trim(), "password": password });
    outcome(post_with_captcha(&agent(), &format!("{}/auth/login", ep.api_client), ep.origin.as_deref(), &body)?)
}

pub fn login_mfa_totp(ep: &Endpoints, ticket: &str, code: &str) -> Result<LoginOutcome, Error> {
    let body = json!({ "code": code.trim(), "ticket": ticket });
    outcome(post_with_captcha(&agent(), &format!("{}/auth/login/mfa/totp", ep.api_client), ep.origin.as_deref(), &body)?)
}

fn encode_query(v: &str) -> String {
    v.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
        _ => format!("%{b:02X}"),
    }).collect()
}

/// One poll of an IP authorization ticket: `Some` once the e-mailed link was opened.
pub fn poll_ip_authorization(ep: &Endpoints, ticket: &str) -> Result<Option<Credentials>, Error> {
    let url = format!("{}/auth/ip-authorization/poll?ticket={}", ep.api_client, encode_query(ticket));
    let (status, body) = get_json(&agent(), &url)?;
    let reply = Reply { status, body };
    if !(200..300).contains(&status) { return Err(failure(&reply)) }
    if reply.body.get("completed").and_then(Value::as_bool) != Some(true) { return Ok(None) }
    credentials(&reply.body).map(Some).ok_or_else(|| Error::local("resposta de autorização inesperada"))
}

pub fn resend_ip_authorization(ep: &Endpoints, ticket: &str) -> Result<(), Error> {
    let r = post(&agent(), &format!("{}/auth/ip-authorization/resend", ep.api_client), ep.origin.as_deref(), &json!({ "ticket": ticket }), None)?;
    if (200..300).contains(&r.status) { Ok(()) } else { Err(failure(&r)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// Serves canned `(status, json)` replies in order and records each request's head and body.
    fn mock(replies: Vec<(u16, Value)>) -> (String, Arc<Mutex<Vec<String>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for (status, body) in replies {
                let (mut s, _) = l.accept().unwrap();
                let mut buf = vec![0u8; 16384];
                let mut n = 0;
                // Read until the body is complete (Content-Length).
                loop {
                    n += s.read(&mut buf[n..]).unwrap();
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    if let Some(i) = text.find("\r\n\r\n") {
                        let len = text.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length: ").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                        if n >= i + 4 + len { log.lock().unwrap().push(text); break }
                    }
                }
                let b = body.to_string();
                write!(s, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len()).unwrap();
            }
        });
        (url, seen)
    }

    fn ep(url: &str) -> Endpoints { Endpoints { api_client: url.into(), gateway: "wss://g".into(), media: None, origin: Some("https://web.example".into()) } }

    #[test]
    fn discovery_reads_the_endpoints_and_falls_back_to_the_api_prefix() {
        let doc = json!({"endpoints": {"api_client": "https://api.example/v1/", "gateway": "wss://gw.example",
            "webapp": "https://web.example/app"}});
        let (url, seen) = mock(vec![(404, json!({})), (200, doc)]);
        let e = discover(&url).unwrap();
        assert_eq!((e.api_client.as_str(), e.gateway.as_str()), ("https://api.example/v1", "wss://gw.example"));
        assert_eq!(e.origin.as_deref(), Some("https://web.example"));
        assert!(seen.lock().unwrap()[1].starts_with("GET /api/.well-known/fluxer"));
    }

    #[test]
    fn password_login_returns_credentials_and_picks_email_or_login() {
        let ok = json!({"user_id": "7", "token": "tok", "user": {"username": "ernesto"}});
        let (url, seen) = mock(vec![(200, ok.clone()), (200, ok)]);
        let LoginOutcome::Signed(c) = login(&ep(&url), "a@b.c", "pw").unwrap() else { panic!() };
        assert_eq!((c.token.as_str(), c.username.as_deref()), ("tok", Some("ernesto")));
        login(&ep(&url), "ernesto", "pw").unwrap();
        let reqs = seen.lock().unwrap();
        let body = |i: usize| -> Value { serde_json::from_str(reqs[i].split("\r\n\r\n").nth(1).unwrap()).unwrap() };
        assert_eq!((body(0)["email"].as_str(), body(0).get("login")), (Some("a@b.c"), None));
        assert_eq!((body(1)["login"].as_str(), body(1).get("email")), (Some("ernesto"), None));
    }

    #[test]
    fn altcha_challenge_is_solved_and_the_request_retried_with_the_token() {
        let challenge = json!({"parameters": {"algorithm": "PBKDF2/SHA-256", "nonce": "00112233445566778899aabbccddeeff",
            "salt": "ffeeddccbbaa99887766554433221100", "cost": 10, "keyLength": 32, "keyPrefix": "0"}, "signature": "s"});
        let ask = json!({"code": "CAPTCHA_REQUIRED", "captcha_provider": "altcha", "altcha_challenge": challenge});
        let (url, seen) = mock(vec![(400, ask), (200, json!({"user_id": "1", "token": "t"}))]);
        assert!(matches!(login(&ep(&url), "u", "p").unwrap(), LoginOutcome::Signed(_)));
        let reqs = seen.lock().unwrap();
        assert!(!reqs[0].to_lowercase().contains("x-captcha-token"));
        assert!(reqs[1].to_lowercase().contains("x-captcha-token"));
    }

    #[test]
    fn mfa_ip_authorization_and_bad_password_are_told_apart() {
        let (url, _) = mock(vec![
            (200, json!({"mfa": true, "ticket": "tk", "totp": true})),
            (403, json!({"code": "IP_AUTHORIZATION_REQUIRED", "email": "a@b.c", "ticket": "ip1", "resend_available_in": 12})),
            (400, json!({"code": "INVALID_FORM_BODY", "message": "Senha inválida"})),
            (200, json!({"user_id": "1", "token": "t2"})),
        ]);
        let e = ep(&url);
        assert_eq!(login(&e, "u", "p").unwrap(), LoginOutcome::Mfa { ticket: "tk".into(), totp: true });
        assert_eq!(login(&e, "u", "p").unwrap(), LoginOutcome::IpAuthorization { ticket: "ip1".into(), email: "a@b.c".into(), resend_available_in: 12 });
        let err = login(&e, "u", "x").unwrap_err();
        assert_eq!((err.status, err.message.as_str()), (Some(400), "Senha inválida"));
        assert!(matches!(login_mfa_totp(&e, "tk", " 123456 ").unwrap(), LoginOutcome::Signed(_)));
    }

    #[test]
    fn api_transport_wins_and_posts_carry_the_web_origin() {
        let doc = json!({"endpoints": {"api_client": "https://web.example/api", "api_transport": "https://direct.example/v1",
            "gateway": "wss://g", "webapp": "https://web.example"}});
        let (url, seen) = mock(vec![(200, doc)]);
        assert_eq!(discover(&url).unwrap().api_client, "https://direct.example/v1");
        drop(seen);
        let (url, seen) = mock(vec![(200, json!({"user_id": "1", "token": "t"}))]);
        login(&ep(&url), "u", "p").unwrap();
        assert!(seen.lock().unwrap()[0].to_lowercase().contains("\r\norigin: https://web.example\r\n"));
    }

    #[test]
    fn ip_authorization_context_may_be_nested_and_polling_waits_for_completion() {
        let nested = json!({"code": "IP_AUTHORIZATION_REQUIRED", "details": {"context": {"ticket": "t/1", "email": "x@y.z"}}});
        let (url, seen) = mock(vec![
            (403, nested),
            (200, json!({"completed": false})),
            (200, json!({"completed": true, "token": "tok", "user_id": "9", "user": {"username": "e"}})),
            (204, json!(null)),
        ]);
        let e = ep(&url);
        let LoginOutcome::IpAuthorization { ticket, resend_available_in, .. } = login(&e, "u", "p").unwrap() else { panic!() };
        assert_eq!((ticket.as_str(), resend_available_in), ("t/1", 30));
        assert_eq!(poll_ip_authorization(&e, &ticket).unwrap(), None);
        let c = poll_ip_authorization(&e, &ticket).unwrap().unwrap();
        assert_eq!((c.token.as_str(), c.username.as_deref()), ("tok", Some("e")));
        resend_ip_authorization(&e, &ticket).unwrap();
        let reqs = seen.lock().unwrap();
        assert!(reqs[1].starts_with("GET /auth/ip-authorization/poll?ticket=t%2F1 "));
        assert!(reqs[3].starts_with("POST /auth/ip-authorization/resend"));
    }

    #[test]
    fn origins_are_scheme_and_host_only() {
        assert_eq!(origin_of("https://web.fluxer.app/api/x?y").as_deref(), Some("https://web.fluxer.app"));
        assert_eq!(origin_of("http://localhost:8080/").as_deref(), Some("http://localhost:8080"));
        assert_eq!(origin_of("wss://gw"), None);
    }

    #[test]
    fn instance_urls_are_normalised() {
        assert_eq!(normalize_instance(" fluxer.com/ "), "https://fluxer.com");
        assert_eq!(normalize_instance("http://localhost:8080"), "http://localhost:8080");
    }
}
