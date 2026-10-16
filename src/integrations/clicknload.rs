//! Click'n'Load v2 listener (the protocol JDownloader introduced, used by link protectors).
//!
//! The browser posts links — usually AES-encrypted — to `http://127.0.0.1:9666`. The listener is
//! started and stopped live from the settings. Requests must carry a loopback `Host` header, like
//! JDownloader requires.

use std::{net::SocketAddr, sync::Arc};

use aes::Aes128;
use axum::{
    Form, Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine as _;
use cbc::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::NoPadding};
use regex::Regex;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::{
    engine::{
        Engine,
        intake::{self, AddOptions},
    },
    events::{Notice, NoticeLevel},
};

const JDCHECK: &str = "jdownloader=true;\r\nvar version='42707';\r\n";
const CROSSDOMAIN: &str = "<?xml version=\"1.0\"?>\n<!DOCTYPE cross-domain-policy SYSTEM \"http://www.macromedia.com/xml/dtds/cross-domain-policy.dtd\">\n<cross-domain-policy>\n<allow-access-from domain=\"*\" />\n</cross-domain-policy>";

/// Starts/stops the listener when the setting changes.
pub async fn supervise(engine: Arc<Engine>, bind: SocketAddr) {
    let mut settings = engine.settings.subscribe();
    let mut running: Option<CancellationToken> = None;
    loop {
        let enabled = settings.borrow_and_update().clicknload.enabled;
        match (enabled, &running) {
            (true, None) => {
                let token = engine.shutdown_token().child_token();
                running = Some(token.clone());
                tokio::spawn(serve(engine.clone(), bind, token));
            }
            (false, Some(token)) => {
                token.cancel();
                running = None;
            }
            _ => {}
        }
        tokio::select! {
            _ = engine.shutdown_token().cancelled() => return,
            changed = settings.changed() => if changed.is_err() { return },
        }
    }
}

async fn serve(engine: Arc<Engine>, bind: SocketAddr, stop: CancellationToken) {
    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!("Click'n'Load cannot listen on {bind}: {error}");
            return;
        }
    };
    tracing::info!("Click'n'Load listening on {bind}");
    let app = Router::new()
        .route("/", get(banner))
        .route("/flash", get(banner).post(banner))
        .route("/flash/", get(banner).post(banner))
        .route("/jdcheck.js", get(|| async { ([(header::CONTENT_TYPE, "text/javascript")], JDCHECK) }))
        .route("/crossdomain.xml", get(|| async { ([(header::CONTENT_TYPE, "text/xml")], CROSSDOMAIN) }))
        .route("/flash/add", post(add))
        .route("/flash/addcrypted2", post(add_crypted))
        .layer(middleware::from_fn(guard))
        .with_state(engine);
    let result = axum::serve(listener, app).with_graceful_shutdown(async move { stop.cancelled().await }).await;
    if let Err(error) = result {
        tracing::error!("Click'n'Load stopped: {error}");
    }
    tracing::info!("Click'n'Load stopped");
}

/// Only loopback `Host` headers are accepted; CORS is open as the protocol requires.
async fn guard(request: axum::extract::Request, next: Next) -> Response {
    let host = request.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or_default();
    let host_name = host.rsplit_once(':').map(|(name, _)| name).unwrap_or(host);
    let mut response = if matches!(host_name, "localhost" | "127.0.0.1" | "[::1]") {
        next.run(request).await
    } else {
        (StatusCode::FORBIDDEN, "Forbidden").into_response()
    };
    let headers = response.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("OPTIONS, GET, POST"));
    headers.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("1800"));
    response
}

async fn banner() -> &'static str {
    "JDownloader\r\n"
}

#[derive(Deserialize)]
struct AddForm {
    #[serde(default)]
    urls: String,
    #[serde(flatten)]
    meta: PackageMeta,
}

#[derive(Deserialize)]
struct CryptedForm {
    crypted: String,
    jk: String,
    #[serde(flatten)]
    meta: PackageMeta,
}

#[derive(Deserialize, Default)]
struct PackageMeta {
    package: Option<String>,
    source: Option<String>,
    referer: Option<String>,
    passwords: Option<String>,
}

async fn add(State(engine): State<Arc<Engine>>, headers: HeaderMap, Form(form): Form<AddForm>) -> Response {
    let links = form.urls.replace(' ', "\n");
    submit(&engine, &links, form.meta, &headers).await
}

async fn add_crypted(State(engine): State<Arc<Engine>>, headers: HeaderMap, Form(form): Form<CryptedForm>) -> Response {
    match decrypt(&form.crypted, &form.jk) {
        Ok(links) => submit(&engine, &links, form.meta, &headers).await,
        Err(error) => failure(&error),
    }
}

