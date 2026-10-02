//! `zega cloud`: the command line for Zega Cloud's public management API
//! (`https://cloud.zega.dev/openapi.json`).
//!
//! The API takes an account API token, made in the dashboard, and nothing
//! else. What costs money (creating a graph or project, resizing, spending
//! caps, restoring a backup) and the tokens themselves are dashboard-only: the
//! API answers 403 `session_required` and this CLI relays that, it does not
//! pretend otherwise.

mod api;
mod confirm;
mod credentials;
mod render;

use api::{normalize_api, ApiError, Body, Client, CloudError, Reply, DASHBOARD, DEFAULT_API};
use clap::{Args, Subcommand, ValueEnum};
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
    /// Rename or delete a project.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// Rename or delete a graph; its keys, custom domains and monitoring.
    Graph {
        #[command(subcommand)]
        command: GraphCommand,
    },
    /// Create, rename or delete a bucket; its keys.
    Bucket {
        #[command(subcommand)]
        command: BucketCommand,
    },
    /// Create, rename or delete a function; read its logs and code, deploy
    /// code, set its variables and secrets.
    Function {
        #[command(subcommand)]
        command: FunctionCommand,
    },
}

/// What a delete asks first. Without a terminal there is nobody to ask.
#[derive(Args)]
struct Confirm {
    /// Delete without asking. Without it, a terminal shows what will be
    /// deleted and asks for its id; without a terminal the command refuses.
    #[arg(long)]
    yes: bool,
}

/// A setting that is on or off.
#[derive(Clone, Copy, ValueEnum)]
enum Switch {
    On,
    Off,
}

impl Switch {
    fn as_str(self) -> &'static str {
        match self {
            Switch::On => "on",
            Switch::Off => "off",
        }
    }
    fn is_on(self) -> bool {
        matches!(self, Switch::On)
    }
}

#[derive(Subcommand)]
enum ProjectCommand {
    /// Rename a project: the label only. Needs a manage token.
    Rename {
        /// A project id.
        id: String,
        /// 1 to 64 characters.
        name: String,
    },
    /// Delete a project and everything in it: its graphs, functions and
    /// buckets. Needs a manage token.
    Delete {
        /// A project id.
        id: String,
        #[command(flatten)]
        confirm: Confirm,
    },
}

