//! integra-bridge — minimal loopback HTTP API for the browser extension.
//!
//! Security model (same as the Python original, extended):
//! * binds to `127.0.0.1` only;
//! * mutating calls require a `Bearer` token compared in constant time;
//! * pairing exchanges the token for a short-lived 6-digit code (15 min);
//! * requests carrying an `Origin` header must come from a browser extension
//!   (`chrome-extension://` / `moz-extension://`);
//! * JSON bodies are capped at 64 KiB (bulk domain lists included).
//!
//! API version 2 adds: `POST /api/domains` (bulk add), `GET /api/profiles`,
//! and profile scoping via the `profile` query/body field.

use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rand::Rng;
use serde_json::{json, Value};
use subtle::ConstantTimeEq;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

pub const API_VERSION: u32 = 2;
pub const MAX_BODY_BYTES: usize = 64 * 1024;
const PAIRING_TTL: Duration = Duration::from_secs(15 * 60);

/// Backend operations the bridge needs; implemented by the app layer so this
/// crate stays UI- and profile-agnostic.
pub trait BridgeStore: Send + Sync {
    fn status(&self) -> Value;
    fn profiles(&self) -> Value;
    fn counts(&self, profile: Option<&str>) -> Value;
    fn contains(&self, profile: Option<&str>, domain: &str) -> (bool, bool);
    fn add_domains(
        &self,
        profile: Option<&str>,
        mode: &str,
        domains: &[String],
    ) -> Result<Value, String>;
    fn remove_domain(&self, profile: Option<&str>, domain: &str) -> Result<Vec<String>, String>;
    fn measure_latency(&self, host: &str) -> (bool, Option<u64>, Option<String>);
    fn log(&self, level: &str, message: &str);
}

struct Pairing {
    code: String,
    expires: Instant,
}

struct Running {
    shutdown: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub struct BridgeServer {
    store: Arc<dyn BridgeStore>,
    token: Mutex<String>,
    pairing: Mutex<Pairing>,
    port: AtomicU16,
    running: Mutex<Option<Running>>,
}

impl BridgeServer {
    pub fn new(store: Arc<dyn BridgeStore>, token: String, port: u16) -> Arc<Self> {
        Arc::new(Self {
            store,
            token: Mutex::new(token),
            pairing: Mutex::new(fresh_pairing()),
            port: AtomicU16::new(port),
            running: Mutex::new(None),
        })
    }

    pub fn is_running(&self) -> bool {
        self.running
            .lock()
            .as_ref()
            .is_some_and(|r| !r.shutdown.load(Ordering::Relaxed))
    }

    pub fn actual_port(&self) -> u16 {
        self.port.load(Ordering::Relaxed)
    }

    pub fn pairing_code(&self) -> String {
        let mut pairing = self.pairing.lock();
        if pairing.expires <= Instant::now() {
            *pairing = fresh_pairing();
        }
        pairing.code.clone()
    }

    pub fn pairing_expires_in(&self) -> u64 {
        self.pairing
            .lock()
            .expires
            .saturating_duration_since(Instant::now())
            .as_secs()
    }

    pub fn regenerate_pairing(&self) -> String {
        let mut pairing = self.pairing.lock();
        *pairing = fresh_pairing();
        pairing.code.clone()
    }

    pub fn update_token(&self, token: String) {
        *self.token.lock() = token;
    }

