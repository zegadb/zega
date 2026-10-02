//! `zega cloud` against a fake Zega Cloud API: a real HTTP server on 127.0.0.1
//! (port 0) that records every request it gets and answers with bodies shaped
//! like the OpenAPI document's schemas (https://cloud.zega.dev/openapi.json:
//! `TokenInfo`, `Project`, `Graph`, `Bucket`, `Function`, `LogEntry`, `Error`).
//! The tests run the real `zega` binary and assert what reached the server and
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

const BIN: &str = env!("CARGO_BIN_EXE_zega");
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
    fn run(&self, args: &[&str], stdin: &str) -> Out {
        let mut child = Command::new(BIN)
            .arg("cloud")
            .args(args)
            .env("HOME", self.home.path())
            .env("APPDATA", self.home.path())
            .env_remove("XDG_CONFIG_HOME")
            .current_dir(self.home.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
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
    assert!(out.stderr.contains("zega cloud login"), "{}", out.stderr);
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
        !out.stderr.contains("manage token") && !out.stderr.contains("zega cloud login"),
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
            && out.stderr.contains("zega cloud login"),
        "{}",
        out.stderr
    );
    assert_no_token(&out);
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
    assert!(!out.stderr.contains("zega cloud login"), "{}", out.stderr);
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
    ] {
        let out = cloud.run(args, "");
        assert!(
            out.success() && out.stdout.contains(shown),
            "{args:?}: {}",
            out.stdout
        );
    }
}
