//! `zega-server cloud` against a fake Zega Cloud API: a real HTTP server on 127.0.0.1
//! (port 0) that records every request it gets and answers with bodies shaped
//! like the OpenAPI document's schemas (https://cloud.zega.dev/openapi.json:
//! `TokenInfo`, `Project`, `Graph`, `Bucket`, `Function`, `LogEntry`, `Error`).
//! The tests run the real `zega-server` binary and assert what reached the server and
//! what came back out, not only what was printed.

use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::Duration,
};

const BIN: &str = env!("CARGO_BIN_EXE_zega-server");
const TOKEN: &str = "zc_ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

// ---- the fake API -----------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    /// Path and query, as sent.
    target: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl Seen {
    fn path(&self) -> &str {
        self.target.split('?').next().unwrap()
    }
    fn query(&self) -> HashMap<String, String> {
        match self.target.split_once('?') {
            Some((_, query)) => url::form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect(),
            None => HashMap::new(),
        }
    }
    fn auth(&self) -> Option<&str> {
        self.headers.get("authorization").map(String::as_str)
    }
}

struct Answer {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
}

fn ok(body: Value) -> Answer {
    Answer {
        status: 200,
        headers: vec![],
        body: body.to_string(),
    }
}

fn error(status: u16, code: &str, message: &str) -> Answer {
    Answer {
        status,
        headers: vec![],
        body: json!({"ok": false, "error": message, "code": code}).to_string(),
    }
}

struct Fake {
    address: std::net::SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Fake {
    fn start(handler: impl Fn(&Seen) -> Answer + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen_in, stop_in) = (seen.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stop_in.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(mut stream) = stream else { continue };
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let Some(request) = read_request(&mut stream) else {
                    continue;
                };
                let answer = handler(&request);
                seen_in.lock().unwrap().push(request);
                let mut head = format!(
                    "HTTP/1.1 {} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n",
                    answer.status,
                    answer.body.len()
                );
                for (name, value) in &answer.headers {
                    head.push_str(&format!("{name}: {value}\r\n"));
                }
                let _ = stream.write_all(format!("{head}\r\n{}", answer.body).as_bytes());
            }
        });
        Self {
            address,
            seen,
            stop,
            thread: Some(thread),
        }
    }
    fn url(&self) -> String {
        format!("http://{}", self.address)
    }
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Seen> {
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(at) = data.windows(4).position(|window| window == b"\r\n\r\n") {
            break at;
        }
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        data.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&data[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target) = (first.next()?.to_string(), first.next()?.to_string());
    let headers: HashMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    let length: usize = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = data[head_end + 4..].to_vec();
    while body.len() < length {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    Some(Seen {
        method,
        target,
        headers,
        body,
    })
}

// ---- fixtures: answers shaped like the OpenAPI schemas --------------------------------------

fn token_info(scope: &str, project: Option<(&str, &str)>) -> Value {
    json!({"token": {
        "id": "tk_1", "name": "ci deploy", "prefix": "zc_ABCDEFGH", "scope": scope,
        "projectId": project.map(|(id, _)| id), "projectName": project.map(|(_, name)| name),
        "createdAt": "2026-10-01T10:00:00.000Z", "lastUsedAt": null, "expiresAt": "2027-01-01T00:00:00.000Z"
    }})
}

fn graph(id: &str, name: &str) -> Value {
    json!({
        "id": id, "name": name, "projectId": "p1", "projectName": "alpha", "region": "yyz", "tier": "256mb",
        "memoryMb": 256, "priceCents": 500, "billing": "monthly", "status": "active",
        "url": format!("/g/{id}/"), "hostUrl": format!("https://{id}.zegadb.com"),
        "graceUntil": null, "deleteAfter": null, "createdAt": "2026-09-30T12:00:00.000Z"
    })
}

fn projects_body() -> Value {
    json!({"projects": [
        {"id": "p1", "name": "alpha", "createdAt": "2026-09-30T12:00:00.000Z", "graphs": [graph("g0filmsfilmsfilms0", "films")]},
        {"id": "p22", "name": "beta", "createdAt": "2026-10-01T08:30:00.000Z", "graphs": []}
    ]})
}

fn function(version: u64) -> Value {
    json!({
        "id": "f0abcdefghijklmnop", "name": "hello", "projectId": "p1", "projectName": "alpha",
        "url": "https://f0abcdefghijklmnop.zegadb.com", "capCents": 0, "deployed": version > 0, "version": version,
        "etag": "abc", "deployedAt": "2026-10-02T09:00:00.000Z", "vars": {"REGION": "yyz"}, "secrets": ["API_KEY"],
        "limits": {"cpuMs": 10, "subRequests": 50}, "logs": "on", "logsStoppedAt": null,
        "createdAt": "2026-10-01T00:00:00.000Z", "updatedAt": "2026-10-02T09:00:00.000Z"
    })
}

const GRAPH: &str = "g0filmsfilmsfilms0";
const FUNCTION: &str = "f0abcdefghijklmnop";

fn named_function(name: &str) -> Value {
    let mut function = function(3);
    function["name"] = json!(name);
    function
}

fn bucket(name: &str) -> Value {
    json!({
        "id": "b0photos", "name": name, "url": "https://b0photos.zegadb.com", "projectId": "p1", "projectName": "alpha",
        "bytesStored": 1500000, "capCents": 0, "createdAt": "2026-10-01T00:00:00.000Z"
    })
}

fn project_named(name: &str) -> Value {
    json!({"id": "p1", "name": name, "createdAt": "2026-09-30T12:00:00.000Z", "graphs": [graph(GRAPH, "films")]})
}

fn listed_key(id: &str, name: &str, prefix: &str) -> Value {
    json!({"id": id, "name": name, "prefix": prefix, "createdAt": "2026-10-01T10:00:00.000Z", "lastUsedAt": null, "revokedAt": null})
}

fn domain(hostname: &str, active: bool) -> Value {
    json!({
        "hostname": hostname, "url": format!("https://{hostname}"), "verified": active, "active": active,
        "status": if active { "active" } else { "pending" }, "sslStatus": if active { json!("active") } else { json!("pending_validation") },
        "records": [{"type": "CNAME", "name": hostname, "value": "edge.zegadb.com"}],
        "createdAt": "2026-10-02T09:00:00.000Z", "expiresAt": null, "checkedAt": "2026-10-02T09:00:05.000Z"
    })
}

fn created(body: Value) -> Answer {
    Answer {
        status: 201,
        headers: vec![],
        body: body.to_string(),
    }
}

/// The whole manage API in one fake: each route answers with the body its
/// schema in the OpenAPI document gives (`Project`, `Graph`, `Bucket`, `Function`,
/// `KeyCreated`, `Domain`, ...). A rename answers with the name it was sent.
fn manage_api(request: &Seen) -> Answer {
    let path = request.path().to_string();
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
    let name = body["name"].as_str().unwrap_or("").to_string();
    match (request.method.as_str(), parts.as_slice()) {
        ("GET", ["v1", "projects", _]) | ("DELETE", ["v1", "projects", _]) => {
            ok(json!({"project": project_named("alpha")}))
        }
        ("PUT", ["v1", "projects", _, "name"]) => ok(json!({"project": project_named(&name)})),
        ("GET", ["v1", "graphs", _]) => ok(json!({"graph": graph(GRAPH, "films")})),
        ("DELETE", ["v1", "graphs", _]) => {
            let mut deleted = graph(GRAPH, "films");
            deleted["status"] = json!("grace");
            deleted["deleteAfter"] = json!("2026-11-01T00:00:00.000Z");
            ok(json!({"graph": deleted}))
        }
        ("PUT", ["v1", "graphs", _, "name"]) => ok(json!({"graph": graph(GRAPH, &name)})),
        ("GET", ["v1", "buckets", _]) | ("DELETE", ["v1", "buckets", _]) => {
            ok(json!({"bucket": bucket("photos")}))
        }
        ("PUT", ["v1", "buckets", _, "name"]) => ok(json!({"bucket": bucket(&name)})),
        ("POST", ["v1", "projects", _, "buckets"]) => created(json!({"bucket": bucket(&name)})),
        ("GET", ["v1", "functions", _]) | ("DELETE", ["v1", "functions", _]) => {
            ok(json!({"function": named_function("hello")}))
        }
        ("PUT", ["v1", "functions", _, "name"]) => ok(json!({"function": named_function(&name)})),
        ("POST", ["v1", "projects", _, "functions"]) => {
            let mut fresh = named_function(&name);
            fresh["deployed"] = json!(false);
            fresh["version"] = json!(0);
            created(json!({"function": fresh}))
        }
        ("PUT", ["v1", "functions", _, "logs"]) => {
            let mut changed = function(3);
            changed["logs"] = body["logs"].clone();
            ok(json!({"function": changed}))
        }
        (_, ["v1", "functions", _, "code" | "vars" | "secrets", ..]) => {
            ok(json!({"function": function(4)}))
        }
        ("GET", ["v1", "graphs" | "buckets", _, "keys"]) => ok(json!({"keys": [
            listed_key("key_a1", "web", "zk_AAAA"),
            {"id": "key_b2", "name": "old", "prefix": "zk_BBBB", "createdAt": "2026-09-01T00:00:00.000Z",
             "lastUsedAt": "2026-09-20T00:00:00.000Z", "revokedAt": "2026-09-25T00:00:00.000Z"}
        ]})),
        ("POST", ["v1", kind @ ("graphs" | "buckets"), _, "keys"]) => {
            let (prefix, secret) = if *kind == "graphs" {
                ("zk_NEWK", "zk_NEWKSECRETSECRETSECRET234567")
            } else {
                ("zs_NEWK", "zs_NEWKSECRETSECRETSECRET234567")
            };
            created(json!({"key": {
                "id": "key_new", "name": body["name"].as_str().unwrap_or("API key"), "prefix": prefix,
                "createdAt": "2026-10-02T10:00:00.000Z", "secret": secret
            }, "shownOnce": true}))
        }
        ("DELETE", ["v1", "graphs" | "buckets", _, "keys", key]) => ok(json!({"revoked": key})),
        ("GET", ["v1", "graphs", _, "domains"]) => ok(json!({
            "domains": [domain("films.example.com", true), domain("new.example.com", false)],
            "cnameTarget": "edge.zegadb.com", "limit": 5
        })),
        ("POST", ["v1", "graphs", _, "domains"]) => {
            created(json!({"domain": domain(body["hostname"].as_str().unwrap_or(""), false)}))
        }
        ("DELETE", ["v1", "graphs", _, "domains", hostname]) => ok(json!({"removed": hostname})),
        ("GET", ["v1", "graphs", _, "monitoring", "settings"]) => ok(
            json!({"graph": GRAPH, "keepQueryText": false, "textRetentionHours": 24, "logRetentionDays": 7}),
        ),
        ("PUT", ["v1", "graphs", _, "monitoring", "settings"]) => ok(
            json!({"graph": GRAPH, "keepQueryText": body["keepQueryText"], "textRetentionHours": 24, "logRetentionDays": 7}),
        ),
        _ => error(
            404,
            "not_found",
            &format!("no route {} {path}", request.method),
        ),
    }
}

/// `"PUT /v1/graphs/{graph}/name"` against `"PUT /v1/graphs/g0x/name"`.
fn matches_route(pattern: &str, seen: &Seen) -> bool {
    let (method, pattern) = pattern.split_once(' ').unwrap();
    let wanted: Vec<&str> = pattern.split('/').collect();
    let got: Vec<&str> = seen.path().split('/').collect();
    method == seen.method
        && wanted.len() == got.len()
        && wanted
            .iter()
            .zip(&got)
            .all(|(wanted, got)| wanted.starts_with('{') || wanted == got)
}

// ---- running the CLI --------------------------------------------------------------------------

struct Out {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Out {
    fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// A user's machine: its own home, so credentials never touch the real one.
struct Cloud {
    home: tempfile::TempDir,
}

impl Cloud {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
        }
    }
    fn credentials(&self) -> PathBuf {
        if cfg!(windows) {
            self.home.path().join("zega").join("cloud.json")
        } else {
            self.home
                .path()
                .join(".config")
                .join("zega")
                .join("cloud.json")
        }
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(BIN);
        command
            .arg("cloud")
            .args(args)
            .env("HOME", self.home.path())
            .env("APPDATA", self.home.path())
            .env_remove("XDG_CONFIG_HOME")
            .current_dir(self.home.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }
    fn run(&self, args: &[&str], stdin: &str) -> Out {
        let mut child = self.command(args).stdin(Stdio::piped()).spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        Out {
            code: output.status.code(),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    }
    /// Run with a real terminal on stdin (a pseudo-terminal), the way a person
    /// at a shell does, and type `typed` into it.
    #[cfg(unix)]
    fn run_at_a_terminal(&self, args: &[&str], typed: &str) -> Out {
        use std::os::fd::{FromRawFd, OwnedFd};
        let (mut master, mut slave) = (0, 0);
        // SAFETY: openpty fills in two new file descriptors, which are wrapped at once.
        let (master, slave) = unsafe {
            let made = libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            assert_eq!(made, 0, "openpty");
            (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave))
        };
        let mut command = self.command(args);
        command.stdin(Stdio::from(slave));
        let child = command.spawn().unwrap();
        // The terminal buffers what is typed until the command reads its line.
        let mut keyboard = std::fs::File::from(master);
        keyboard.write_all(typed.as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        drop(keyboard);
        Out {
            code: output.status.code(),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    }
    /// Store a token for `api` the way `login` does, without going through it.
    fn log_in(&self, api: &str, token: &str) {
        let path = self.credentials();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, json!({"api": api, "token": token}).to_string()).unwrap();
    }
}

fn assert_no_token(out: &Out) {
    assert!(
        !out.stdout.contains(TOKEN),
        "token on stdout: {}",
        out.stdout
    );
    assert!(
        !out.stderr.contains(TOKEN),
        "token on stderr: {}",
        out.stderr
    );
}

fn bearer() -> String {
    format!("Bearer {TOKEN}")
}

// ---- credentials ------------------------------------------------------------------------------

#[test]
fn login_verifies_the_token_then_saves_it_privately() {
    let fake = Fake::start(|_| ok(token_info("manage", None)));
    let cloud = Cloud::new();
    let out = cloud.run(&["--api", &fake.url(), "login"], &format!("{TOKEN}\n"));
    assert!(out.success(), "{}", out.stderr);
    assert_no_token(&out);
    // It asked the API who the token is, with the token.
    let seen = fake.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        (seen[0].method.as_str(), seen[0].path()),
        ("GET", "/v1/whoami")
    );
    assert_eq!(seen[0].auth(), Some(bearer().as_str()));
    // It shows what the API said about the token: name, prefix, scope.
    for shown in [
        "ci deploy",
        "zc_ABCDEFGH",
        "manage",
        "all projects",
        "2027-01-01",
    ] {
        assert!(
            out.stdout.contains(shown),
            "{shown} missing from {}",
            out.stdout
        );
    }
    // And saved the token with the host it belongs to.
    let stored: Value =
        serde_json::from_str(&std::fs::read_to_string(cloud.credentials()).unwrap()).unwrap();
    assert_eq!(stored, json!({"api": fake.url(), "token": TOKEN}));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode =
            |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&cloud.credentials()), 0o600, "the credentials file");
        assert_eq!(
            mode(cloud.credentials().parent().unwrap()),
            0o700,
            "its directory"
        );
    }
    let leftovers: Vec<_> = std::fs::read_dir(cloud.credentials().parent().unwrap())
        .unwrap()
        .collect();
    assert_eq!(leftovers.len(), 1, "no partial file is left behind");
}

