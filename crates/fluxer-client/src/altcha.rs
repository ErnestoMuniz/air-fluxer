//! Altcha v2 proof-of-work (PBKDF2 flavour), as solved by `altcha-lib` 2.8 in the web client
//! (`fluxer_app/src/features/auth/altcha`). The server sends `parameters` + `signature`; we find a
//! counter whose derived key starts with `keyPrefix` and return the base64 token for the
//! `X-Captcha-Token` header.

use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Deserialize)]
pub struct Parameters {
    pub algorithm: String,
    pub nonce: String,
    pub salt: String,
    pub cost: u32,
    #[serde(rename = "keyLength", default = "default_key_length")]
    pub key_length: usize,
    #[serde(rename = "keyPrefix")]
    pub key_prefix: String,
}

fn default_key_length() -> usize { 32 }

#[derive(Debug, PartialEq, Eq)]
pub enum Error { Unsupported(String), Malformed, Timeout }

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(a) => write!(f, "unsupported captcha algorithm {a}"),
            Self::Malformed => f.write_str("malformed captcha challenge"),
            Self::Timeout => f.write_str("captcha took too long"),
        }
    }
}
impl std::error::Error for Error {}

fn hex(bytes: &[u8]) -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() }

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 { return None }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

fn derive(algorithm: &str, password: &[u8], salt: &[u8], cost: u32, out: &mut [u8]) {
    match algorithm {
        "PBKDF2/SHA-384" => pbkdf2::pbkdf2_hmac::<sha2::Sha384>(password, salt, cost, out),
        "PBKDF2/SHA-512" => pbkdf2::pbkdf2_hmac::<sha2::Sha512>(password, salt, cost, out),
        _ => pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password, salt, cost, out),
    }
}

/// Solves `challenge` (`{ parameters, signature }`) and returns the token. Counters are written as
/// big-endian u32 after the nonce (altcha's default `uint32` mode) and interleaved over `threads`.
pub fn solve(challenge: &Value, threads: usize, timeout: Duration) -> Result<String, Error> {
    let p: Parameters = serde_json::from_value(challenge.get("parameters").cloned().ok_or(Error::Malformed)?)
        .map_err(|_| Error::Malformed)?;
    if !matches!(p.algorithm.as_str(), "PBKDF2/SHA-256" | "PBKDF2/SHA-384" | "PBKDF2/SHA-512") {
        return Err(Error::Unsupported(p.algorithm));
    }
    let nonce = unhex(&p.nonce).ok_or(Error::Malformed)?;
    let salt = unhex(&p.salt).ok_or(Error::Malformed)?;
    let prefix = p.key_prefix.to_lowercase();
    if prefix.is_empty() || !prefix.bytes().all(|b| b.is_ascii_hexdigit()) || prefix.len() > p.key_length * 2 {
        return Err(Error::Malformed);
    }
    let started = Instant::now();
    let done = AtomicBool::new(false);
    let threads = threads.clamp(1, 16);

    let found = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads).map(|t| {
            let (nonce, salt, prefix, p, done) = (&nonce, &salt, &prefix, &p, &done);
            scope.spawn(move || {
                let mut password = nonce.clone();
                password.extend_from_slice(&[0; 4]);
                let at = nonce.len();
                let mut key = vec![0u8; p.key_length];
                let mut counter = t as u32;
                let mut n = 0u32;
                loop {
                    if done.load(Ordering::Relaxed) { return None }
                    if n % 16 == 0 && started.elapsed() > timeout { done.store(true, Ordering::Relaxed); return None }
                    password[at..].copy_from_slice(&counter.to_be_bytes());
                    derive(&p.algorithm, &password, salt, p.cost, &mut key);
                    if hex(&key).starts_with(prefix.as_str()) {
                        done.store(true, Ordering::Relaxed);
                        return Some((counter, hex(&key)));
                    }
                    counter = match counter.checked_add(threads as u32) { Some(c) => c, None => return None };
                    n += 1;
                }
            })
        }).collect();
        handles.into_iter().filter_map(|h| h.join().ok().flatten()).next()
    });
    let (counter, derived) = found.ok_or(Error::Timeout)?;
    let time = (started.elapsed().as_secs_f64() * 10_000.0).floor() / 10.0;
    let body = json!({
        "challenge": { "parameters": challenge["parameters"], "signature": challenge["signature"] },
        "solution": { "counter": counter, "derivedKey": derived, "time": time },
    });
    Ok(base64::engine::general_purpose::STANDARD.encode(body.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a challenge the way `createChallenge` does in deterministic mode: derive the key for
    /// a known counter and use its first bytes as the prefix.
    fn challenge(algorithm: &str, counter: u32) -> Value {
        let (nonce, salt) = ("00112233445566778899aabbccddeeff", "ffeeddccbbaa99887766554433221100");
        let mut pw = unhex(nonce).unwrap();
        pw.extend_from_slice(&counter.to_be_bytes());
        let mut key = [0u8; 32];
        derive(algorithm, &pw, &unhex(salt).unwrap(), 50, &mut key);
        json!({ "parameters": { "algorithm": algorithm, "nonce": nonce, "salt": salt, "cost": 50,
            "keyLength": 32, "keyPrefix": hex(&key[..2]) }, "signature": "sig" })
    }

    #[test]
    fn finds_a_counter_and_builds_the_token() {
        for alg in ["PBKDF2/SHA-256", "PBKDF2/SHA-512"] {
            let c = challenge(alg, 300);
            let token = solve(&c, 4, Duration::from_secs(60)).unwrap();
            let v: Value = serde_json::from_slice(&base64::engine::general_purpose::STANDARD.decode(token).unwrap()).unwrap();
            assert_eq!(v["challenge"]["signature"], "sig");
            let counter = v["solution"]["counter"].as_u64().unwrap() as u32;
            // The returned counter really derives a key with the prefix.
            let mut pw = unhex("00112233445566778899aabbccddeeff").unwrap();
            pw.extend_from_slice(&counter.to_be_bytes());
            let mut key = [0u8; 32];
            derive(alg, &pw, &unhex("ffeeddccbbaa99887766554433221100").unwrap(), 50, &mut key);
            assert_eq!(hex(&key), v["solution"]["derivedKey"]);
            assert!(hex(&key).starts_with(c["parameters"]["keyPrefix"].as_str().unwrap()));
        }
    }

    #[test]
    fn rejects_unknown_algorithms_and_bad_prefixes() {
        let mut c = challenge("PBKDF2/SHA-256", 1);
        c["parameters"]["algorithm"] = json!("ARGON2ID");
        assert!(matches!(solve(&c, 1, Duration::from_secs(1)), Err(Error::Unsupported(_))));
        let mut c = challenge("PBKDF2/SHA-256", 1);
        c["parameters"]["keyPrefix"] = json!("");
        assert_eq!(solve(&c, 1, Duration::from_secs(1)), Err(Error::Malformed));
    }
}
