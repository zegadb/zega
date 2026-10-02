//! `zega cloud`: the command line for Zega Cloud's public management API
//! (`https://cloud.zega.dev/openapi.json`).
//!
//! The API takes an account API token, made in the dashboard, and nothing
//! else. What costs money (creating a graph or project, resizing, spending
//! caps, restoring a backup) and the tokens themselves are dashboard-only: the
//! API answers 403 `session_required` and this CLI relays that, it does not
//! pretend otherwise.

mod api;
mod credentials;
mod render;

use api::{normalize_api, ApiError, Body, Client, CloudError, Reply, DASHBOARD, DEFAULT_API};
use clap::{Args, Subcommand};
use serde_json::{json, Value};
use std::{
    io::{self, Write},
    path::PathBuf,
};

#[derive(Args)]
pub struct CloudArgs {
    /// The API address. Default https://cloud.zega.dev (staging:
    /// https://cloud.zega.world). `login` stores it with the token; later
    /// commands use the stored one.
    #[arg(long, global = true, value_name = "URL")]
    api: Option<String>,
    /// Use the token in this file instead of the one `login` stored: one
    /// nonempty token, a final newline is fine. For CI. Never an environment
    /// variable. With `login`, the token to store is read from this file.
    #[arg(long, global = true, value_name = "PATH")]
    token_file: Option<PathBuf>,
    /// Print the API's JSON unchanged instead of a table.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: CloudCommand,
}

#[derive(Subcommand)]
enum CloudCommand {
    /// Store an API token (made at https://dashboard.zega.dev/tokens). Reads
    /// it from stdin, or from a hidden prompt on a terminal; never from an
    /// argument. The token is checked with the API before it is saved.
    Login,
    /// Remove the stored token. The token itself stays valid until you revoke
    /// it in the dashboard.
    Logout,
    /// Show which token this is: name, scope, project restriction, expiry.
    Whoami,
    /// List projects, or show one with its graphs.
    Projects {
        /// A project id.
        id: Option<String>,
    },
    /// List graphs, or show one.
    Graphs {
        /// A graph id.
        id: Option<String>,
    },
    /// List storage buckets, or show one.
    Buckets {
        /// A bucket id.
        id: Option<String>,
    },
    /// List functions, or show one (its variables and secret names too).
    Functions {
        /// A function id (f0 and 16 characters).
        id: Option<String>,
    },
    /// This month's usage and bill per graph.
    Usage,
    /// The regions a graph can run in.
    Regions,
    /// Read a function's logs, deploy its code, set its variables and secrets.
    Function {
        #[command(subcommand)]
        command: FunctionCommand,
    },
}