#[test]
fn login_refuses_a_token_the_api_rejects_and_keeps_the_old_one() {
    let fake = Fake::start(|_| {
        error(
            401,
            "invalid_token",
            "That API token is not valid: check it was copied whole.",
        )
    });
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), "zc_old");
    let out = cloud.run(&["--api", &fake.url(), "login"], &format!("{TOKEN}\n"));
    assert_eq!(out.code, Some(1));
    assert!(
        out.stderr.contains("That API token is not valid"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("invalid_token"), "{}", out.stderr);
    assert_no_token(&out);
    let stored: Value =
        serde_json::from_str(&std::fs::read_to_string(cloud.credentials()).unwrap()).unwrap();
    assert_eq!(
        stored["token"], "zc_old",
        "a rejected token must not replace the stored one"
    );
}

#[test]
fn login_never_takes_the_token_from_an_argument() {
    let cloud = Cloud::new();
    let out = cloud.run(&["login", TOKEN], "");
    assert_eq!(
        out.code,
        Some(2),
        "clap refuses an argument: {}",
        out.stderr
    );
    assert!(!cloud.credentials().exists());
}

#[test]
fn login_from_a_token_file() {
    let fake = Fake::start(|_| ok(token_info("read", Some(("p1", "alpha")))));
    let cloud = Cloud::new();
    let file = cloud.home.path().join("token");
    std::fs::write(&file, format!("{TOKEN}\n")).unwrap();
    let out = cloud.run(
        &[
            "--api",
            &fake.url(),
            "--token-file",
            file.to_str().unwrap(),
            "login",
        ],
        "",
    );
    assert!(out.success(), "{}", out.stderr);
    assert!(
        out.stdout.contains("alpha (p1)"),
        "the project restriction is shown: {}",
        out.stdout
    );
    assert_eq!(fake.seen()[0].auth(), Some(bearer().as_str()));
    assert!(cloud.credentials().exists());
}