    pub fn start(self: &Arc<Self>) -> Result<u16, String> {
        let mut guard = self.running.lock();
        if guard
            .as_ref()
            .is_some_and(|r| !r.shutdown.load(Ordering::Relaxed))
        {
            return Err(integra_core::i18n::tr_with(
                "bridge.already_running",
                &[("port", &self.actual_port().to_string())],
            ));
        }
        let port = self.port.load(Ordering::Relaxed);
        let server = Server::http(("127.0.0.1", port)).map_err(|e| {
            integra_core::i18n::tr_with("bridge.start_failed", &[("error", &e.to_string())])
        })?;
        let actual = server
            .server_addr()
            .to_ip()
            .map(|a| a.port())
            .unwrap_or(port);
        self.port.store(actual, Ordering::Relaxed);

        let shutdown = Arc::new(AtomicBool::new(false));
        let loop_shutdown = shutdown.clone();
        let bridge = Arc::clone(self);
        let thread = std::thread::Builder::new()
            .name("integra-bridge".into())
            .spawn(move || {
                while !loop_shutdown.load(Ordering::Relaxed) {
                    match server.try_recv() {
                        Ok(Some(request)) => {
                            let bridge = Arc::clone(&bridge);
                            std::thread::spawn(move || bridge.handle(request));
                        }
                        Ok(None) => std::thread::sleep(Duration::from_millis(40)),
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| e.to_string())?;

        *guard = Some(Running {
            shutdown,
            thread: Some(thread),
        });
        let actual = self.actual_port();
        self.store.log(
            "SUCCESS",
            &integra_core::i18n::tr_with("bridge.log_started", &[("port", &actual.to_string())]),
        );
        Ok(actual)
    }

    pub fn stop(&self) {
        let running = self.running.lock().take();
        if let Some(running) = running {
            running.shutdown.store(true, Ordering::Relaxed);
            if let Some(thread) = running.thread {
                let _ = thread.join();
            }
            self.store
                .log("INFO", &integra_core::i18n::tr("bridge.log_stopped"));
        }
    }

    // ── request handling ─────────────────────────────────────

    fn handle(self: &Arc<Self>, mut request: Request) {
        let path = request.url().to_string();
        let (route, query) = split_query(&path);
        let method = request.method().clone();
        let origin = header_value(&request, "Origin");

        if !valid_origin(origin.as_deref()) {
            return respond(
                request,
                origin.as_deref(),
                403,
                json!({"ok": false, "error": integra_core::i18n::tr("bridge.invalid_origin")}),
            );
        }

        if method == Method::Options {
            let response = with_cors(Response::empty(StatusCode(204)), origin.as_deref());
            let _ = request.respond(response);
            return;
        }

        // Single auth gate for every /api/* route.
        let needs_auth = route.starts_with("/api/");
        if needs_auth && !self.token_ok(&request) {
            return respond(
                request,
                origin.as_deref(),
                401,
                json!({"ok": false, "error": integra_core::i18n::tr("bridge.auth_required")}),
            );
        }

        match (method, route.as_str()) {
            (Method::Get, "/health") => respond(
                request,
                origin.as_deref(),
                200,
                json!({"ok": true, "service": "integra-bridge", "api_version": API_VERSION, "paired": false}),
            ),
            (Method::Post, "/pair") => self.handle_pair(request, origin.as_deref()),

            (Method::Get, "/api/status") => {
                let profile = query_param(&query, "profile");
                let mut data = self.store.status();
                if let Value::Object(map) = &mut data {
                    map.insert("ok".into(), json!(true));
                    map.insert("api_version".into(), json!(API_VERSION));
                    map.insert("bridge_port".into(), json!(self.actual_port()));
                    map.insert("counts".into(), self.store.counts(profile.as_deref()));
                    if let Some(p) = &profile {
                        map.insert("profile".into(), json!(p));
                    }
                }
                respond(request, origin.as_deref(), 200, data);
            }
            (Method::Get, "/api/profiles") => {
                respond(
                    request,
                    origin.as_deref(),
                    200,
                    json!({"ok": true, "profiles": self.store.profiles()}),
                );
            }
            (Method::Get, "/api/ping") => {
                let host = query_param(&query, "host").unwrap_or_default();
                match integra_core::normalize_host(&host, false) {
                    Ok(normalized) => {
                        let (ok, latency, error) = self.store.measure_latency(&normalized);
                        respond(
                            request,
                            origin.as_deref(),
                            200,
                            json!({"ok": ok, "host": normalized, "latency_ms": latency, "error": error}),
                        );
                    }
                    Err(err) => respond(
                        request,
                        origin.as_deref(),
                        400,
                        json!({"ok": false, "error": err.to_string()}),
                    ),
                }
            }
            (Method::Get, "/api/domain") => {
                let profile = query_param(&query, "profile");
                let raw = query_param(&query, "domain").unwrap_or_default();
                match integra_core::normalize_host(&raw, true) {
                    Ok(domain) => {
                        let (bypass, exclude) = self.store.contains(profile.as_deref(), &domain);
                        respond(
                            request,
                            origin.as_deref(),
                            200,
                            json!({"ok": true, "domain": domain, "bypass": bypass, "exclude": exclude}),
                        );
                    }
                    Err(err) => respond(
                        request,
                        origin.as_deref(),
                        400,
                        json!({"ok": false, "error": err.to_string()}),
                    ),
                }
            }
            (Method::Post, "/api/domain") => {
                let payload = match self.read_json(&mut request) {
                    Ok(payload) => payload,
                    Err((status, err)) => return respond(request, origin.as_deref(), status, err),
                };
                let profile = payload
                    .get("profile")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let mode = payload
                    .get("mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_lowercase();
                if !["bypass", "exclude"].contains(&mode.as_str()) {
                    return respond(
                        request,
                        origin.as_deref(),
                        400,
                        json!({"ok": false, "error": integra_core::i18n::tr("bridge.bad_mode")}),
                    );
                }
                let raw = payload
                    .get("domain")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                match integra_core::normalize_host(&raw, true) {
                    Ok(domain) => {
                        match self.store.add_domains(
                            profile.as_deref(),
                            &mode,
                            std::slice::from_ref(&domain),
                        ) {
                            Ok(report) => {
                                self.store.log(
                                    "INFO",
                                    &integra_core::i18n::tr_with(
                                        "bridge.domain_log",
                                        &[("domain", &domain), ("mode", &mode)],
                                    ),
                                );
                                let mut out = report;
                                if let Value::Object(map) = &mut out {
                                    map.insert("ok".into(), json!(true));
                                    map.insert("domain".into(), json!(domain));
                                    map.insert("mode".into(), json!(mode));
                                    map.insert("restart_required".into(), json!(true));
                                }
                                respond(request, origin.as_deref(), 200, out);
                            }
                            Err(err) => respond(
                                request,
                                origin.as_deref(),
                                400,
                                json!({"ok": false, "error": err}),
                            ),
                        }
                    }
                    Err(err) => respond(
                        request,
                        origin.as_deref(),
                        400,
                        json!({"ok": false, "error": err.to_string()}),
                    ),
                }
            }
            // NEW in API v2: bulk add — powers the extension's "capture all URLs" flow.
            (Method::Post, "/api/domains") => {
                let payload = match self.read_json(&mut request) {
                    Ok(payload) => payload,
                    Err((status, err)) => return respond(request, origin.as_deref(), status, err),
                };
                let profile = payload
                    .get("profile")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let mode = payload
                    .get("mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_lowercase();
                if !["bypass", "exclude"].contains(&mode.as_str()) {
                    return respond(
                        request,
                        origin.as_deref(),
                        400,
                        json!({"ok": false, "error": integra_core::i18n::tr("bridge.bad_mode")}),
                    );
                }
                let domains: Vec<String> = payload
                    .get("domains")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .take(2000)
                            .collect()
                    })
                    .unwrap_or_default();
                if domains.is_empty() {
                    return respond(
                        request,
                        origin.as_deref(),
                        400,
                        json!({"ok": false, "error": "domains[] is empty"}),
                    );
                }
                match self.store.add_domains(profile.as_deref(), &mode, &domains) {
                    Ok(report) => {
                        let added = report
                            .get("added")
                            .and_then(|v| v.as_array())
                            .map(|a| a.len())
                            .unwrap_or(0);
                        self.store.log(
                            "INFO",
                            &integra_core::i18n::tr_with(
                                "bridge.bulk_log",
                                &[("added", &added.to_string()), ("mode", &mode)],
                            ),
                        );
                        let mut out = report;
                        if let Value::Object(map) = &mut out {
                            map.insert("ok".into(), json!(true));
                            map.insert("mode".into(), json!(mode));
                            map.insert("restart_required".into(), json!(true));
                        }
                        respond(request, origin.as_deref(), 200, out);
                    }
                    Err(err) => respond(
                        request,
                        origin.as_deref(),
                        400,
                        json!({"ok": false, "error": err}),
                    ),
                }
            }
            (Method::Delete, "/api/domain") => {
                let profile = query_param(&query, "profile");
                let raw = query_param(&query, "domain").unwrap_or_default();
                match integra_core::normalize_host(&raw, true) {
                    Ok(domain) => match self.store.remove_domain(profile.as_deref(), &domain) {
                        Ok(removed) => {
                            self.store.log(
                                "INFO",
                                &integra_core::i18n::tr_with(
                                    "bridge.domain_removed_log",
                                    &[("domain", &domain)],
                                ),
                            );
                            respond(
                                request,
                                origin.as_deref(),
                                200,
                                json!({"ok": true, "domain": domain, "removed_from": removed, "restart_required": !removed.is_empty()}),
                            );
                        }
                        Err(err) => respond(
                            request,
                            origin.as_deref(),
                            400,
                            json!({"ok": false, "error": err}),
                        ),
                    },
                    Err(err) => respond(
                        request,
                        origin.as_deref(),
                        400,
                        json!({"ok": false, "error": err.to_string()}),
                    ),
                }
            }
            (_, _) => respond(
                request,
                origin.as_deref(),
                404,
                json!({"ok": false, "error": integra_core::i18n::tr("bridge.route_missing")}),
            ),
        }
    }

    fn handle_pair(self: &Arc<Self>, mut request: Request, origin: Option<&str>) {
        let payload = match self.read_json(&mut request) {
            Ok(payload) => payload,
            Err((status, err)) => return respond(request, origin, status, err),
        };
        let code = payload
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let pairing = self.pairing.lock();
        let valid = pairing.expires > Instant::now()
            && bool::from(code.as_bytes().ct_eq(pairing.code.as_bytes()));
        if !valid {
            drop(pairing);
            return respond(
                request,
                origin,
                401,
                json!({"ok": false, "error": integra_core::i18n::tr("bridge.bad_pair_code")}),
            );
        }
        let token = self.token.lock().clone();
        let port = self.actual_port();
        drop(pairing);
        self.store
            .log("INFO", &integra_core::i18n::tr("bridge.connected"));
        respond(
            request,
            origin,
            200,
            json!({"ok": true, "token": token, "port": port, "api_version": API_VERSION}),
        );
    }

    fn token_ok(&self, request: &Request) -> bool {
        let provided = header_value(request, "Authorization")
            .and_then(|h| h.strip_prefix("Bearer ").map(|s| s.trim().to_string()))
            .unwrap_or_default();
        let token = self.token.lock().clone();
        !provided.is_empty() && bool::from(provided.as_bytes().ct_eq(token.as_bytes()))
    }

    /// Parse a JSON object body. Errors carry the HTTP response to send.
    fn read_json(&self, request: &mut Request) -> Result<Value, (u16, Value)> {
        let length = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("Content-Length"))
            .and_then(|h| h.value.as_str().parse::<usize>().ok())
            .unwrap_or(0);
        let fail = |key: &str| {
            Err((
                400u16,
                json!({"ok": false, "error": integra_core::i18n::tr(key)}),
            ))
        };
        if length == 0 || length > MAX_BODY_BYTES {
            return fail("bridge.bad_length");
        }
        let mut body = vec![0u8; length];
        if request.as_reader().read_exact(&mut body).is_err() {
            return fail("bridge.bad_json");
        }
        match serde_json::from_slice::<Value>(&body) {
            Ok(value @ Value::Object(_)) => Ok(value),
            Ok(_) => fail("bridge.json_object"),
            Err(_) => fail("bridge.bad_json"),
        }
    }
}

fn fresh_pairing() -> Pairing {
    let code = format!("{:06}", rand::thread_rng().gen_range(0..1_000_000u32));
    Pairing {
        code,
        expires: Instant::now() + PAIRING_TTL,
    }
}

fn split_query(path: &str) -> (String, String) {
    match path.split_once('?') {
        Some((route, query)) => (route.to_string(), query.to_string()),
        None => (path.to_string(), String::new()),
    }
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        if k == key {
            Some(urldecode(v))
        } else {
            None
        }
    })
}

fn urldecode(value: &str) -> String {
    let mut out = Vec::new();
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                if let Ok(byte) = u8::from_str_radix(&value[i + 1..i + 3], 16) {
                    out.push(byte);
                    i += 2;
                } else {
                    out.push(b'%');
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn header_value(request: &Request, name: &str) -> Option<String> {
    let needle = name.to_ascii_lowercase();
    request
        .headers()
        .iter()
        .find(|h| h.field.as_str().to_ascii_lowercase() == needle)
        .map(|h| h.value.as_str().to_string())
}

fn valid_origin(origin: Option<&str>) -> bool {
    match origin {
        Some(origin) => {
            origin.starts_with("chrome-extension://") || origin.starts_with("moz-extension://")
        }
        None => true, // non-browser clients (curl, tests) carry no Origin
    }
}

fn with_cors<R: Read>(response: Response<R>, origin: Option<&str>) -> Response<R> {
    let mut response = response;
    if let Some(origin) = origin {
        if origin.starts_with("chrome-extension://") || origin.starts_with("moz-extension://") {
            if let Ok(header) = Header::from_bytes("Access-Control-Allow-Origin", origin.as_bytes())
            {
                response.add_header(header);
            }
            if let Ok(header) = Header::from_bytes("Vary", b"Origin") {
                response.add_header(header);
            }
        }
    }
    for (name, value) in [
        (
            "Access-Control-Allow-Headers",
            "Authorization, Content-Type",
        ),
        ("Access-Control-Allow-Methods", "GET, POST, DELETE, OPTIONS"),
        ("Cache-Control", "no-store"),
        ("X-Content-Type-Options", "nosniff"),
    ] {
        if let Ok(header) = Header::from_bytes(name, value) {
            response.add_header(header);
        }
    }
    response
}

fn respond(request: Request, origin: Option<&str>, status: u16, payload: Value) {
    let body = serde_json::to_vec(&payload).unwrap_or_default();
    let response = Response::from_data(body).with_status_code(StatusCode(status));
    let response = with_cors(response, origin);
    let response = response.with_header(
        Header::from_bytes("Content-Type", b"application/json; charset=utf-8").unwrap(),
    );
    let _ = request.respond(response);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpStream;

    struct MockStore;

    impl BridgeStore for MockStore {
        fn status(&self) -> Value {
            json!({"running": false})
        }
        fn profiles(&self) -> Value {
            json!([{"id": "p1", "name": "Main"}])
        }
        fn counts(&self, _profile: Option<&str>) -> Value {
            json!({"bypass": 3, "exclude": 1})
        }
        fn contains(&self, _profile: Option<&str>, domain: &str) -> (bool, bool) {
            (
                domain == "in-bypass.example",
                domain == "in-exclude.example",
            )
        }
        fn add_domains(
            &self,
            _profile: Option<&str>,
            mode: &str,
            domains: &[String],
        ) -> Result<Value, String> {
            Ok(json!({"added": domains, "duplicates": [], "invalid": [], "mode": mode}))
        }
        fn remove_domain(
            &self,
            _profile: Option<&str>,
            domain: &str,
        ) -> Result<Vec<String>, String> {
            Ok(vec!["list-general".into(), domain.len().to_string()])
        }
        fn measure_latency(&self, _host: &str) -> (bool, Option<u64>, Option<String>) {
            (true, Some(42), None)
        }
        fn log(&self, _level: &str, _message: &str) {}
    }

    fn raw_request(port: u16, request: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut buf = String::new();
        let _ = stream.read_to_string(&mut buf);
        let status = buf
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let body = buf.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, body)
    }

    #[test]
    fn pairing_auth_and_endpoints() {
        let bridge = BridgeServer::new(Arc::new(MockStore), "secret-token".into(), 0);
        let port = bridge.start().unwrap();
        assert!(port > 0);

        // /health is open.
        let (status, body) = raw_request(
            port,
            "GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 200);
        assert!(body.contains("integra-bridge"));

        // API requires the token.
        let (status, _) = raw_request(
            port,
            "GET /api/status HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 401);

        // Wrong pairing code.
        let wrong = r#"{"code":"000000"}"#;
        let (status, _) = raw_request(
            port,
            &format!("POST /pair HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", wrong.len(), wrong),
        );
        assert_eq!(status, 401);

        // Correct code → token.
        let code = bridge.pairing_code();
        let body = format!("{{\"code\":\"{code}\"}}");
        let (status, resp) = raw_request(
            port,
            &format!("POST /pair HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body),
        );
        assert_eq!(status, 200);
        let parsed: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(parsed["token"], "secret-token");
        assert_eq!(parsed["api_version"], API_VERSION);

        // Authorized status.
        let (status, resp) = raw_request(
            port,
            "GET /api/status HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer secret-token\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 200);
        let parsed: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(parsed["counts"]["bypass"], 3);

        // Ping + domain lookup + bulk add + delete.
        let (status, resp) = raw_request(
            port,
            "GET /api/ping?host=discord.com HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer secret-token\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 200);
        assert!(resp.contains("\"latency_ms\":42"));

        let (status, resp) = raw_request(
            port,
            "GET /api/domain?domain=in-bypass.example HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer secret-token\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 200);
        assert!(resp.contains("\"bypass\":true"));

        let bulk = r#"{"domains":["a.example","b.example"],"mode":"bypass"}"#;
        let (status, resp) = raw_request(
            port,
            &format!("POST /api/domains HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer secret-token\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", bulk.len(), bulk),
        );
        assert_eq!(status, 200);
        assert!(resp.contains("a.example"));

        let (status, resp) = raw_request(
            port,
            "DELETE /api/domain?domain=x.example HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer secret-token\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 200);
        assert!(resp.contains("removed_from"));

        // Bad Origin is rejected.
        let (status, _) = raw_request(
            port,
            "GET /health HTTP/1.1\r\nHost: x\r\nOrigin: https://evil.example\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 403);

        // Extension Origin passes.
        let (status, _) = raw_request(
            port,
            "GET /health HTTP/1.1\r\nHost: x\r\nOrigin: chrome-extension://abcdef\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 200);

        // Unknown route.
        let (status, _) = raw_request(
            port,
            "GET /api/nope HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer secret-token\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 404);

        bridge.stop();
        assert!(!bridge.is_running());
    }

    #[test]
    fn urldecode_handles_percent_escapes() {
        assert_eq!(urldecode("a%20b+c"), "a b c");
        assert_eq!(urldecode("xn--e1afmkfd.%D1%80%D1%84"), "xn--e1afmkfd.рф");
    }
}