#[derive(Subcommand)]
enum FunctionCommand {
    /// A page of a function's requests, newest first, with their console
    /// lines. Needs a read token.
    Logs {
        /// A function id.
        id: String,
        /// Start of the window: an ISO time or epoch milliseconds. Default: 24 hours before --to.
        #[arg(long)]
        from: Option<String>,
        /// End of the window. Default: now.
        #[arg(long)]
        to: Option<String>,
        /// Requests in the page, 1 to 200. Default 50.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=200))]
        limit: Option<u32>,
        /// Only requests that answered this HTTP status.
        #[arg(long, value_parser = clap::value_parser!(u32).range(100..=599))]
        status: Option<u32>,
        /// Search the method, path, error and every console line.
        #[arg(long)]
        search: Option<String>,
        /// The cursor a previous page printed.
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Deploy a file as the function's code: one ES module of JavaScript, at
    /// most 1,000,000 bytes (bundle TypeScript first). Needs a manage token.
    Deploy {
        /// A function id.
        id: String,
        /// The module to upload, sent as it is.
        file: PathBuf,
    },
    /// Plain-text variables. Needs a manage token.
    Var {
        #[command(subcommand)]
        command: VarCommand,
    },
    /// Secrets: set and removed, never read back. Needs a manage token.
    Secret {
        #[command(subcommand)]
        command: SecretCommand,
    },
}

#[derive(Subcommand)]
enum VarCommand {
    /// Set a variable (visible in `functions <id>`; for a secret use `secret set`).
    Set {
        /// A function id.
        id: String,
        /// 1 to 64 letters, digits or underscores, not starting with a digit.
        name: String,
        /// The value, at most 5,120 characters.
        value: String,
    },
    /// Remove a variable.
    Unset {
        /// A function id.
        id: String,
        /// The variable's name.
        name: String,
    },
}

#[derive(Subcommand)]
enum SecretCommand {
    /// Set a secret. The value comes from stdin, or a hidden prompt on a
    /// terminal; never from an argument, which any process listing shows.
    Set {
        /// A function id.
        id: String,
        /// 1 to 64 letters, digits or underscores, not starting with a digit.
        name: String,
    },
    /// Remove a secret.
    Unset {
        /// A function id.
        id: String,
        /// The secret's name.
        name: String,
    },
}

struct Session {
    client: Client,
    api: String,
}

/// Run a `zega cloud` command. Returns the process exit code: 0, or 1 after a
/// message on stderr.
pub fn run(args: CloudArgs) -> i32 {
    let api_for_hints = args.api.clone();
    let source_for_hints = args.token_file.clone();
    match execute(args) {
        Ok(()) => 0,
        Err(error) => {
            eprint!(
                "{}",
                describe(
                    &error,
                    api_for_hints.as_deref(),
                    source_for_hints.as_deref()
                )
            );
            1
        }
    }
}

fn execute(args: CloudArgs) -> Result<(), CloudError> {
    let json = args.json;
    match &args.command {
        CloudCommand::Login => return login(&args),
        CloudCommand::Logout => return logout(),
        _ => {}
    }
    let session = session(&args)?;
    let client = &session.client;
    match args.command {
        CloudCommand::Login | CloudCommand::Logout => {
            unreachable!("handled before a session is opened")
        }
        CloudCommand::Whoami => show(json, client.get(&["v1", "whoami"], &[])?, |body| {
            render::whoami(body, Some(&session.api))
        }),
        CloudCommand::Regions => show(json, client.get(&["v1", "regions"], &[])?, render::regions),
        CloudCommand::Usage => show(json, client.get(&["v1", "usage"], &[])?, render::usage),
        CloudCommand::Projects { id: None } => show(
            json,
            client.get(&["v1", "projects"], &[])?,
            render::projects,
        ),
        CloudCommand::Projects { id: Some(id) } => show(
            json,
            client.get(&["v1", "projects", &id], &[])?,
            render::project_one,
        ),
        CloudCommand::Graphs { id: None } => {
            show(json, client.get(&["v1", "graphs"], &[])?, render::graphs)
        }
        CloudCommand::Graphs { id: Some(id) } => show(
            json,
            client.get(&["v1", "graphs", &id], &[])?,
            render::graph_one,
        ),
        CloudCommand::Buckets { id: None } => {
            show(json, client.get(&["v1", "buckets"], &[])?, render::buckets)
        }
        CloudCommand::Buckets { id: Some(id) } => show(
            json,
            client.get(&["v1", "buckets", &id], &[])?,
            render::bucket_one,
        ),
        CloudCommand::Functions { id: None } => show(
            json,
            client.get(&["v1", "functions"], &[])?,
            render::functions,
        ),
        CloudCommand::Functions { id: Some(id) } => show(
            json,
            client.get(&["v1", "functions", &id], &[])?,
            render::function_one,
        ),
        CloudCommand::Function { command } => function(client, json, command),
    }
}

fn function(client: &Client, json: bool, command: FunctionCommand) -> Result<(), CloudError> {
    match command {
        FunctionCommand::Logs {
            id,
            from,
            to,
            limit,
            status,
            search,
            cursor,
        } => {
            let query: Vec<(&str, String)> = [
                ("from", from),
                ("to", to),
                ("limit", limit.map(|limit| limit.to_string())),
                ("status", status.map(|status| status.to_string())),
                ("q", search),
                ("cursor", cursor),
            ]
            .into_iter()
            .filter_map(|(name, value)| value.map(|value| (name, value)))
            .collect();
            show(
                json,
                client.get(&["v1", "functions", &id, "logs"], &query)?,
                render::logs,
            )
        }
        FunctionCommand::Deploy { id, file } => {
            let code = std::fs::read(&file)
                .map_err(|error| format!("cannot read {}: {error}", file.display()))?;
            let reply = client.call(
                "PUT",
                &["v1", "functions", &id, "code"],
                &[],
                Body::Module(&code),
            )?;
            done(json, reply, |function| {
                format!(
                    "deployed {} as version {}\n{}\n",
                    name_of(function, &id),
                    render_field(function, "version"),
                    render_field(function, "url")
                )
            })
        }
        FunctionCommand::Var {
            command: VarCommand::Set { id, name, value },
        } => {
            let reply = client.call(
                "PUT",
                &["v1", "functions", &id, "vars", &name],
                &[],
                Body::Json(json!({ "value": value })),
            )?;
            done(json, reply, |function| {
                format!("set variable {name} on {}\n", name_of(function, &id))
            })
        }
        FunctionCommand::Var {
            command: VarCommand::Unset { id, name },
        } => {
            let reply = client.call(
                "DELETE",
                &["v1", "functions", &id, "vars", &name],
                &[],
                Body::None,
            )?;
            done(json, reply, |function| {
                format!("removed variable {name} from {}\n", name_of(function, &id))
            })
        }
        FunctionCommand::Secret {
            command: SecretCommand::Set { id, name },
        } => {
            let value = credentials::read_secret(&name)?;
            let reply = client.call(
                "PUT",
                &["v1", "functions", &id, "secrets", &name],
                &[],
                Body::Json(json!({ "value": value })),
            )?;
            done(json, reply, |function| {
                format!("set secret {name} on {}\n", name_of(function, &id))
            })
        }
        FunctionCommand::Secret {
            command: SecretCommand::Unset { id, name },
        } => {
            let reply = client.call(
                "DELETE",
                &["v1", "functions", &id, "secrets", &name],
                &[],
                Body::None,
            )?;
            done(json, reply, |function| {
                format!("removed secret {name} from {}\n", name_of(function, &id))
            })
        }
    }
}

/// The function a manage call answered with: `{ "function": { ... } }`.
fn name_of(function: &Value, id: &str) -> String {
    let name = function.get("name").and_then(Value::as_str).unwrap_or("");
    if name.is_empty() {
        render::clean(id)
    } else {
        format!("{} ({})", render::clean(name), render::clean(id))
    }
}

fn render_field(function: &Value, name: &str) -> String {
    match function.get(name) {
        Some(Value::String(text)) => render::clean(text),
        Some(Value::Number(number)) => number.to_string(),
        _ => "-".to_string(),
    }
}

/// Print a read answer: the API's bytes with `--json`, else the rendering.
fn show(
    json: bool,
    reply: Reply,
    human: impl FnOnce(&Value) -> Result<String, CloudError>,
) -> Result<(), CloudError> {
    if json {
        return print_bytes(&reply.raw, true);
    }
    print_bytes(human(&reply.json)?.as_bytes(), false)
}

/// Print what a change answered (`{ "function": ... }`): the bytes with `--json`, else one line.
fn done(json: bool, reply: Reply, line: impl FnOnce(&Value) -> String) -> Result<(), CloudError> {
    if json {
        return print_bytes(&reply.raw, true);
    }
    let function = reply.json.get("function").cloned().unwrap_or(Value::Null);
    print_bytes(line(&function).as_bytes(), false)
}

/// Write to stdout. A closed pipe (`zega cloud projects | head`) ends the
/// output quietly.
fn print_bytes(bytes: &[u8], newline_if_missing: bool) -> Result<(), CloudError> {
    let mut out = io::stdout().lock();
    let result = out.write_all(bytes).and_then(|()| {
        if newline_if_missing && !bytes.ends_with(b"\n") {
            out.write_all(b"\n")?;
        }
        out.flush()
    });
    match result {
        Err(error) if error.kind() != io::ErrorKind::BrokenPipe => Err(CloudError::Local(format!(
            "cannot write to stdout: {error}"
        ))),
        _ => Ok(()),
    }
}

fn login(args: &CloudArgs) -> Result<(), CloudError> {
    let api = normalize_api(args.api.as_deref().unwrap_or(DEFAULT_API))?;
    let token = match &args.token_file {
        Some(path) => credentials::read_token_file(path)?,
        None => credentials::read_token()?,
    };
    let client = Client::new(&api, &token)?;
    // Verified before it is saved: a mistyped token never replaces a good one.
    let reply = client.get(&["v1", "whoami"], &[])?;
    let path = credentials::save(&api, &token)?;
    let who = render::whoami(&reply.json, None)?;
    print_bytes(
        format!("logged in to {api}\n{who}stored in {}\n", path.display()).as_bytes(),
        false,
    )
}

fn logout() -> Result<(), CloudError> {
    let message = match credentials::remove()? {
        Some(path) => format!(
            "removed {}\nThe token itself is still valid until you revoke it at {DASHBOARD}/tokens\n",
            path.display()
        ),
        None => "not logged in: no stored token\n".to_string(),
    };
    print_bytes(message.as_bytes(), false)
}

/// The token and host a command uses. `--token-file` brings its own token and
/// goes to `--api` or production; a stored token only ever goes to the host it
/// was made for.
fn session(args: &CloudArgs) -> Result<Session, CloudError> {
    let requested = args.api.as_deref().map(normalize_api).transpose()?;
    if let Some(path) = &args.token_file {
        let token = credentials::read_token_file(path)?;
        let api = requested.unwrap_or_else(|| DEFAULT_API.to_string());
        return Ok(Session {
            client: Client::new(&api, &token)?,
            api,
        });
    }
    let stored = credentials::load()?.ok_or("not logged in: run `zega cloud login` (the token is made at https://dashboard.zega.dev/tokens), or pass --token-file")?;
    let api = normalize_api(&stored.api)?;
    if let Some(requested) = requested.filter(|requested| *requested != api) {
        return Err(CloudError::Local(format!(
            "you are logged in to {api}, not {requested}: run `zega cloud login --api {requested}` first, or pass --token-file"
        )));
    }
    Ok(Session {
        client: Client::new(&api, &stored.token)?,
        api,
    })
}

/// The text a failed command prints on stderr: the API's sentence and code
/// first, then what to do about it.
fn describe(error: &CloudError, api: Option<&str>, token_file: Option<&std::path::Path>) -> String {
    match error {
        CloudError::Local(message) => format!("zega cloud: {message}\n"),
        CloudError::Network { host, detail } => {
            let mut text = format!("zega cloud: could not reach {host}: {detail}\n");
            if api.is_some() {
                text.push_str("Check the address given to --api.\n");
            }
            text
        }
        CloudError::Api(error) => {
            let mut text = format!(
                "zega cloud: {} (code: {}, HTTP {})\n",
                render::clean(&error.message),
                render::clean(&error.code),
                error.status
            );
            for line in guidance(error, token_file) {
                text.push_str(&line);
                text.push('\n');
            }
            text
        }
    }
}

fn guidance(error: &ApiError, token_file: Option<&std::path::Path>) -> Vec<String> {
    let mut lines = Vec::new();
    let new_token = match token_file {
        Some(path) => format!("put a new token in {}", path.display()),
        None => "run `zega cloud login`".to_string(),
    };
    match (error.status, error.code.as_str()) {
        (401, "wrong_credential") => lines.push(format!(
            "That is a graph or storage key, not an API token. Make an API token at {DASHBOARD}/tokens, then {new_token}."
        )),
        (401, _) => lines.push(format!("Make a token at {DASHBOARD}/tokens, then {new_token}.")),
        (_, "scope_required") => lines.push(format!(
            "This needs a manage token. Make one at {DASHBOARD}/tokens, then {new_token}."
        )),
        (_, "session_required") => {
            // The API's own sentence says where it is done; add it only when it does not.
            if !error.message.contains("dashboard") {
                lines.push(format!("This is done in the dashboard ({DASHBOARD}), signed in; an API token cannot do it."));
            }
        }
        (_, "project_restricted") => lines.push(
            "This token is restricted to one project and this call is account-wide. Use a token without a project restriction.".to_string(),
        ),
        _ => {}
    }
    if matches!(error.status, 429 | 503) {
        lines.push(match error.retry_after {
            Some(1) => "Wait 1 second, then run the command again.".to_string(),
            Some(seconds) => format!("Wait {seconds} seconds, then run the command again."),
            None => "Wait a little, then run the command again.".to_string(),
        });
    }
    match error.effect.as_deref() {
        Some("nothing") => lines.push("The change did not take effect.".to_string()),
        Some("partly") => lines.push("The change took effect in part: look at the function (`zega cloud functions <id>`) before you repeat it.".to_string()),
        Some("unknown") => lines.push("It is not known whether the change took effect: look at the function (`zega cloud functions <id>`); repeating it is safe.".to_string()),
        _ => {}
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(status: u16, code: &str, message: &str) -> ApiError {
        ApiError {
            status,
            code: code.into(),
            message: message.into(),
            retry_after: None,
            effect: None,
        }
    }

    #[test]
    fn a_session_only_answer_is_not_told_twice() {
        let said = error(
            403,
            "session_required",
            "Resizing is done in the dashboard (https://dashboard.zega.dev), signed in.",
        );
        assert!(guidance(&said, None).is_empty());
        let silent = error(403, "session_required", "Not available to a token.");
        assert!(guidance(&silent, None)[0].contains("dashboard"));
    }

    #[test]
    fn a_rejected_token_file_is_named() {
        let expired = error(401, "token_expired", "The token has expired.");
        let path = std::path::Path::new("ci/token");
        assert!(guidance(&expired, Some(path))[0].contains("ci/token"));
        assert!(guidance(&expired, None)[0].contains("zega cloud login"));
    }
}