#[test]
fn a_later_command_uses_the_stored_token_and_host_and_logout_forgets_them() {
    let fake = Fake::start(|request| match request.path() {
        "/v1/whoami" => ok(token_info("read", None)),
        _ => ok(projects_body()),
    });
    let cloud = Cloud::new();
    assert!(cloud.run(&["--api", &fake.url(), "login"], TOKEN).success());
    // No --api, no token: both come from the file.
    let out = cloud.run(&["projects"], "");
    assert!(out.success(), "{}", out.stderr);
    let seen = fake.seen();
    assert_eq!(seen[1].path(), "/v1/projects");
    assert_eq!(seen[1].auth(), Some(bearer().as_str()));

    let out = cloud.run(&["logout"], "");
    assert!(out.success(), "{}", out.stderr);
    assert!(!cloud.credentials().exists());
    assert_no_token(&out);
    // Logged out: the command stops before any request.
    let before = fake.seen().len();
    let out = cloud.run(&["projects"], "");
    assert_eq!(out.code, Some(1));
    assert!(out.stderr.contains("zega-server cloud login"), "{}", out.stderr);
    assert_eq!(fake.seen().len(), before, "no request without a token");
    // Logging out twice is not an error.
    assert!(cloud.run(&["logout"], "").success());
}

#[test]
fn a_stored_token_is_never_sent_to_another_host() {
    let stored_for = Fake::start(|_| ok(projects_body()));
    let other = Fake::start(|_| ok(projects_body()));
    let cloud = Cloud::new();
    cloud.log_in(&stored_for.url(), TOKEN);
    let out = cloud.run(&["--api", &other.url(), "projects"], "");
    assert_eq!(out.code, Some(1));
    assert!(out.stderr.contains("logged in to"), "{}", out.stderr);
    assert!(other.seen().is_empty() && stored_for.seen().is_empty());
    assert_no_token(&out);
}

#[test]
fn token_file_is_used_instead_of_the_stored_token() {
    let fake = Fake::start(|_| ok(projects_body()));
    let cloud = Cloud::new();
    cloud.log_in("https://cloud.zega.world", "zc_stored_elsewhere");
    let file = cloud.home.path().join("ci-token");
    std::fs::write(&file, format!("{TOKEN}\n")).unwrap();
    let out = cloud.run(
        &[
            "--api",
            &fake.url(),
            "--token-file",
            file.to_str().unwrap(),
            "projects",
        ],
        "",
    );
    assert!(out.success(), "{}", out.stderr);
    assert_eq!(
        fake.seen()[0].auth(),
        Some(bearer().as_str()),
        "the file's token, not the stored one"
    );
    assert_no_token(&out);
}

#[test]
fn a_bad_token_file_fails_before_any_request() {
    let fake = Fake::start(|_| ok(projects_body()));
    let cloud = Cloud::new();
    let empty = cloud.home.path().join("empty");
    std::fs::write(&empty, "\n").unwrap();
    let two = cloud.home.path().join("two");
    std::fs::write(&two, "zc_a zc_b\n").unwrap();
    for (file, why) in [
        (empty, "nonempty"),
        (two, "nonempty"),
        (cloud.home.path().join("missing"), "cannot read"),
    ] {
        let out = cloud.run(
            &[
                "--api",
                &fake.url(),
                "--token-file",
                file.to_str().unwrap(),
                "projects",
            ],
            "",
        );
        assert_eq!(out.code, Some(1), "{}", file.display());
        assert!(out.stderr.contains(why), "{}", out.stderr);
    }
    assert!(fake.seen().is_empty());
}

#[test]
fn a_token_is_only_sent_over_https_except_to_localhost() {
    let cloud = Cloud::new();
    let out = cloud.run(&["--api", "http://cloud.example.com", "login"], TOKEN);
    assert_eq!(out.code, Some(1));
    assert!(out.stderr.contains("https"), "{}", out.stderr);
    assert!(!cloud.credentials().exists());
}

// ---- reading ----------------------------------------------------------------------------------

#[test]
fn projects_prints_an_aligned_table_and_json_prints_the_apis_bytes() {
    let fake = Fake::start(|_| ok(projects_body()));
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run(&["projects"], "");
    assert!(out.success(), "{}", out.stderr);
    assert_eq!(
        out.stdout,
        [
            "ID   NAME   GRAPHS  CREATED",
            "p1   alpha       1  2026-09-30T12:00:00.000Z",
            "p22  beta        0  2026-10-01T08:30:00.000Z",
            "",
        ]
        .join("\n")
    );
    let out = cloud.run(&["projects", "--json"], "");
    assert!(out.success(), "{}", out.stderr);
    assert_eq!(
        out.stdout,
        format!("{}\n", projects_body()),
        "the API's body, unchanged"
    );
}

#[test]
fn a_resource_id_is_one_path_segment_and_shows_the_resource() {
    let fake = Fake::start(|request| match request.path() {
        "/v1/graphs/g0filmsfilmsfilms0" => {
            ok(json!({"graph": graph("g0filmsfilmsfilms0", "films")}))
        }
        _ => error(404, "graph_not_found", "No such graph."),
    });
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run(&["graphs", "g0filmsfilmsfilms0"], "");
    assert!(out.success(), "{}", out.stderr);
    for shown in [
        "films",
        "alpha (p1)",
        "yyz",
        "256mb (256 MB)",
        "monthly, $5.00 base price",
        "https://g0filmsfilmsfilms0.zegadb.com",
    ] {
        assert!(
            out.stdout.contains(shown),
            "{shown} missing from {}",
            out.stdout
        );
    }
    // A slash typed into an id cannot reach another route.
    let out = cloud.run(&["graphs", "../projects"], "");
    assert_eq!(out.code, Some(1));
    let seen = fake.seen();
    assert_eq!(seen[1].path(), "/v1/graphs/..%2Fprojects", "{:?}", seen[1]);
    assert!(
        out.stderr.contains("No such graph") && out.stderr.contains("graph_not_found"),
        "{}",
        out.stderr
    );
}

#[test]
fn every_read_command_asks_the_route_the_spec_names() {
    let fake = Fake::start(|request| match request.path() {
        "/v1/whoami" => ok(token_info("read", None)),
        "/v1/regions" => ok(
            json!({"regions": [{"id": "yyz", "city": "Toronto", "country": "CA", "continent": "NA"}]}),
        ),
        "/v1/usage" => ok(json!({
            "month": "2026-10", "toDateCents": 150, "projectedCents": 480, "worstCaseCents": 900,
            "graphs": [{"graph": "g0films", "month": "2026-10", "usage": {"queries": 1200, "writes": 30, "storageGb": 0.25, "egressGb": 1.5, "days": []},
                "included": {}, "overage": {"egressGb": {"extra": 0, "cents": 0}}, "nodes": {"count": 40, "included": 1000, "countedAt": null},
                "capCents": 0, "paused": true, "writesPaused": true, "toDate": {"lines": [], "totalCents": 150, "worstCaseCents": 900, "capReached": false, "upfrontCents": 0},
                "projected": {"lines": [], "totalCents": 480, "worstCaseCents": 900, "capReached": false, "upfrontCents": 0}, "projectedUsage": {}}]
        })),
        "/v1/buckets" => ok(
            json!({"buckets": [{"id": "b0photos", "name": "photos", "url": "https://b0photos.zegadb.com", "projectId": "p1", "projectName": "alpha", "bytesStored": 1500000, "capCents": 0, "createdAt": "2026-10-01T00:00:00.000Z"}]}),
        ),
        "/v1/functions" => ok(json!({"functions": [function(3)]})),
        "/v1/functions/f0abcdefghijklmnop" => ok(json!({"function": function(3)})),
        other => error(404, "not_found", &format!("no route {other}")),
    });
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let run = |args: &[&str]| {
        let out = cloud.run(args, "");
        assert!(out.success(), "{args:?}: {}", out.stderr);
        out.stdout
    };
    assert!(run(&["whoami"]).contains("Scope") && run(&["whoami"]).contains(&fake.url()));
    assert!(run(&["regions"]).contains("Toronto"));
    let usage = run(&["usage"]);
    for shown in [
        "2026-10",
        "$1.50",
        "$4.80",
        "$9.00",
        "g0films",
        "40/1000",
        "paused at cap",
    ] {
        assert!(usage.contains(shown), "{shown} missing from {usage}");
    }
    assert!(run(&["buckets"]).contains("1.5 MB"));
    let functions = run(&["functions"]);
    assert!(
        functions.contains("f0abcdefghijklmnop") && functions.contains("v3"),
        "{functions}"
    );
    let one = run(&["functions", "f0abcdefghijklmnop"]);
    for shown in ["REGION=yyz", "API_KEY", "10 ms CPU, 50 subrequests"] {
        assert!(one.contains(shown), "{shown} missing from {one}");
    }
    let paths: Vec<String> = fake
        .seen()
        .iter()
        .map(|seen| format!("{} {}", seen.method, seen.path()))
        .collect();
    assert_eq!(
        paths,
        [
            "GET /v1/whoami",
            "GET /v1/whoami",
            "GET /v1/regions",
            "GET /v1/usage",
            "GET /v1/buckets",
            "GET /v1/functions",
            "GET /v1/functions/f0abcdefghijklmnop"
        ]
    );
    assert!(fake
        .seen()
        .iter()
        .all(|seen| seen.auth() == Some(bearer().as_str())));
}

