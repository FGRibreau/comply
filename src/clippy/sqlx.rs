//! Where `sqlx::query!` reads query types under clippy.
//!
//! A `.sqlx` cache may lack test queries.
//! comply then retries online, against the database sqlx reads.
//! Else the refusal names both remedies.

use anyhow::Result;
use std::ffi::OsStr;
use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::toolchain::ToolchainError;

/// The error sqlx raises for an uncached query.
/// Versions 0.7 to 0.9 share its wording.
const UNCACHED_QUERY: &str = "`SQLX_OFFLINE=true` but there is no cached data for this query";

/// The variable that turns the query macros offline.
const OFFLINE_VAR: &str = "SQLX_OFFLINE";

/// How long the database gets to accept a TCP connection.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// The most bytes of a `.env` file comply reads.
const DOTENV_CAP: u64 = 64 * 1024;

/// Where one clippy run reads query types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    /// Whatever the caller's environment says.
    Inherited,
    /// The `.sqlx` cache only.
    Cache,
    /// The database of `DATABASE_URL`, even when a `.env` says offline.
    Database,
}

impl Source {
    /// The cache when the workspace ships one.
    /// A `SQLX_OFFLINE` the caller set wins.
    pub(super) fn first(workspace: &Path) -> Self {
        if forces_offline(workspace, std::env::var_os(OFFLINE_VAR).as_deref()) {
            Self::Cache
        } else {
            Self::Inherited
        }
    }

    pub(super) fn apply(self, cmd: &mut Command) {
        match self {
            Self::Inherited => {}
            Self::Cache => {
                cmd.env(OFFLINE_VAR, "true");
            }
            Self::Database => {
                cmd.env(OFFLINE_VAR, "false");
            }
        }
    }
}

/// Split out of [`Source::first`] to test without touching the environment.
fn forces_offline(workspace: &Path, caller_choice: Option<&OsStr>) -> bool {
    let already_chosen = caller_choice.is_some_and(|v| !v.is_empty());
    !already_chosen && workspace.join(".sqlx").is_dir()
}

/// Crates the cache misses, and the database to use instead.
#[derive(Debug)]
pub(super) struct CacheMiss {
    crates: Vec<String>,
    database: String,
}

impl CacheMiss {
    /// The online run's refusal, prefixed with why it ran.
    pub(super) fn explain(&self, refusal: anyhow::Error) -> anyhow::Error {
        let Some(ToolchainError::Unusable { what }) = refusal.downcast_ref::<ToolchainError>()
        else {
            return refusal;
        };
        let crates = self.crates.join(", ");
        ToolchainError::Unusable {
            what: format!(
                "The `.sqlx` cache lacks queries of {crates}. comply ran clippy against \
                 {} instead, and that failed too. {what}",
                self.database
            ),
        }
        .into()
    }
}

/// What a cache-first run tells comply to do next.
/// `Ok(None)`: judge this run. `Ok(Some(_))`: run online.
/// `Err`: the cache missed and no database can fill in.
pub(super) fn cache_miss(
    workspace: &Path,
    source: Source,
    stdout: &[u8],
) -> Result<Option<CacheMiss>> {
    if source != Source::Cache {
        return Ok(None);
    }
    let missed = uncached_crates(stdout);
    if missed.is_empty() {
        return Ok(None);
    }
    let crates: Vec<String> = missed.iter().map(|(name, _)| format!("`{name}`")).collect();
    let mut databases: Vec<String> = Vec::new();
    for (_, dir) in &missed {
        let database =
            database_for(workspace, dir).map_err(|why| refusal(workspace, &crates, &why, stdout))?;
        if !databases.contains(&database) {
            databases.push(database);
        }
    }
    Ok(Some(CacheMiss {
        crates,
        database: databases.join(" and "),
    }))
}

/// Crates stopped by an uncached query, with their directory.
fn uncached_crates(stdout: &[u8]) -> Vec<(String, PathBuf)> {
    let mut crates: Vec<(String, PathBuf)> = Vec::new();
    for envelope in super::envelopes(stdout) {
        let uncached = envelope
            .message
            .as_ref()
            .is_some_and(|m| m.level == "error" && m.message.starts_with(UNCACHED_QUERY));
        let (Some(manifest), Some(target)) = (envelope.manifest_path, envelope.target) else {
            continue;
        };
        let dir = Path::new(&manifest).parent().map(Path::to_path_buf).unwrap_or_default();
        let entry = (target.name, dir);
        if uncached && !crates.contains(&entry) {
            crates.push(entry);
        }
    }
    crates
}

