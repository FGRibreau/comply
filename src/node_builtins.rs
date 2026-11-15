//! Which import specifiers name a Node.js builtin module.
//!
//! A subpath under a builtin name is an npm package.
//! `punycode/punycode.js` is userland; `fs/promises` is a builtin.

/// Builtins resolved with or without the `node:` prefix.
/// Source: `require('node:module').builtinModules`.
const NODE_BUILTINS: &[&str] = &[
    "_http_agent",
    "_http_client",
    "_http_common",
    "_http_incoming",
    "_http_outgoing",
    "_http_server",
    "_stream_duplex",
    "_stream_passthrough",
    "_stream_readable",
    "_stream_transform",
    "_stream_wrap",
    "_stream_writable",
    "_tls_common",
    "_tls_wrap",
    "assert",
    "assert/strict",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "dns/promises",
    "domain",
    "events",
    "fs",
    "fs/promises",
    "http",
    "http2",
    "https",
    "inspector",
    "inspector/promises",
    "module",
    "net",
    "os",
    "path",
    "path/posix",
    "path/win32",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "readline/promises",
    "repl",
    "stream",
    "stream/consumers",
    "stream/promises",
    "stream/web",
    "string_decoder",
    "sys",
    "timers",
    "timers/promises",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "util/types",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

/// Builtins resolved only with the `node:` prefix.
/// A bare `test` is the npm package `test`.
const PREFIX_ONLY_BUILTINS: &[&str] = &["sea", "sqlite", "test", "test/reporters"];

/// True when Node resolves `specifier` to a builtin module.
///
/// `fs`, `node:fs` and `fs/promises` are builtins.
/// `punycode/punycode.js` and bare `test` are not.
pub(crate) fn is_node_builtin(specifier: &str) -> bool {
    match specifier.strip_prefix("node:") {
        Some(name) => NODE_BUILTINS.contains(&name) || PREFIX_ONLY_BUILTINS.contains(&name),
        None => NODE_BUILTINS.contains(&specifier),
    }
}