#[test]
fn function_logs_sends_the_filters_and_prints_requests_with_their_console_lines() {
    let fake = Fake::start(|_| {
        ok(json!({
            "function": "f0abcdefghijklmnop", "logs": "on", "from": "2026-10-01T00:00:00.000Z", "to": "2026-10-02T00:00:00.000Z",
            "retentionDays": 7, "cursor": "next-page-1",
            "entries": [{"id": "r1", "time": "2026-10-01T23:59:00.000Z", "method": "GET", "path": "/hello", "status": 500,
                "durationMs": 3, "cpuMs": 1, "outcome": "exception", "exception": "TypeError: x is undefined",
                "logs": [{"time": "2026-10-01T23:59:00.000Z", "level": "error", "message": "boom\nsecond line"}], "logsTruncated": false}]
        }))
    });
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run(
        &[
            "function",
            "logs",
            "f0abcdefghijklmnop",
            "--limit",
            "5",
            "--status",
            "500",
            "--search",
            "a b&c",
        ],
        "",
    );
    assert!(out.success(), "{}", out.stderr);
    let seen = fake.seen();
    assert_eq!(seen[0].path(), "/v1/functions/f0abcdefghijklmnop/logs");
    let query = seen[0].query();
    assert_eq!(query.get("limit").map(String::as_str), Some("5"));
    assert_eq!(query.get("status").map(String::as_str), Some("500"));
    assert_eq!(
        query.get("q").map(String::as_str),
        Some("a b&c"),
        "the search text survives encoding"
    );
    assert_eq!(query.len(), 3);
    for shown in [
        "GET /hello  500  3 ms  exception",
        "! TypeError: x is undefined",
        "error  boom",
        "second line",
        "--cursor next-page-1",
    ] {
        assert!(
            out.stdout.contains(shown),
            "{shown} missing from {}",
            out.stdout
        );
    }
    let out = cloud.run(
        &["function", "logs", "f0abcdefghijklmnop", "--limit", "500"],
        "",
    );
    assert_eq!(
        out.code,
        Some(2),
        "a limit past 200 is refused before any request"
    );
    assert_eq!(fake.seen().len(), 1);
}

// ---- managing functions -----------------------------------------------------------------------

#[test]
fn deploy_sends_the_files_bytes_as_the_module() {
    let fake = Fake::start(|_| ok(json!({"function": function(4)})));
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    // Sent exactly as it is on disk: not trimmed, not re-encoded (CRLF, non-ASCII, a final newline).
    let module: &[u8] =
        "export default { fetch() { return new Response('h\u{e9}llo\r\n'); } }\n".as_bytes();
    let file = cloud.home.path().join("worker.js");
    std::fs::write(&file, module).unwrap();
    let out = cloud.run(
        &[
            "function",
            "deploy",
            "f0abcdefghijklmnop",
            file.to_str().unwrap(),
        ],
        "",
    );
    assert!(out.success(), "{}", out.stderr);
    let seen = fake.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        (seen[0].method.as_str(), seen[0].path()),
        ("PUT", "/v1/functions/f0abcdefghijklmnop/code")
    );
    assert_eq!(
        seen[0].headers.get("content-type").map(String::as_str),
        Some("application/javascript")
    );
    assert_eq!(seen[0].auth(), Some(bearer().as_str()));
    assert_eq!(seen[0].body, module);
    assert!(
        out.stdout
            .contains("deployed hello (f0abcdefghijklmnop) as version 4"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("https://f0abcdefghijklmnop.zegadb.com"),
        "{}",
        out.stdout
    );
    let out = cloud.run(
        &[
            "function",
            "deploy",
            "f0abcdefghijklmnop",
            "--json",
            file.to_str().unwrap(),
        ],
        "",
    );
    assert_eq!(
        out.stdout,
        format!("{}\n", json!({"function": function(4)}))
    );
}

#[test]
fn a_module_the_platform_refuses_is_reported_and_fails() {
    let fake = Fake::start(|_| {
        error(
            400,
            "code_rejected",
            "Uncaught SyntaxError: Unexpected token '}' at worker.js:3",
        )
    });
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let file = cloud.home.path().join("worker.js");
    std::fs::write(&file, "}").unwrap();
    let out = cloud.run(
        &[
            "function",
            "deploy",
            "f0abcdefghijklmnop",
            file.to_str().unwrap(),
        ],
        "",
    );
    assert_eq!(out.code, Some(1));
    assert!(
        out.stderr.contains("Unexpected token") && out.stderr.contains("code_rejected"),
        "{}",
        out.stderr
    );
    let out = cloud.run(
        &[
            "function",
            "deploy",
            "f0abcdefghijklmnop",
            "no-such-file.js",
        ],
        "",
    );
    assert_eq!(out.code, Some(1));
    assert!(out.stderr.contains("no-such-file.js"), "{}", out.stderr);
    assert_eq!(
        fake.seen().len(),
        1,
        "a missing file is caught before any request"
    );
}

#[test]
fn variables_and_secrets_use_the_routes_and_bodies_the_spec_gives() {
    let fake = Fake::start(|_| ok(json!({"function": function(2)})));
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let id = "f0abcdefghijklmnop";
    assert!(cloud
        .run(&["function", "var", "set", id, "REGION", "yyz east"], "")
        .success());
    assert!(cloud
        .run(&["function", "var", "unset", id, "REGION"], "")
        .success());
    // The secret's value is read from stdin, minus the one line break that ends it.
    let secret = "s3cr3t-value\nwith a second line";
    let out = cloud.run(
        &["function", "secret", "set", id, "API_KEY"],
        &format!("{secret}\n"),
    );
    assert!(out.success(), "{}", out.stderr);
    assert!(
        !out.stdout.contains("s3cr3t") && !out.stderr.contains("s3cr3t"),
        "the secret is never echoed"
    );
    assert!(cloud
        .run(&["function", "secret", "unset", id, "API_KEY"], "")
        .success());
    let seen = fake.seen();
    let shape: Vec<(String, String)> = seen
        .iter()
        .map(|seen| (seen.method.clone(), seen.path().to_string()))
        .collect();
    assert_eq!(
        shape,
        [
            ("PUT", format!("/v1/functions/{id}/vars/REGION")),
            ("DELETE", format!("/v1/functions/{id}/vars/REGION")),
            ("PUT", format!("/v1/functions/{id}/secrets/API_KEY")),
            ("DELETE", format!("/v1/functions/{id}/secrets/API_KEY")),
        ]
        .map(|(method, path)| (method.to_string(), path))
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&seen[0].body).unwrap(),
        json!({"value": "yyz east"})
    );
    assert!(seen[1].body.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&seen[2].body).unwrap(),
        json!({"value": secret})
    );
    assert!(seen
        .iter()
        .all(|seen| seen.auth() == Some(bearer().as_str())));
    // An empty secret (a closed stdin) is not sent.
    let out = cloud.run(&["function", "secret", "set", id, "EMPTY"], "");
    assert_eq!(out.code, Some(1));
    assert_eq!(fake.seen().len(), 4);
}

#[test]
fn the_secret_value_is_not_an_argument_the_cli_accepts() {
    let cloud = Cloud::new();
    let out = cloud.run(
        &[
            "function",
            "secret",
            "set",
            "f0abcdefghijklmnop",
            "API_KEY",
            "oops-in-argv",
        ],
        "",
    );
    assert_eq!(out.code, Some(2), "{}", out.stderr);
}

// ---- managing: renames, creates, deletes ------------------------------------------------------

fn requests(fake: &Fake) -> Vec<String> {
    fake.seen()
        .iter()
        .map(|seen| format!("{} {}", seen.method, seen.path()))
        .collect()
}

