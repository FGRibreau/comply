//! no-undeclared-env-var — runtime env reads missing from `.env.example`.
//!
//! The nearest `.env.example` above a file scopes it. No such file, no check.

mod oxc_typescript;
mod rust;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::project::walk_up_finding;
use crate::rules::RuleDef;
use crate::rules::backend::{Backend, CheckCtx};
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "no-undeclared-env-var",
    description: "A runtime environment read must be declared in the nearest `.env.example`.",
    remediation: "Add `NAME=` (or `# NAME=`) to the nearest `.env.example`, or \
                  allow-list a tooling name in `comply.toml`:\n\n\
                  [rules.no-undeclared-env-var]\n\
                  allowed = [\"NODE_ENV\"]",
    severity: Severity::Error,
    doc_url: None,
    categories: &["deployment"],

    skip_in_test_dir: true,
    skip_in_relaxed_dir: false,
};

/// Registers the Rust and TS/JS backends.
pub fn register() -> RuleDef {
    RuleDef {
        meta: META,
        backends: vec![
            (
                Language::TypeScript,
                Backend::Oxc(Box::new(oxc_typescript::Check)),
            ),
            (
                Language::JavaScript,
                Backend::Oxc(Box::new(oxc_typescript::Check)),
            ),
            (Language::Tsx, Backend::Oxc(Box::new(oxc_typescript::Check))),
            (Language::Rust, Backend::TreeSitter(Box::new(rust::Check))),
        ],
    }
}

/// True when no allowlist entry or nearest `.env.example` declares `name`.
fn is_undeclared(ctx: &CheckCtx, name: &str) -> bool {
    let allowed = ctx
        .config
        .required_string_list(META.id, "allowed", ctx.lang);
    if allowed.iter().any(|entry| entry == name) {
        return false;
    }
    let Some(dir) = ctx
        .path
        .parent()
        .and_then(|parent| walk_up_finding(parent, ".env.example"))
    else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(dir.join(".env.example")) else {
        return false;
    };
    !text.lines().any(|line| declared_key(line) == Some(name))
}

/// The key a `.env.example` line assigns, commented out or not.
fn declared_key(line: &str) -> Option<&str> {
    let line = line.trim_start_matches('\u{feff}').trim_start();
    let line = line.strip_prefix('#').unwrap_or(line).trim_start();
    let line = line.strip_prefix("export ").unwrap_or(line);
    let (key, _) = line.split_once('=')?;
    let key = key.trim();
    let is_identifier =
        !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    is_identifier.then_some(key)
}

fn message(name: &str) -> String {
    format!("`{name}` is read at runtime but missing from the nearest `.env.example`.")
}