/// Why no database can replace the cache.
#[derive(Debug, thiserror::Error)]
enum NoDatabase {
    /// Neither the environment nor a `.env` names one.
    #[error("no `DATABASE_URL` is set, in the environment or a `.env` above {}", .dir.display())]
    Unset { dir: PathBuf },
    /// It accepted no connection in time.
    #[error(
        "the database at {host}:{port} (`DATABASE_URL`, {origin}) accepted no connection \
         within {} s",
        PROBE_TIMEOUT.as_secs()
    )]
    Unreachable {
        host: String,
        port: u16,
        origin: Origin,
    },
    /// Its URL names no endpoint comply knows.
    #[error("`DATABASE_URL` ({origin}) names no database comply can reach")]
    UnreadableUrl { origin: Origin },
}

/// Where a `DATABASE_URL` was read.
#[derive(Debug, PartialEq, Eq)]
enum Origin {
    Environment,
    File(PathBuf),
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Environment => f.write_str("set in the environment"),
            Self::File(path) => write!(f, "set in {}", path.display()),
        }
    }
}

/// The database sqlx reads for the crate in `dir`.
/// It must accept a connection first.
fn database_for(workspace: &Path, dir: &Path) -> Result<String, NoDatabase> {
    let from_env = std::env::var("DATABASE_URL").ok();
    let (url, origin) = database_url(workspace, dir, from_env).ok_or_else(|| NoDatabase::Unset {
        dir: dir.to_path_buf(),
    })?;
    match endpoint(&url) {
        Some(Endpoint::Local) => Ok(format!("the local database of `DATABASE_URL` ({origin})")),
        Some(Endpoint::Tcp(host, port)) if accepts_connections(&host, port) => {
            Ok(format!("the database at {host}:{port} (`DATABASE_URL`, {origin})"))
        }
        Some(Endpoint::Tcp(host, port)) => Err(NoDatabase::Unreachable { host, port, origin }),
        None => Err(NoDatabase::UnreadableUrl { origin }),
    }
}

/// The `DATABASE_URL` sqlx reads for `dir`, and its origin.
///
/// `from_env` wins over any `.env`.
/// Next comes the crate's own `.env`, alone.
/// Else the nearest above the workspace root.
fn database_url(
    workspace: &Path,
    dir: &Path,
    from_env: Option<String>,
) -> Option<(String, Origin)> {
    if let Some(url) = from_env.filter(|url| !url.is_empty()) {
        return Some((url, Origin::Environment));
    }
    let own = dir.join(".env");
    let file = if own.is_file() {
        own
    } else {
        workspace.ancestors().map(|d| d.join(".env")).find(|f| f.is_file())?
    };
    let url = dotenv_value(&file, "DATABASE_URL")?;
    Some((url, Origin::File(file)))
}

/// The value of `key` in a `.env`, read like dotenvy.
/// `export`, quotes and a trailing comment are dropped.
/// The last entry wins, as in sqlx 0.8.
fn dotenv_value(file: &Path, key: &str) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(file)
        .ok()?
        .take(DOTENV_CAP)
        .read_to_string(&mut text)
        .ok()?;
    text.lines()
        .rev()
        .find_map(|line| {
            let line = line.trim();
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (name, value) = line.split_once('=')?;
            (name.trim() == key).then(|| unquote(value.trim()))
        })
        .filter(|value| !value.is_empty())
}

fn unquote(value: &str) -> String {
    for quote in ['"', '\''] {
        if let Some(rest) = value.strip_prefix(quote) {
            return rest.split(quote).next().unwrap_or_default().to_owned();
        }
    }
    value.split(" #").next().unwrap_or_default().trim().to_owned()
}

/// Where the database of a `DATABASE_URL` listens.
#[derive(Debug, PartialEq, Eq)]
enum Endpoint {
    /// A TCP host and port, probed first.
    /// A connection there can hang.
    Tcp(String, u16),
    /// A file or Unix socket, failing fast when absent.
    Local,
}

fn endpoint(url: &str) -> Option<Endpoint> {
    let (scheme, rest) = url.split_once(':')?;
    let default_port = match scheme {
        "postgres" | "postgresql" => 5432,
        "mysql" | "mariadb" => 3306,
        "sqlite" => return Some(Endpoint::Local),
        _ => return None,
    };
    let rest = rest.strip_prefix("//")?;
    let (authority, tail) = rest.split_at(rest.find(['/', '?']).unwrap_or(rest.len()));
    let socket_dir = tail
        .split_once('?')
        .into_iter()
        .flat_map(|(_, query)| query.split('&'))
        .any(|pair| pair.starts_with("host=/") || pair.starts_with("host=%2F"));
    let hostport = authority.rsplit_once('@').map_or(authority, |(_, hostport)| hostport);
    let (host, port) = match hostport.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && !port.contains(']') => {
            (host, port.parse().ok()?)
        }
        _ => (hostport, default_port),
    };
    if socket_dir || host.is_empty() {
        return Some(Endpoint::Local);
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    Some(Endpoint::Tcp(host.to_owned(), port))
}