#[test]
fn every_manage_route_is_one_command_and_sends_what_the_spec_says() {
    // The manage routes of https://cloud.zega.dev/openapi.json (`x-zega-scope: manage`).
    let spec = [
        "DELETE /v1/projects/{project}",
        "PUT /v1/projects/{project}/name",
        "DELETE /v1/graphs/{graph}",
        "PUT /v1/graphs/{graph}/name",
        "POST /v1/graphs/{graph}/keys",
        "DELETE /v1/graphs/{graph}/keys/{key}",
        "POST /v1/graphs/{graph}/domains",
        "DELETE /v1/graphs/{graph}/domains/{hostname}",
        "PUT /v1/graphs/{graph}/monitoring/settings",
        "POST /v1/projects/{project}/buckets",
        "DELETE /v1/buckets/{bucket}",
        "PUT /v1/buckets/{bucket}/name",
        "POST /v1/buckets/{bucket}/keys",
        "DELETE /v1/buckets/{bucket}/keys/{key}",
        "POST /v1/projects/{project}/functions",
        "DELETE /v1/functions/{function}",
        "PUT /v1/functions/{function}/name",
        "PUT /v1/functions/{function}/code",
        "PUT /v1/functions/{function}/logs",
        "PUT /v1/functions/{function}/vars/{name}",
        "DELETE /v1/functions/{function}/vars/{name}",
        "PUT /v1/functions/{function}/secrets/{name}",
        "DELETE /v1/functions/{function}/secrets/{name}",
    ];
    let f = FUNCTION;
    // (command line, stdin, the route it must reach, the JSON body it must send)
    let commands: Vec<(Vec<&str>, &str, &str, Option<Value>)> = vec![
        (
            vec!["project", "delete", "p1", "--yes"],
            "",
            "DELETE /v1/projects/{project}",
            None,
        ),
        (
            vec!["project", "rename", "p1", "beta"],
            "",
            "PUT /v1/projects/{project}/name",
            Some(json!({"name": "beta"})),
        ),
        (
            vec!["graph", "delete", GRAPH, "--yes"],
            "",
            "DELETE /v1/graphs/{graph}",
            None,
        ),
        (
            vec!["graph", "rename", GRAPH, "movies"],
            "",
            "PUT /v1/graphs/{graph}/name",
            Some(json!({"name": "movies"})),
        ),
        (
            vec!["graph", "key", "create", GRAPH, "--name", "ci"],
            "",
            "POST /v1/graphs/{graph}/keys",
            Some(json!({"name": "ci"})),
        ),
        (
            vec!["graph", "key", "revoke", GRAPH, "key_a1"],
            "",
            "DELETE /v1/graphs/{graph}/keys/{key}",
            None,
        ),
        (
            vec!["graph", "domain", "add", GRAPH, "films.example.com"],
            "",
            "POST /v1/graphs/{graph}/domains",
            Some(json!({"hostname": "films.example.com"})),
        ),
        (
            vec!["graph", "domain", "remove", GRAPH, "films.example.com"],
            "",
            "DELETE /v1/graphs/{graph}/domains/{hostname}",
            None,
        ),
        (
            vec![
                "graph",
                "monitoring",
                "set",
                GRAPH,
                "--keep-query-text",
                "on",
            ],
            "",
            "PUT /v1/graphs/{graph}/monitoring/settings",
            Some(json!({"keepQueryText": true})),
        ),
        (
            vec!["bucket", "create", "p1", "photos"],
            "",
            "POST /v1/projects/{project}/buckets",
            Some(json!({"name": "photos"})),
        ),
        (
            vec!["bucket", "delete", "b0photos", "--yes"],
            "",
            "DELETE /v1/buckets/{bucket}",
            None,
        ),
        (
            vec!["bucket", "rename", "b0photos", "pics"],
            "",
            "PUT /v1/buckets/{bucket}/name",
            Some(json!({"name": "pics"})),
        ),
        (
            vec!["bucket", "key", "create", "b0photos"],
            "",
            "POST /v1/buckets/{bucket}/keys",
            Some(json!({})),
        ),
        (
            vec!["bucket", "key", "revoke", "b0photos", "key_a1"],
            "",
            "DELETE /v1/buckets/{bucket}/keys/{key}",
            None,
        ),
        (
            vec!["function", "create", "p1", "hello"],
            "",
            "POST /v1/projects/{project}/functions",
            Some(json!({"name": "hello"})),
        ),
        (
            vec!["function", "delete", f, "--yes"],
            "",
            "DELETE /v1/functions/{function}",
            None,
        ),
        (
            vec!["function", "rename", f, "greeter"],
            "",
            "PUT /v1/functions/{function}/name",
            Some(json!({"name": "greeter"})),
        ),
        (
            vec!["function", "logging", f, "off"],
            "",
            "PUT /v1/functions/{function}/logs",
            Some(json!({"logs": "off"})),
        ),
        (
            vec!["function", "var", "set", f, "REGION", "yyz"],
            "",
            "PUT /v1/functions/{function}/vars/{name}",
            Some(json!({"value": "yyz"})),
        ),
        (
            vec!["function", "var", "unset", f, "REGION"],
            "",
            "DELETE /v1/functions/{function}/vars/{name}",
            None,
        ),
        (
            vec!["function", "secret", "set", f, "API_KEY"],
            "hush\n",
            "PUT /v1/functions/{function}/secrets/{name}",
            Some(json!({"value": "hush"})),
        ),
        (
            vec!["function", "secret", "unset", f, "API_KEY"],
            "",
            "DELETE /v1/functions/{function}/secrets/{name}",
            None,
        ),
    ];
    let fake = Fake::start(manage_api);
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let file = cloud.home.path().join("worker.js");
    std::fs::write(&file, "export default {}").unwrap();
    for (args, stdin, _, _) in &commands {
        let out = cloud.run(args, stdin);
        assert!(out.success(), "{args:?}: {}", out.stderr);
        assert_no_token(&out);
    }
    let deploy = cloud.run(&["function", "deploy", f, file.to_str().unwrap()], "");
    assert!(deploy.success(), "{}", deploy.stderr);

    let seen = fake.seen();
    assert_eq!(
        seen.len(),
        commands.len() + 1,
        "one request per command: {:?}",
        requests(&fake)
    );
    for (seen, (args, _, route, body)) in seen.iter().zip(&commands) {
        assert!(
            matches_route(route, seen),
            "{args:?} sent {} {}, wanted {route}",
            seen.method,
            seen.path()
        );
        assert_eq!(seen.auth(), Some(bearer().as_str()), "{args:?}");
        match body {
            Some(body) => assert_eq!(
                &serde_json::from_slice::<Value>(&seen.body).unwrap(),
                body,
                "{args:?}"
            ),
            None => assert!(seen.body.is_empty(), "{args:?} sent a body"),
        }
    }
    // The commands above, with the deploy, reach every manage route of the spec and no other.
    let mut reached: Vec<&str> = commands.iter().map(|(_, _, route, _)| *route).collect();
    reached.push("PUT /v1/functions/{function}/code");
    reached.sort_unstable();
    let mut wanted = spec.to_vec();
    wanted.sort_unstable();
    assert_eq!(reached, wanted);
}

#[test]
fn a_rename_says_what_the_resource_is_now_called_and_json_is_the_apis_body() {
    let fake = Fake::start(manage_api);
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    for (args, shown) in [
        (
            vec!["project", "rename", "p1", "beta"],
            "renamed project p1 to beta\n",
        ),
        (
            vec!["graph", "rename", GRAPH, "movies"],
            "renamed graph g0filmsfilmsfilms0 to movies\n",
        ),
        (
            vec!["bucket", "rename", "b0photos", "pics"],
            "renamed bucket b0photos to pics\n",
        ),
        (
            vec!["function", "rename", FUNCTION, "greeter"],
            "renamed function f0abcdefghijklmnop to greeter\n",
        ),
    ] {
        let out = cloud.run(&args, "");
        assert!(out.success(), "{args:?}: {}", out.stderr);
        assert_eq!(out.stdout, shown);
    }
    let out = cloud.run(&["project", "rename", "p1", "beta", "--json"], "");
    assert_eq!(
        out.stdout,
        format!("{}\n", json!({"project": project_named("beta")}))
    );
}

#[test]
fn creating_a_bucket_and_a_function_prints_the_new_address() {
    let fake = Fake::start(manage_api);
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run(&["bucket", "create", "p1", "photos"], "");
    assert!(out.success(), "{}", out.stderr);
    assert!(
        out.stdout.contains("created bucket photos (b0photos)")
            && out.stdout.contains("https://b0photos.zegadb.com"),
        "{}",
        out.stdout
    );
    let out = cloud.run(&["function", "create", "p1", "hello"], "");
    assert!(out.success(), "{}", out.stderr);
    assert!(
        out.stdout
            .contains("created function hello (f0abcdefghijklmnop)")
            && out.stdout.contains("https://f0abcdefghijklmnop.zegadb.com")
            && out.stdout.contains("404 until you deploy"),
        "{}",
        out.stdout
    );
    let out = cloud.run(&["function", "create", "p1", "hello", "--json"], "");
    let mut fresh = named_function("hello");
    fresh["deployed"] = json!(false);
    fresh["version"] = json!(0);
    assert_eq!(out.stdout, format!("{}\n", json!({"function": fresh})));
}

const DELETES: [(&[&str], &str, &str); 4] = [
    (
        &["project", "delete", "p1"],
        "/v1/projects/p1",
        "deleted project alpha (p1)",
    ),
    (
        &["graph", "delete", GRAPH],
        "/v1/graphs/g0filmsfilmsfilms0",
        "deleted graph films (g0filmsfilmsfilms0)",
    ),
    (
        &["bucket", "delete", "b0photos"],
        "/v1/buckets/b0photos",
        "deleted bucket photos (b0photos)",
    ),
    (
        &["function", "delete", FUNCTION],
        "/v1/functions/f0abcdefghijklmnop",
        "deleted function hello (f0abcdefghijklmnop)",
    ),
];