#[derive(Subcommand)]
enum GraphCommand {
    /// Rename a graph: the label only. Needs a manage token.
    Rename {
        /// A graph id.
        id: String,
        /// 1 to 64 characters.
        name: String,
    },
    /// Delete a graph. A project's last graph cannot be deleted alone: delete
    /// the project. Needs a manage token.
    Delete {
        /// A graph id.
        id: String,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// The graph's API keys (`zk_...`): list, create, revoke.
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
    /// The graph's custom domains: list, add, remove.
    Domain {
        #[command(subcommand)]
        command: DomainCommand,
    },
    /// The graph's monitoring settings: show, set.
    Monitoring {
        #[command(subcommand)]
        command: MonitoringCommand,
    },
}

#[derive(Subcommand)]
enum BucketCommand {
    /// Create a bucket in a project (the project needs a paid graph). Needs a
    /// manage token.
    Create {
        /// The project's id.
        project: String,
        /// The bucket's label, 1 to 64 characters.
        name: String,
    },
    /// Rename a bucket: the label only. Needs a manage token.
    Rename {
        /// A bucket id.
        id: String,
        /// 1 to 64 characters.
        name: String,
    },
    /// Delete a bucket. Refused while it holds objects. Needs a manage token.
    Delete {
        /// A bucket id.
        id: String,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// The bucket's storage keys (`zs_...`): list, create, revoke.
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
}

/// A graph's or a bucket's keys.
#[derive(Subcommand)]
enum KeyCommand {
    /// List the keys: how to recognise each, never a secret. Needs a read token.
    List {
        /// The graph's or bucket's id.
        id: String,
    },
    /// Make a key. Its secret is printed once and cannot be read again.
    /// Needs a manage token.
    Create {
        /// The graph's or bucket's id.
        id: String,
        /// A label, up to 64 characters.
        #[arg(long)]
        name: Option<String>,
    },
    /// Revoke a key: the next call with it is a 401. Needs a manage token.
    Revoke {
        /// The graph's or bucket's id.
        id: String,
        /// The key's id (from `key list`).
        key: String,
    },
}

#[derive(Subcommand)]
enum DomainCommand {
    /// List the domains and the DNS record each needs. Needs a read token.
    List {
        /// A graph id.
        id: String,
    },
    /// Add a domain you own, and print the CNAME to set. Needs a manage token.
    Add {
        /// A graph id.
        id: String,
        /// For example graph.example.com.
        hostname: String,
    },
    /// Remove a domain. Needs a manage token.
    Remove {
        /// A graph id.
        id: String,
        /// The domain.
        hostname: String,
    },
}

#[derive(Subcommand)]
enum MonitoringCommand {
    /// Show the settings. Needs a read token.
    Show {
        /// A graph id.
        id: String,
    },
    /// Change a setting. Needs a manage token.
    Set {
        /// A graph id.
        id: String,
        /// Keep each call's exact query text for 24 hours, for the owner only
        /// (off by default; off deletes what was kept).
        #[arg(long, value_enum, value_name = "on|off")]
        keep_query_text: Switch,
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
    /// Create a function in a project. Its address answers 404 until code is
    /// deployed. Needs a manage token.
    Create {
        /// The project's id.
        project: String,
        /// The function's label, 1 to 64 characters.
        name: String,
    },
    /// Rename a function: the label only. Needs a manage token.
    Rename {
        /// A function id.
        id: String,
        /// 1 to 64 characters.
        name: String,
    },
    /// Delete a function and its code. Needs a manage token.
    Delete {
        /// A function id.
        id: String,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// Print the deployed code exactly as it is, or write it to a file. Needs
    /// a read token.
    Code {
        /// A function id.
        id: String,
        /// Write the code to this file (replacing it) instead of stdout.
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Turn the function's logs on or off. They are off until turned on, and
    /// every logged event is billed under the function's own cap. Needs a
    /// manage token.
    Logging {
        /// A function id.
        id: String,
        /// on or off.
        #[arg(value_enum)]
        setting: Switch,
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
        CloudCommand::Project { command } => project_command(client, json, command),
        CloudCommand::Graph { command } => graph_command(client, json, command),
        CloudCommand::Bucket { command } => bucket_command(client, json, command),
        CloudCommand::Function { command } => function(client, json, command),
    }
}

fn project_command(client: &Client, json: bool, command: ProjectCommand) -> Result<(), CloudError> {
    match command {
        ProjectCommand::Rename { id, name } => rename(client, json, Kind::Project, &id, &name),
        ProjectCommand::Delete { id, confirm } => {
            delete(client, json, confirm.yes, Kind::Project, &id)
        }
    }
}

fn graph_command(client: &Client, json: bool, command: GraphCommand) -> Result<(), CloudError> {
    match command {
        GraphCommand::Rename { id, name } => rename(client, json, Kind::Graph, &id, &name),
        GraphCommand::Delete { id, confirm } => delete(client, json, confirm.yes, Kind::Graph, &id),
        GraphCommand::Key { command } => keys(client, json, Kind::Graph, command),
        GraphCommand::Domain {
            command: DomainCommand::List { id },
        } => show(
            json,
            client.get(&["v1", "graphs", &id, "domains"], &[])?,
            render::domains,
        ),
        GraphCommand::Domain {
            command: DomainCommand::Add { id, hostname },
        } => {
            let reply = client.call(
                "POST",
                &["v1", "graphs", &id, "domains"],
                &[],
                Body::Json(json!({ "hostname": hostname })),
            )?;
            done(json, reply, "domain", |domain| {
                render::domain_added(domain, &id)
            })
        }
        GraphCommand::Domain {
            command: DomainCommand::Remove { id, hostname },
        } => {
            let reply = client.call(
                "DELETE",
                &["v1", "graphs", &id, "domains", &hostname],
                &[],
                Body::None,
            )?;
            done(json, reply, "removed", |removed| {
                format!(
                    "removed domain {} from graph {}\n",
                    render::clean(removed.as_str().unwrap_or(&hostname)),
                    render::clean(&id)
                )
            })
        }
        GraphCommand::Monitoring {
            command: MonitoringCommand::Show { id },
        } => show(
            json,
            client.get(&["v1", "graphs", &id, "monitoring", "settings"], &[])?,
            render::monitoring,
        ),
        GraphCommand::Monitoring {
            command:
                MonitoringCommand::Set {
                    id,
                    keep_query_text,
                },
        } => {
            let reply = client.call(
                "PUT",
                &["v1", "graphs", &id, "monitoring", "settings"],
                &[],
                Body::Json(json!({ "keepQueryText": keep_query_text.is_on() })),
            )?;
            if json {
                return print_bytes(&reply.raw, true);
            }
            print_bytes(
                format!(
                    "monitoring settings of graph {}\n{}",
                    render::clean(&id),
                    render::monitoring(&reply.json)?
                )
                .as_bytes(),
                false,
            )
        }
    }
}

fn bucket_command(client: &Client, json: bool, command: BucketCommand) -> Result<(), CloudError> {
    match command {
        BucketCommand::Create { project, name } => {
            let reply = client.call(
                "POST",
                &["v1", "projects", &project, "buckets"],
                &[],
                Body::Json(json!({ "name": name })),
            )?;
            done(json, reply, "bucket", |bucket| {
                format!(
                    "created bucket {}\nAddress {}\nMake a key with `zega cloud bucket key create {}`\n",
                    name_of(bucket, ""),
                    render_field(bucket, "url"),
                    render_field(bucket, "id")
                )
            })
        }
        BucketCommand::Rename { id, name } => rename(client, json, Kind::Bucket, &id, &name),
        BucketCommand::Delete { id, confirm } => {
            delete(client, json, confirm.yes, Kind::Bucket, &id)
        }
        BucketCommand::Key { command } => keys(client, json, Kind::Bucket, command),
    }
}

/// The four kinds of resource that are renamed and deleted the same way.
#[derive(Clone, Copy)]
enum Kind {
    Project,
    Graph,
    Bucket,
    Function,
}

impl Kind {
    /// `project`: the word in messages and the key of a single resource in an answer.
    fn noun(self) -> &'static str {
        match self {
            Kind::Project => "project",
            Kind::Graph => "graph",
            Kind::Bucket => "bucket",
            Kind::Function => "function",
        }
    }
    /// `projects`: the path segment after `/v1/`.
    fn collection(self) -> &'static str {
        match self {
            Kind::Project => "projects",
            Kind::Graph => "graphs",
            Kind::Bucket => "buckets",
            Kind::Function => "functions",
        }
    }
    /// What deleting it does, from the API's own description of the route.
    fn consequence(self) -> &'static str {
        match self {
            Kind::Project => "This deletes its graphs, functions and buckets too. An unpaid graph is gone at once; a paid one is cancelled and its data deleted 30 days later, after which nothing brings it back.",
            Kind::Graph => "An unpaid graph is gone at once. A paid one is cancelled and its machine stopped; its data is deleted 30 days later, after which nothing brings it back.",
            Kind::Bucket => "Its keys are revoked with it. It is refused while the bucket holds objects.",
            Kind::Function => "Its code is deleted and its address answers 404 from then on.",
        }
    }
}

fn rename(client: &Client, json: bool, kind: Kind, id: &str, name: &str) -> Result<(), CloudError> {
    let reply = client.call(
        "PUT",
        &["v1", kind.collection(), id, "name"],
        &[],
        Body::Json(json!({ "name": name })),
    )?;
    done(json, reply, kind.noun(), |item| {
        format!(
            "renamed {} {} to {}\n",
            kind.noun(),
            render::clean(id),
            render::clean(item.get("name").and_then(Value::as_str).unwrap_or(name))
        )
    })
}

/// Delete a project, graph, bucket or function after it was confirmed: by
/// `--yes`, or by typing its id at a terminal. Nothing reaches the API before
/// that, not even the lookup that says what is about to go.
fn delete(client: &Client, json: bool, yes: bool, kind: Kind, id: &str) -> Result<(), CloudError> {
    if !yes {
        confirm::require_terminal(kind.noun())?;
        let shown = client.get(&["v1", kind.collection(), id], &[])?;
        let item = shown.json.get(kind.noun()).cloned().unwrap_or(Value::Null);
        let shown_id = item.get("id").and_then(Value::as_str).unwrap_or(id);
        // On stderr: the question is not the command's output, so `--json` stays clean.
        eprint!(
            "{}",
            render::doomed(kind.noun(), &item, shown_id, kind.consequence())
        );
        confirm::ask(shown_id)?;
    }
    let reply = client.call("DELETE", &["v1", kind.collection(), id], &[], Body::None)?;
    done(json, reply, kind.noun(), |item| {
        let mut line = format!("deleted {} {}\n", kind.noun(), name_of(item, id));
        if let Some(when) = item
            .get("deleteAfter")
            .and_then(Value::as_str)
            .filter(|_| matches!(kind, Kind::Graph))
        {
            line.push_str(&format!("its data is deleted on {}\n", render::clean(when)));
        }
        line
    })
}

/// A graph's or a bucket's keys.
fn keys(client: &Client, json: bool, kind: Kind, command: KeyCommand) -> Result<(), CloudError> {
    let collection = kind.collection();
    match command {
        KeyCommand::List { id } => show(
            json,
            client.get(&["v1", collection, &id, "keys"], &[])?,
            render::keys,
        ),
        KeyCommand::Create { id, name } => {
            let body = match name {
                Some(name) => json!({ "name": name }),
                None => json!({}),
            };
            let reply = client.call(
                "POST",
                &["v1", collection, &id, "keys"],
                &[],
                Body::Json(body),
            )?;
            done(json, reply, "key", |key| {
                render::key_created(key, &format!("{} {}", kind.noun(), render::clean(&id)))
            })
        }
        KeyCommand::Revoke { id, key } => {
            let reply = client.call(
                "DELETE",
                &["v1", collection, &id, "keys", &key],
                &[],
                Body::None,
            )?;
            done(json, reply, "revoked", |revoked| {
                format!(
                    "revoked key {} of {} {}\n",
                    render::clean(revoked.as_str().unwrap_or(&key)),
                    kind.noun(),
                    render::clean(&id)
                )
            })
        }
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
        FunctionCommand::Create { project, name } => {
            let reply = client.call(
                "POST",
                &["v1", "projects", &project, "functions"],
                &[],
                Body::Json(json!({ "name": name })),
            )?;
            done(json, reply, "function", |function| {
                format!(
                    "created function {}\nAddress {} (404 until you deploy: `zega cloud function deploy {} <file>`)\n",
                    name_of(function, ""),
                    render_field(function, "url"),
                    render_field(function, "id")
                )
            })
        }
        FunctionCommand::Rename { id, name } => rename(client, json, Kind::Function, &id, &name),
        FunctionCommand::Delete { id, confirm } => {
            delete(client, json, confirm.yes, Kind::Function, &id)
        }
        FunctionCommand::Logging { id, setting } => {
            let reply = client.call(
                "PUT",
                &["v1", "functions", &id, "logs"],
                &[],
                Body::Json(json!({ "logs": setting.as_str() })),
            )?;
            done(json, reply, "function", |function| {
                format!(
                    "logs are {} for {}\n",
                    render_field(function, "logs"),
                    name_of(function, &id)
                )
            })
        }
        FunctionCommand::Code { id, out } => {
            if json && out.is_some() {
                return Err("--json prints the API's answer, which holds the code: it cannot be combined with --out".into());
            }
            let reply = client.get(&["v1", "functions", &id, "code"], &[])?;
            if json {
                return print_bytes(&reply.raw, true);
            }
            let code = reply
                .json
                .get("code")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("function {} has no deployed code yet: `zega cloud function deploy {} <file>`", render::clean(&id), render::clean(&id)))?;
            match out {
                // Written only now, so a failed request never empties an existing file.
                Some(path) => {
                    std::fs::write(&path, code)
                        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
                    print_bytes(
                        format!("wrote {} bytes to {}\n", code.len(), path.display()).as_bytes(),
                        false,
                    )
                }
                None => print_bytes(code.as_bytes(), false),
            }
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
            done(json, reply, "function", |function| {
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
            done(json, reply, "function", |function| {
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
            done(json, reply, "function", |function| {
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
            done(json, reply, "function", |function| {
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
            done(json, reply, "function", |function| {
                format!("removed secret {name} from {}\n", name_of(function, &id))
            })
        }
    }
}

/// A resource a manage call answered with (`{ "function": { ... } }`) as
/// `name (id)`; the id the API gave, else `fallback`.
fn name_of(item: &Value, fallback: &str) -> String {
    let name = item.get("name").and_then(Value::as_str).unwrap_or("");
    let id = item.get("id").and_then(Value::as_str).unwrap_or(fallback);
    match (name.is_empty(), id.is_empty()) {
        (false, false) => format!("{} ({})", render::clean(name), render::clean(id)),
        (false, true) => render::clean(name),
        _ => render::clean(id),
    }
}

fn render_field(item: &Value, name: &str) -> String {
    match item.get(name) {
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

/// Print what a change answered (`{ "function": ... }`, `{ "revoked": "..." }`):
/// the bytes with `--json`, else the text made from the answer's `key`.
fn done(
    json: bool,
    reply: Reply,
    key: &str,
    line: impl FnOnce(&Value) -> String,
) -> Result<(), CloudError> {
    if json {
        return print_bytes(&reply.raw, true);
    }
    let item = reply.json.get(key).cloned().unwrap_or(Value::Null);
    print_bytes(line(&item).as_bytes(), false)
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
        Some("partly") => lines.push("The change took effect in part: look at what you changed (`zega cloud projects|graphs|buckets|functions <id>`) before you repeat it.".to_string()),
        Some("unknown") => lines.push("It is not known whether the change took effect: look at what you changed (`zega cloud projects|graphs|buckets|functions <id>`); repeating it is safe.".to_string()),
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