/// Whether `host:port` accepts a TCP connection within [`PROBE_TIMEOUT`].
/// Name lookup runs on a thread: `getaddrinfo` takes no timeout.
fn accepts_connections(host: &str, port: u16) -> bool {
    let started = Instant::now();
    let target = (host.to_owned(), port);
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let reached = target.to_socket_addrs().is_ok_and(|mut addresses| {
            addresses.any(|address| {
                let left = PROBE_TIMEOUT.saturating_sub(started.elapsed());
                !left.is_zero() && TcpStream::connect_timeout(&address, left).is_ok()
            })
        });
        // The receiver is gone once the probe timed out.
        sender.send(reached).unwrap_or_default();
    });
    receiver.recv_timeout(PROBE_TIMEOUT).unwrap_or(false)
}

/// The refusal when no database can replace the cache.
fn refusal(workspace: &Path, crates: &[String], why: &NoDatabase, stdout: &[u8]) -> anyhow::Error {
    let quoted = super::build_errors(stdout)
        .into_iter()
        .take(super::QUOTED_BUILD_ERRORS)
        .collect::<Vec<_>>()
        .join("\n");
    ToolchainError::Unusable {
        what: format!(
            "cargo clippy could not compile {ws}; no Rust file was linted. \
             The `.sqlx` cache lacks queries of {crates}, which clippy compiles \
             with `--all-targets --all-features`, and {why}. Either cache them: \
             `cargo sqlx prepare --workspace -- --all-targets --all-features` in {ws}. \
             Or start the database of `DATABASE_URL`: comply then lints them against it.\n\
             {quoted}",
            ws = workspace.display(),
            crates = crates.join(", "),
        ),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// A workspace whose `.sqlx` cache is committed starts offline.
    /// The caller's own choice wins.
    #[test]
    fn offline_is_forced_only_for_a_cached_workspace_the_caller_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let cached = dir.path().join("cached");
        std::fs::create_dir_all(cached.join(".sqlx")).unwrap();
        let plain = dir.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();

        assert!(forces_offline(&cached, None));
        assert!(forces_offline(&cached, Some(OsStr::new(""))));
        assert!(!forces_offline(&cached, Some(OsStr::new("false"))));
        assert!(!forces_offline(&plain, None));
    }

    /// A crate's lib and test builds report the same miss.
    /// The crate is named once; other errors name nothing.
    #[test]
    fn uncached_crates_names_each_crate_once() {
        let json = br#"{"reason":"compiler-message","manifest_path":"/ws/it/Cargo.toml","target":{"name":"it","kind":["lib"]},"message":{"message":"`SQLX_OFFLINE=true` but there is no cached data for this query, run `cargo sqlx prepare`","level":"error","spans":[]}}
{"reason":"compiler-message","manifest_path":"/ws/it/Cargo.toml","target":{"name":"it","kind":["lib"]},"message":{"message":"`SQLX_OFFLINE=true` but there is no cached data for this query, run `cargo sqlx prepare`","level":"error","spans":[]}}
{"reason":"compiler-message","manifest_path":"/ws/core/Cargo.toml","target":{"name":"core","kind":["lib"]},"message":{"message":"mismatched types","level":"error","spans":[]}}
{"reason":"build-finished","success":false}"#;
        assert_eq!(
            uncached_crates(json),
            vec![("it".to_owned(), PathBuf::from("/ws/it"))]
        );
    }

    /// Only a cache-first run can miss the cache.
    #[test]
    fn cache_miss_is_read_only_after_a_cache_first_run() {
        let json = br#"{"reason":"compiler-message","manifest_path":"/ws/it/Cargo.toml","target":{"name":"it","kind":["lib"]},"message":{"message":"`SQLX_OFFLINE=true` but there is no cached data for this query","level":"error","spans":[]}}"#;
        for source in [Source::Inherited, Source::Database] {
            assert!(cache_miss(Path::new("/ws"), source, json).unwrap().is_none());
        }
        assert!(cache_miss(Path::new("/ws"), Source::Cache, b"").unwrap().is_none());
    }

    #[test]
    fn dotenv_value_reads_plain_exported_and_quoted_entries() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".env");
        let cases = [
            ("DATABASE_URL=postgres://a@h/db\n", Some("postgres://a@h/db")),
            (
                "export DATABASE_URL=\"postgres://a:b@h:5433/db\" # used by sqlx\n",
                Some("postgres://a:b@h:5433/db"),
            ),
            ("DATABASE_URL='postgres://h/db'\n", Some("postgres://h/db")),
            ("DATABASE_URL=postgres://h/db # local\n", Some("postgres://h/db")),
            ("DATABASE_URL=postgres://h/a\nDATABASE_URL=postgres://h/b\n", Some("postgres://h/b")),
            ("# DATABASE_URL=postgres://h/db\nAPI_DATABASE_URL=x\n", None),
            ("DATABASE_URL=\n", None),
        ];
        for (text, expected) in cases {
            std::fs::write(&file, text).unwrap();
            assert_eq!(dotenv_value(&file, "DATABASE_URL").as_deref(), expected, "{text}");
        }
    }

    /// The environment wins over every `.env`.
    /// A crate's own `.env` wins, even without the key.
    #[test]
    fn database_url_prefers_the_environment_then_the_crate_env_file_then_the_nearest_above() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let member = ws.join("member");
        let other = ws.join("other");
        std::fs::create_dir_all(&member).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(dir.path().join(".env"), "DATABASE_URL=postgres://above/db\n").unwrap();
        std::fs::write(member.join(".env"), "OTHER=1\n").unwrap();

        assert_eq!(database_url(&ws, &member, None), None);
        let (url, origin) = database_url(&ws, &other, None).unwrap();
        assert_eq!(url, "postgres://above/db");
        assert_eq!(origin, Origin::File(dir.path().join(".env")));
        let set = Some("postgres://env/db".to_owned());
        let (url, origin) = database_url(&ws, &member, set).unwrap();
        assert_eq!((url.as_str(), origin), ("postgres://env/db", Origin::Environment));
        let (url, _) = database_url(&ws, &other, Some(String::new())).unwrap();
        assert_eq!(url, "postgres://above/db");
    }

    #[test]
    fn endpoint_reads_host_and_port_of_tcp_urls_only() {
        let tcp = |host: &str, port| Some(Endpoint::Tcp(host.to_owned(), port));
        let cases = [
            ("postgres://u:p@db.example:6543/app", tcp("db.example", 6543)),
            ("postgresql://u@127.0.0.1/app?sslmode=disable", tcp("127.0.0.1", 5432)),
            ("postgres://u:p%40ss@[::1]:5433/app", tcp("::1", 5433)),
            ("postgres://[::1]/app", tcp("::1", 5432)),
            ("mysql://root@localhost/app", tcp("localhost", 3306)),
            ("postgres:///app?host=/tmp", Some(Endpoint::Local)),
            ("postgres://u@localhost/app?host=%2Ftmp", Some(Endpoint::Local)),
            ("sqlite://data.db", Some(Endpoint::Local)),
            ("redis://localhost", None),
            ("postgres://h:notaport/app", None),
            ("not a url", None),
        ];
        for (url, expected) in cases {
            assert_eq!(endpoint(url), expected, "{url}");
        }
    }

    /// A real listener answers; its port, once closed, does not.
    #[test]
    fn accepts_connections_tells_an_open_port_from_a_closed_one() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(accepts_connections("127.0.0.1", port));
        drop(listener);
        let started = Instant::now();
        assert!(!accepts_connections("127.0.0.1", port));
        assert!(started.elapsed() <= PROBE_TIMEOUT + Duration::from_secs(1));
    }

    /// TEST-NET-1 never answers: the probe gives up on time.
    #[test]
    fn accepts_connections_gives_up_within_its_timeout() {
        let started = Instant::now();
        assert!(!accepts_connections("192.0.2.1", 5432));
        assert!(started.elapsed() <= PROBE_TIMEOUT + Duration::from_secs(1));
    }

    /// The online refusal keeps rustc's words.
    /// It also says why comply went online.
    #[test]
    fn explain_prefixes_the_online_refusal_with_the_cache_miss() {
        let miss = CacheMiss {
            crates: vec!["`it`".to_owned()],
            database: "the database at h:5432".to_owned(),
        };
        let inner: anyhow::Error = ToolchainError::Unusable {
            what: "cargo clippy could not compile /ws\nrelation \"item\" does not exist".to_owned(),
        }
        .into();
        let text = miss.explain(inner).to_string();
        assert!(text.starts_with("The `.sqlx` cache lacks queries of `it`."), "{text}");
        assert!(text.contains("the database at h:5432"), "{text}");
        assert!(text.contains("relation \"item\" does not exist"), "{text}");
    }
}