#[test]
fn a_delete_with_no_terminal_and_no_yes_refuses_and_sends_nothing() {
    let fake = Fake::start(manage_api);
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    for (args, _, _) in DELETES {
        // Piping the id in is not a confirmation: only a person at a terminal, or --yes.
        let id = args[2];
        let out = cloud.run(args, &format!("{id}\n"));
        assert_eq!(out.code, Some(1), "{args:?}");
        assert!(
            out.stderr.contains("--yes") && out.stderr.contains("Nothing was deleted"),
            "{}",
            out.stderr
        );
        assert!(out.stdout.is_empty(), "{}", out.stdout);
    }
    assert!(
        fake.seen().is_empty(),
        "not even a lookup before the answer: {:?}",
        requests(&fake)
    );
}

#[test]
fn a_delete_with_yes_sends_one_delete_and_does_not_ask() {
    let fake = Fake::start(manage_api);
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    for (args, path, shown) in DELETES {
        let mut args = args.to_vec();
        args.push("--yes");
        let out = cloud.run(&args, "");
        assert!(out.success(), "{args:?}: {}", out.stderr);
        assert!(out.stdout.contains(shown), "{}", out.stdout);
        assert!(
            !out.stderr.contains("Type "),
            "it must not ask: {}",
            out.stderr
        );
        let seen = fake.seen();
        let last = seen.last().unwrap();
        assert_eq!((last.method.as_str(), last.path()), ("DELETE", path));
        assert_eq!(last.auth(), Some(bearer().as_str()));
    }
    assert_eq!(fake.seen().len(), DELETES.len(), "no lookup with --yes");
    // A paid graph is not gone at once: the output says when its data is.
    let out = cloud.run(&["graph", "delete", GRAPH, "--yes"], "");
    assert!(
        out.stdout
            .contains("its data is deleted on 2026-11-01T00:00:00.000Z"),
        "{}",
        out.stdout
    );
    let out = cloud.run(&["bucket", "delete", "b0photos", "--yes", "--json"], "");
    assert_eq!(
        out.stdout,
        format!("{}\n", json!({"bucket": bucket("photos")}))
    );
}

#[cfg(unix)]
#[test]
fn at_a_terminal_a_delete_shows_what_goes_and_needs_the_id_typed() {
    let fake = Fake::start(manage_api);
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    for (args, path, shown) in DELETES {
        let id = args[2];
        // Anything but the id cancels, and nothing is deleted.
        for wrong in ["yes\n", "y\n", "\n", "p\n", &format!("{id}x\n")] {
            let before = fake.seen().len();
            let out = cloud.run_at_a_terminal(args, wrong);
            assert_eq!(
                out.code,
                Some(1),
                "{args:?} typed {wrong:?}: {}",
                out.stderr
            );
            assert!(
                out.stderr.contains("not confirmed: nothing was deleted"),
                "{}",
                out.stderr
            );
            assert!(out.stdout.is_empty(), "{}", out.stdout);
            let sent: Vec<_> = fake.seen()[before..].to_vec();
            assert_eq!(sent.len(), 1, "only the lookup: {sent:?}");
            assert_eq!((sent[0].method.as_str(), sent[0].path()), ("GET", path));
        }
        // The id, typed, deletes.
        let before = fake.seen().len();
        let out = cloud.run_at_a_terminal(args, &format!("{id}\n"));
        assert!(out.success(), "{args:?}: {}", out.stderr);
        assert!(out.stdout.contains(shown), "{}", out.stdout);
        let sent: Vec<_> = fake.seen()[before..].to_vec();
        assert_eq!(
            sent.iter()
                .map(|s| (s.method.as_str(), s.path()))
                .collect::<Vec<_>>(),
            [("GET", path), ("DELETE", path)],
            "{args:?}"
        );
        // What it said before it asked: what, by name and id, and what it does.
        assert!(
            out.stderr.contains(&format!("Type {id} to delete it")),
            "{}",
            out.stderr
        );
    }
    let out = cloud.run_at_a_terminal(&["project", "delete", "p1"], "p1\n");
    for shown in [
        "About to delete the project alpha (p1)",
        "graph: films (g0filmsfilmsfilms0)",
        "30 days later",
    ] {
        assert!(
            out.stderr.contains(shown),
            "{shown} missing from {}",
            out.stderr
        );
    }
}

#[cfg(unix)]
#[test]
fn a_confirmed_delete_with_json_keeps_stdout_the_apis_body() {
    let fake = Fake::start(manage_api);
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run_at_a_terminal(
        &["function", "delete", FUNCTION, "--json"],
        &format!("{FUNCTION}\n"),
    );
    assert!(out.success(), "{}", out.stderr);
    assert_eq!(
        out.stdout,
        format!("{}\n", json!({"function": named_function("hello")}))
    );
}

#[test]
fn a_delete_the_platform_refuses_is_reported_and_fails() {
    let fake = Fake::start(|request| match request.method.as_str() {
        "DELETE" if request.path().starts_with("/v1/buckets") => error(
            409,
            "bucket_not_empty",
            "That bucket still holds objects: empty it first, or delete the project.",
        ),
        "DELETE" => error(
            409,
            "last_graph",
            "This is the project's only graph: delete the project.",
        ),
        _ => error(404, "not_found", "no"),
    });
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run(&["bucket", "delete", "b0photos", "--yes"], "");
    assert_eq!(out.code, Some(1));
    assert!(
        out.stderr.contains("still holds objects") && out.stderr.contains("bucket_not_empty"),
        "{}",
        out.stderr
    );
    let out = cloud.run(&["graph", "delete", GRAPH, "--yes"], "");
    assert_eq!(out.code, Some(1));
    assert!(
        out.stderr.contains("last_graph") && out.stderr.contains("delete the project"),
        "{}",
        out.stderr
    );
}

// ---- managing: keys, domains, monitoring, logs, code ------------------------------------------

#[test]
fn a_new_keys_secret_is_printed_once_and_marked_as_shown_once() {
    let fake = Fake::start(manage_api);
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    for (args, secret, holder) in [
        (
            &["graph", "key", "create", GRAPH, "--name", "ci"][..],
            "zk_NEWKSECRETSECRETSECRET234567",
            "graph g0filmsfilmsfilms0",
        ),
        (
            &["bucket", "key", "create", "b0photos"][..],
            "zs_NEWKSECRETSECRETSECRET234567",
            "bucket b0photos",
        ),
    ] {
        let out = cloud.run(args, "");
        assert!(out.success(), "{args:?}: {}", out.stderr);
        assert_eq!(
            out.stdout.matches(secret).count(),
            1,
            "the secret appears once: {}",
            out.stdout
        );
        assert!(
            !out.stderr.contains(secret),
            "never on stderr: {}",
            out.stderr
        );
        assert!(out.stdout.contains("shown once"), "{}", out.stdout);
        assert!(
            out.stdout.contains(holder) && out.stdout.contains("key_new"),
            "{}",
            out.stdout
        );
        assert_no_token(&out);
    }
    // The label goes in the body when given and the body is `{}` when not.
    let seen = fake.seen();
    assert_eq!(
        serde_json::from_slice::<Value>(&seen[0].body).unwrap(),
        json!({"name": "ci"})
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&seen[1].body).unwrap(),
        json!({})
    );
    // --json: the API's answer unchanged, secret and `shownOnce` included.
    let out = cloud.run(
        &["graph", "key", "create", GRAPH, "--name", "ci", "--json"],
        "",
    );
    assert_eq!(
        out.stdout,
        format!(
            "{}\n",
            json!({"key": {"id": "key_new", "name": "ci", "prefix": "zk_NEWK", "createdAt": "2026-10-02T10:00:00.000Z",
                "secret": "zk_NEWKSECRETSECRETSECRET234567"}, "shownOnce": true})
        )
    );
}

#[test]
fn keys_are_listed_without_secrets_and_revoked_by_id() {
    let fake = Fake::start(manage_api);
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run(&["graph", "key", "list", GRAPH], "");
    assert!(out.success(), "{}", out.stderr);
    assert_eq!(
        out.stdout,
        [
            "ID      NAME  PREFIX      CREATED                   LAST USED                 REVOKED",
            "key_a1  web   zk_AAAA...  2026-10-01T10:00:00.000Z  never                     -",
            "key_b2  old   zk_BBBB...  2026-09-01T00:00:00.000Z  2026-09-20T00:00:00.000Z  2026-09-25T00:00:00.000Z",
            "",
        ]
        .join("\n")
    );
    assert!(cloud
        .run(&["bucket", "key", "list", "b0photos"], "")
        .success());
    let out = cloud.run(&["bucket", "key", "revoke", "b0photos", "key_a1"], "");
    assert_eq!(out.stdout, "revoked key key_a1 of bucket b0photos\n");
    assert_eq!(
        requests(&fake),
        [
            "GET /v1/graphs/g0filmsfilmsfilms0/keys",
            "GET /v1/buckets/b0photos/keys",
            "DELETE /v1/buckets/b0photos/keys/key_a1"
        ]
    );
    let out = cloud.run(&["graph", "key", "revoke", GRAPH, "key_zz"], "");
    assert_eq!(
        out.stdout,
        "revoked key key_zz of graph g0filmsfilmsfilms0\n"
    );
}