async fn submit(engine: &Engine, links: &str, meta: PackageMeta, headers: &HeaderMap) -> Response {
    let referer = headers.get(header::REFERER).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let name = [meta.package, meta.source, meta.referer, referer]
        .into_iter()
        .flatten()
        .map(|n| n.trim().to_owned())
        .find(|n| !n.is_empty() && !n.contains("://"));
    let options = AddOptions {
        name: name.clone(),
        archive_password: meta.passwords.and_then(|p| p.lines().next().map(str::to_owned)),
        paused: Some(engine.settings.get().clicknload.paused),
        ..Default::default()
    };
    match intake::add_text(engine, links, &options).await {
        Ok(ids) => {
            let count = intake::parse_links(links);
            let count = count.http.len() + count.magnets.len();
            let label = name.unwrap_or_else(|| format!("{} link(s)", count));
            tracing::info!("Click'n'Load: received {count} link(s) ({} package(s))", ids.len());
            engine.events.notice(
                Notice::new(NoticeLevel::Info, "clicknload_added", format!("Click'n'Load: {label}"))
                    .param("name", &label)
                    .param("count", count.to_string()),
            );
            "success\r\n".into_response()
        }
        Err(error) => failure(&error.to_string()),
    }
}

fn failure(message: &str) -> Response {
    tracing::warn!("Click'n'Load request rejected: {message}");
    (StatusCode::INTERNAL_SERVER_ERROR, format!("failed {message}\r\n")).into_response()
}

/// Decrypts an `addcrypted2` payload: AES-128-CBC, key = IV = hex string returned by `jk`.
pub fn decrypt(crypted: &str, jk: &str) -> Result<String, String> {
    static KEY: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let key_regex = KEY.get_or_init(|| Regex::new(r#"['"]([0-9a-fA-F]{32})['"]"#).expect("valid regex"));
    let key_hex = key_regex.captures(jk).map(|c| c[1].to_owned()).ok_or("could not read the key from jk")?;
    let key: [u8; 16] =
        hex::decode(key_hex).map_err(|e| e.to_string())?.try_into().map_err(|_| "key must be 16 bytes")?;

    // Form decoding turns `+` into spaces: restore them before decoding base64.
    let data = base64::engine::general_purpose::STANDARD
        .decode(crypted.trim().replace(' ', "+"))
        .map_err(|e| format!("invalid base64: {e}"))?;
    if data.is_empty() || data.len() % 16 != 0 {
        return Err("encrypted data length must be a multiple of 16 bytes".into());
    }

    let attempt = |iv: &[u8; 16], ciphertext: &[u8]| -> Option<String> {
        let mut buffer = ciphertext.to_vec();
        let plain =
            cbc::Decryptor::<Aes128>::new(&key.into(), iv.into()).decrypt_padded::<NoPadding>(&mut buffer).ok()?;
        let plain = strip_padding(plain);
        String::from_utf8(plain.iter().copied().filter(|b| *b != 0).collect()).ok()
    };
    let text = attempt(&key, &data)
        .or_else(|| {
            // Some senders prefix the IV to the ciphertext.
            let iv: [u8; 16] = data.get(..16)?.try_into().ok()?;
            attempt(&iv, data.get(16..)?)
        })
        .ok_or("decrypted data is not valid text")?;
    Ok(text.replace('\r', "").trim().to_owned())
}

/// Removes PKCS#7 padding when present; zero padding is stripped by the caller.
fn strip_padding(data: &[u8]) -> &[u8] {
    match data.last() {
        Some(&n)
            if (1..=16).contains(&n)
                && data.len() >= n as usize
                && data[data.len() - n as usize..].iter().all(|b| *b == n) =>
        {
            &data[..data.len() - n as usize]
        }
        _ => data,
    }
}

#[cfg(test)]
mod tests {
    use cbc::cipher::{BlockModeEncrypt, block_padding::ZeroPadding};

    use super::*;

    fn encrypt(plain: &str, key: &[u8; 16]) -> String {
        let mut buffer = vec![0u8; plain.len().div_ceil(16) * 16];
        let encrypted = cbc::Encryptor::<Aes128>::new(key.into(), key.into())
            .encrypt_padded_b2b::<ZeroPadding>(plain.as_bytes(), &mut buffer)
            .unwrap()
            .to_vec();
        base64::engine::general_purpose::STANDARD.encode(encrypted)
    }

    #[test]
    fn decrypts_addcrypted2_payloads() {
        let key = *b"1234567890123456";
        let jk = format!("function f(){{ return '{}';}}", hex::encode(key));
        let crypted = encrypt("https://a.com/1\r\nhttps://b.com/2", &key);
        // Browsers send `+` as spaces in form data.
        let crypted = crypted.replace('+', " ");
        assert_eq!(decrypt(&crypted, &jk).unwrap(), "https://a.com/1\nhttps://b.com/2");
    }

    /// A payload encrypted with another AES implementation (pyca/cryptography).
    #[test]
    fn decrypts_a_known_payload() {
        let jk = format!("function f(){{ return '{}';}}", hex::encode(b"1234567890123456"));
        let links = decrypt("Ce7HOMz/NWeK+mfBziWFi/yKwbN8M7juLD+HICbHjRY=", &jk).unwrap();
        assert_eq!(links, "https://a.com/1\nhttps://b.com/2");
    }

    #[test]
    fn rejects_bad_keys() {
        assert!(decrypt("AAAA", "function f(){ return 'nothex'; }").is_err());
    }
}