#[test]
fn a_key_that_does_not_exist_is_reported() {
    let fake = Fake::start(|_| error(404, "key_not_found", "No such key on this graph."));
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run(&["graph", "key", "revoke", GRAPH, "key_zz"], "");
    assert_eq!(out.code, Some(1));
    assert!(
        out.stderr.contains("No such key") && out.stderr.contains("key_not_found"),
        "{}",
        out.stderr
    );
}

#[test]
fn custom_domains_are_listed_added_with_their_dns_record_and_removed() {
    let fake = Fake::start(manage_api);
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run(&["graph", "domain", "list", GRAPH], "");
    assert!(out.success(), "{}", out.stderr);
    assert_eq!(
        out.stdout,
        [
            "2 of 5 domains; each one's CNAME points to edge.zegadb.com",
            "",
            "HOSTNAME           STATE                                     DNS RECORD",
            "films.example.com  active                                    CNAME films.example.com -> edge.zegadb.com",
            "new.example.com    pending (certificate pending_validation)  CNAME new.example.com -> edge.zegadb.com",
            "",
        ]
        .join("\n")
    );
    let out = cloud.run(&["graph", "domain", "add", GRAPH, "graph.example.com"], "");
    assert!(out.success(), "{}", out.stderr);
    for shown in [
        "added domain graph.example.com to graph g0filmsfilmsfilms0",
        "Set this DNS record at your DNS provider: CNAME graph.example.com -> edge.zegadb.com",
    ] {
        assert!(
            out.stdout.contains(shown),
            "{shown} missing from {}",
            out.stdout
        );
    }
    let out = cloud.run(
        &["graph", "domain", "remove", GRAPH, "graph.example.com"],
        "",
    );
    assert_eq!(
        out.stdout,
        "removed domain graph.example.com from graph g0filmsfilmsfilms0\n"
    );
    let seen = fake.seen();
    assert_eq!(
        serde_json::from_slice::<Value>(&seen[1].body).unwrap(),
        json!({"hostname": "graph.example.com"})
    );
    assert_eq!(
        requests(&fake),
        [
            "GET /v1/graphs/g0filmsfilmsfilms0/domains",
            "POST /v1/graphs/g0filmsfilmsfilms0/domains",
            "DELETE /v1/graphs/g0filmsfilmsfilms0/domains/graph.example.com"
        ]
    );
}

#[test]
fn monitoring_settings_are_shown_and_set() {
    let fake = Fake::start(manage_api);
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run(&["graph", "monitoring", "show", GRAPH], "");
    assert!(out.success(), "{}", out.stderr);
    for shown in ["Keep query text   off", "Request log kept", "7 days"] {
        assert!(
            out.stdout.contains(shown),
            "{shown} missing from {}",
            out.stdout
        );
    }
    let out = cloud.run(
        &[
            "graph",
            "monitoring",
            "set",
            GRAPH,
            "--keep-query-text",
            "on",
        ],
        "",
    );
    assert!(out.success(), "{}", out.stderr);
    assert!(
        out.stdout.contains("on (24 hours, for the owner only)"),
        "{}",
        out.stdout
    );
    let seen = fake.seen();
    assert_eq!(
        (seen[1].method.as_str(), seen[1].path()),
        ("PUT", "/v1/graphs/g0filmsfilmsfilms0/monitoring/settings")
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&seen[1].body).unwrap(),
        json!({"keepQueryText": true})
    );
    let out = cloud.run(
        &[
            "graph",
            "monitoring",
            "set",
            GRAPH,
            "--keep-query-text",
            "off",
        ],
        "",
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fake.seen()[2].body).unwrap(),
        json!({"keepQueryText": false})
    );
    assert!(
        out.stdout.contains("Keep query text   off"),
        "{}",
        out.stdout
    );
    // Neither an unknown word nor a missing setting is sent.
    for args in [
        &[
            "graph",
            "monitoring",
            "set",
            GRAPH,
            "--keep-query-text",
            "maybe",
        ][..],
        &["graph", "monitoring", "set", GRAPH][..],
    ] {
        assert_eq!(cloud.run(args, "").code, Some(2), "{args:?}");
    }
    assert_eq!(fake.seen().len(), 3);
}

#[test]
fn function_logging_turns_the_logs_on_and_off() {
    let fake = Fake::start(manage_api);
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run(&["function", "logging", FUNCTION, "on"], "");
    assert!(out.success(), "{}", out.stderr);
    assert_eq!(out.stdout, "logs are on for hello (f0abcdefghijklmnop)\n");
    let out = cloud.run(&["function", "logging", FUNCTION, "off"], "");
    assert_eq!(out.stdout, "logs are off for hello (f0abcdefghijklmnop)\n");
    let seen = fake.seen();
    assert_eq!(
        (seen[0].method.as_str(), seen[0].path()),
        ("PUT", "/v1/functions/f0abcdefghijklmnop/logs")
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&seen[0].body).unwrap(),
        json!({"logs": "on"})
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&seen[1].body).unwrap(),
        json!({"logs": "off"})
    );
    assert_eq!(
        cloud
            .run(&["function", "logging", FUNCTION, "sometimes"], "")
            .code,
        Some(2)
    );
    assert_eq!(fake.seen().len(), 2);
}

/// What `function deploy` was last given: CRLF, non-ASCII, no final newline.
const MODULE: &str = "export default { fetch() { return new Response('h\u{e9}llo \u{1f600}\\n'); } }\r\n// no final newline";

#[test]
fn function_code_prints_the_module_exactly_and_out_writes_the_same_bytes() {
    let fake = Fake::start(|_| ok(json!({"code": MODULE})));
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run(&["function", "code", FUNCTION], "");
    assert!(out.success(), "{}", out.stderr);
    assert_eq!(out.stdout, MODULE, "the module, not a byte more or less");
    let target = cloud.home.path().join("worker.js");
    std::fs::write(&target, "old contents that are longer than nothing").unwrap();
    let out = cloud.run(
        &[
            "function",
            "code",
            FUNCTION,
            "--out",
            target.to_str().unwrap(),
        ],
        "",
    );
    assert!(out.success(), "{}", out.stderr);
    assert_eq!(std::fs::read(&target).unwrap(), MODULE.as_bytes());
    assert!(
        out.stdout
            .contains(&format!("wrote {} bytes", MODULE.len())),
        "{}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("export default"),
        "the code is in the file, not on the screen"
    );
    let seen = fake.seen();
    assert_eq!(seen.len(), 2);
    assert!(seen.iter().all(|seen| (seen.method.as_str(), seen.path())
        == ("GET", "/v1/functions/f0abcdefghijklmnop/code")));
    assert!(seen
        .iter()
        .all(|seen| seen.auth() == Some(bearer().as_str())));
    // --json: the API's answer unchanged.
    let out = cloud.run(&["function", "code", FUNCTION, "--json"], "");
    assert_eq!(out.stdout, format!("{}\n", json!({"code": MODULE})));
}

#[test]
fn function_code_that_cannot_be_had_leaves_files_alone() {
    let fake = Fake::start(|request| match request.path() {
        "/v1/functions/f0abcdefghijklmnop/code" => ok(json!({"code": null})),
        _ => error(404, "function_not_found", "No such function."),
    });
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let target = cloud.home.path().join("worker.js");
    std::fs::write(&target, "keep me").unwrap();
    let path = target.to_str().unwrap();
    // Not deployed yet: a failure, and the existing file is not emptied.
    let out = cloud.run(&["function", "code", FUNCTION, "--out", path], "");
    assert_eq!(out.code, Some(1));
    assert!(
        out.stderr.contains("no deployed code yet"),
        "{}",
        out.stderr
    );
    let out = cloud.run(
        &["function", "code", "f0nosuchfunction000", "--out", path],
        "",
    );
    assert_eq!(out.code, Some(1));
    assert!(out.stderr.contains("function_not_found"), "{}", out.stderr);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep me");
    // --json carries the code on stdout, so it cannot also go to a file: refused before any request.
    let before = fake.seen().len();
    let out = cloud.run(&["function", "code", FUNCTION, "--out", path, "--json"], "");
    assert_eq!(out.code, Some(1));
    assert!(out.stderr.contains("--out"), "{}", out.stderr);
    assert_eq!(fake.seen().len(), before);
    // A file that cannot be written is an error, not a silent success.
    let out = cloud.run(
        &[
            "function",
            "code",
            FUNCTION,
            "--out",
            cloud.home.path().join("no/such/dir/x.js").to_str().unwrap(),
        ],
        "",
    );
    assert_eq!(out.code, Some(1));
}

// ---- errors -----------------------------------------------------------------------------------

#[test]
fn session_required_relays_the_apis_sentence_and_does_not_pretend() {
    let sentence = "Resizing a graph (its machine and its price) changes what you are billed, so it is done in the dashboard (https://dashboard.zega.dev), signed in; an API token cannot.";
    let fake = Fake::start(move |_| error(403, "session_required", sentence));
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    // `var set` is only the carrier here: any route can answer this code.
    let out = cloud.run(
        &["function", "var", "set", "f0abcdefghijklmnop", "A", "b"],
        "",
    );
    assert_eq!(out.code, Some(1));
    assert!(out.stderr.contains(sentence), "{}", out.stderr);
    assert!(out.stderr.contains("session_required"), "{}", out.stderr);
    assert_eq!(
        out.stderr.matches("dashboard").count(),
        sentence.matches("dashboard").count(),
        "the API already says where; nothing is added: {}",
        out.stderr
    );
    assert!(
        !out.stderr.contains("manage token") && !out.stderr.contains("zega-server cloud login"),
        "{}",
        out.stderr
    );
    assert_no_token(&out);
}

#[test]
fn scope_required_says_to_make_a_manage_token() {
    let fake = Fake::start(|_| {
        error(
            403,
            "scope_required",
            "This token can only read. Make a manage token to change things.",
        )
    });
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let file = cloud.home.path().join("worker.js");
    std::fs::write(&file, "export default {}").unwrap();
    let out = cloud.run(
        &[
            "function",
            "deploy",
            "f0abcdefghijklmnop",
            file.to_str().unwrap(),
        ],
        "",
    );
    assert_eq!(out.code, Some(1));
    assert!(
        out.stderr.contains("scope_required") && out.stderr.contains("can only read"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("needs a manage token"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("https://dashboard.zega.dev/tokens")
            && out.stderr.contains("zega-server cloud login"),
        "{}",
        out.stderr
    );
    assert_no_token(&out);
}

#[test]
fn a_new_command_relays_session_required_and_scope_required() {
    let sentence = "Creating a graph changes what you are billed, so it is done in the dashboard (https://dashboard.zega.dev), signed in; an API token cannot.";
    let fake = Fake::start(move |request| {
        if request.method == "POST" && request.path().starts_with("/v1/projects") {
            error(403, "session_required", sentence)
        } else {
            error(
                403,
                "scope_required",
                "This token can only read. Make a manage token to change things.",
            )
        }
    });
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run(&["bucket", "create", "p1", "photos"], "");
    assert_eq!(out.code, Some(1));
    assert!(
        out.stderr.contains(sentence) && out.stderr.contains("session_required"),
        "{}",
        out.stderr
    );
    assert_eq!(
        out.stderr.matches("dashboard").count(),
        sentence.matches("dashboard").count(),
        "the API already says where: {}",
        out.stderr
    );
    for args in [
        &["graph", "key", "create", GRAPH][..],
        &["project", "delete", "p1", "--yes"][..],
        &["function", "logging", FUNCTION, "on"][..],
    ] {
        let out = cloud.run(args, "");
        assert_eq!(out.code, Some(1), "{args:?}");
        assert!(
            out.stderr.contains("scope_required") && out.stderr.contains("needs a manage token"),
            "{args:?}: {}",
            out.stderr
        );
        assert!(
            !out.stdout.contains("created") && !out.stdout.contains("deleted"),
            "{}",
            out.stdout
        );
        assert_no_token(&out);
    }
}

#[test]
fn a_rejected_token_in_a_token_file_points_at_the_file() {
    let fake = Fake::start(|_| error(401, "token_expired", "This token expired on 2026-10-01."));
    let cloud = Cloud::new();
    let file = cloud.home.path().join("ci-token");
    std::fs::write(&file, TOKEN).unwrap();
    let out = cloud.run(
        &[
            "--api",
            &fake.url(),
            "--token-file",
            file.to_str().unwrap(),
            "projects",
        ],
        "",
    );
    assert_eq!(out.code, Some(1));
    assert!(
        out.stderr.contains("token_expired") && out.stderr.contains("expired on 2026-10-01"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("ci-token"),
        "names the file to fix: {}",
        out.stderr
    );
    assert!(!out.stderr.contains("zega-server cloud login"), "{}", out.stderr);
}

#[test]
fn a_429_says_how_long_to_wait_and_is_not_retried() {
    let fake = Fake::start(|_| {
        Answer {
        status: 429,
        headers: vec![("retry-after", "17".to_string())],
        body: json!({"ok": false, "error": "Too many requests: 300 a minute.", "code": "rate_limited", "retryAfterSeconds": 17}).to_string(),
    }
    });
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run(&["projects"], "");
    assert_eq!(out.code, Some(1));
    assert!(
        out.stderr.contains("Too many requests") && out.stderr.contains("rate_limited"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("Wait 17 seconds"), "{}", out.stderr);
    assert_eq!(fake.seen().len(), 1, "no automatic retry");
}

#[test]
fn a_503_says_how_long_to_wait_and_whether_the_change_took_effect() {
    let fake = Fake::start(|_| {
        Answer {
            status: 503,
            headers: vec![],
            body: json!({"ok": false, "error": "Zega Cloud is busy.", "code": "busy_try_again", "retryAfterSeconds": 3, "effect": "unknown"}).to_string(),
        }
    });
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run(&["function", "var", "unset", "f0abcdefghijklmnop", "A"], "");
    assert_eq!(out.code, Some(1));
    assert!(out.stderr.contains("Wait 3 seconds"), "{}", out.stderr);
    assert!(
        out.stderr
            .contains("not known whether the change took effect"),
        "{}",
        out.stderr
    );
    assert_eq!(fake.seen().len(), 1);
}

#[test]
fn an_answer_that_is_not_the_apis_still_fails_with_its_status() {
    let fake = Fake::start(|_| Answer {
        status: 502,
        headers: vec![],
        body: "<html>Bad gateway</html>".into(),
    });
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run(&["projects"], "");
    assert_eq!(out.code, Some(1));
    assert!(
        out.stderr.contains("HTTP 502") && out.stderr.contains("http_502"),
        "{}",
        out.stderr
    );
}

#[test]
fn an_unreachable_host_is_named_and_fails() {
    // A port that was open a moment ago and is closed now.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let cloud = Cloud::new();
    let api = format!("http://127.0.0.1:{port}");
    cloud.log_in(&api, TOKEN);
    let out = cloud.run(&["projects"], "");
    assert_eq!(out.code, Some(1));
    assert!(
        out.stderr
            .contains(&format!("could not reach 127.0.0.1:{port}")),
        "{}",
        out.stderr
    );
    assert_no_token(&out);
}

#[test]
fn a_redirect_is_never_followed_with_the_token() {
    let target = Fake::start(|_| ok(projects_body()));
    let target_url = target.url();
    let fake = Fake::start(move |_| Answer {
        status: 302,
        headers: vec![("location", format!("{target_url}/v1/projects"))],
        body: String::new(),
    });
    let cloud = Cloud::new();
    cloud.log_in(&fake.url(), TOKEN);
    let out = cloud.run(&["projects"], "");
    assert_eq!(out.code, Some(1));
    assert!(
        target.seen().is_empty(),
        "the token must not follow a redirect"
    );
}

// ---- help -------------------------------------------------------------------------------------

#[test]
fn help_lists_every_command_and_the_global_flags() {
    let cloud = Cloud::new();
    let help = cloud.run(&["--help"], "");
    assert!(help.success());
    for shown in [
        "login",
        "logout",
        "whoami",
        "projects",
        "graphs",
        "buckets",
        "functions",
        "usage",
        "regions",
        "project",
        "graph",
        "bucket",
        "function",
        "--api",
        "--token-file",
        "--json",
    ] {
        assert!(
            help.stdout.contains(shown),
            "{shown} missing from {}",
            help.stdout
        );
    }
    for (args, shown) in [
        (&["login", "--help"][..], "never from an argument"),
        (&["function", "--help"][..], "deploy"),
        (&["function", "deploy", "--help"][..], "1,000,000 bytes"),
        (
            &["function", "secret", "set", "--help"][..],
            "hidden prompt",
        ),
        (&["function", "logs", "--help"][..], "--limit"),
        (&["project", "delete", "--help"][..], "--yes"),
        (
            &["graph", "key", "create", "--help"][..],
            "cannot be read again",
        ),
        (&["function", "code", "--help"][..], "--out"),
        (&["function", "logging", "--help"][..], "billed"),
        (
            &["graph", "monitoring", "set", "--help"][..],
            "--keep-query-text",
        ),
    ] {
        let out = cloud.run(args, "");
        assert!(
            out.success() && out.stdout.contains(shown),
            "{args:?}: {}",
            out.stdout
        );
    }
}
