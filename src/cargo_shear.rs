//! cargo-shear subprocess — flag unused dependencies in `Cargo.toml`.
//!
//! Why this lives in Comply: dead deps are a security and compile-time
//! liability. Each unused crate widens the supply chain, slows builds, and
//! adds version-bump churn. `cargo shear` (https://crates.io/crates/cargo-shear)
//! detects them by walking the source tree and comparing imports against
//! the `[dependencies]` table.
//!
//! How it works:
//! 1. `is_available()` probes `cargo shear --version`. Cached in a
//!    `OnceLock`.
//! 2. `lint_files()` finds the workspace root for any `.rs` file in the
//!    input set (the nearest `Cargo.toml` ancestor) and runs:
//!
//!        cargo shear --format=json <workspace>
//!
//! 3. The JSON output is parsed; each "finding" of `code = shear/unused_dependency`
//!    becomes one comply diagnostic on the offending `Cargo.toml`. The
//!    byte `offset` is converted to a 1-based line/column by reading the
//!    manifest and counting newlines.
//! 4. `drop_proven_used()` then drops the findings rustc contradicts.
//!    cargo-shear reads sources and never expands a macro.
//!    It misses code a derive emits, like `::strum::ParseError`.
//!    It misses code `include!` loads from `OUT_DIR`, like `include_proto!`.
//!    rustc judges after expansion, through clippy's run.
//!
//! Workspaces are de-duplicated so we don't shell out twice for the same
//! manifest when many `.rs` files share a parent crate.

use rustc_hash::FxHashSet;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use crate::clippy::{ExternUsage, PackageExterns};
use crate::diagnostic::{Diagnostic, Severity};
use crate::files::SourceFile;
use crate::runner_helpers;

/// Stable comply rule id surfaced in diagnostics for unused-dependency
/// findings. Mirrors the convention used by clippy/oxlint runners.
pub const RULE_ID: &str = "rust-unused-dep";

/// Cached availability probe for `cargo shear`.
pub fn is_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| runner_helpers::probe_binary("cargo", &["shear", "--version"]))
}

/// Run `cargo shear` on every workspace touched by `files` and return the
/// remapped diagnostics. Files outside any workspace are skipped silently —
/// shear can only operate on a real Cargo project root.
#[must_use = "diagnostics from cargo-shear must be reported"]
pub fn lint_files(files: &[&SourceFile]) -> Result<Vec<Diagnostic>> {
    if files.is_empty() {
        return Ok(vec![]);
    }
    let mut diagnostics = Vec::new();
    let mut roots = FxHashSet::default();
    for f in files {
        if let Some(root) = runner_helpers::find_cargo_workspace_root(&f.path) {
            roots.insert(root);
        }
    }
    for workspace in roots {
        diagnostics.extend(lint_workspace(&workspace)?);
    }
    Ok(diagnostics)
}

/// Run `cargo shear --format=json` from inside `workspace` and parse the
/// result. We `current_dir(workspace)` instead of passing it as a positional
/// argument because shear honors `CARGO_MANIFEST_DIR` and the cwd more
/// reliably than its `PATH` argument when invoked from a child process.
fn lint_workspace(workspace: &Path) -> Result<Vec<Diagnostic>> {
    let output = Command::new("cargo")
        .args(["shear", "--format=json"])
        .current_dir(workspace)
        .output()
        .with_context(|| format!("failed to invoke `cargo shear` in {}", workspace.display()))?;
    // Empty stdout = nothing to parse. Either shear truly found nothing
    // or it errored before producing output; either way, return cleanly
    // rather than crashing on empty JSON.
    if output.stdout.is_empty() {
        return Ok(vec![]);
    }
    let Ok(report) = serde_json::from_slice::<ShearReport>(&output.stdout) else {
        return Ok(vec![]);
    };
    convert_findings(report.findings, workspace)
}

/// Check whether the dep named in a shear message is declared as
/// `optional = true` in the manifest. Optional deps are feature-gated
/// by design — flagging them as unused is noise because cargo-shear
/// only sees the default feature set.
fn is_optional_dep(manifest_path: &Path, message: &str) -> bool {
    let Some(dep_name) = dependency_key(message) else {
        return false;
    };
    let Ok(content) = fs::read_to_string(manifest_path) else {
        return false;
    };
    let Ok(toml) = content.parse::<toml::Value>() else {
        return false;
    };
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(deps) = toml.get(section).and_then(|d| d.as_table())
            && let Some(entry) = deps.get(dep_name)
                && entry
                    .get("optional")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    return true;
                }
    }
    false
}

/// The dependency key a shear message names between backticks.
fn dependency_key(message: &str) -> Option<&str> {
    message.split('`').nth(1).filter(|name| !name.is_empty())
}

/// Drop each finding whose dependency rustc proves used.
/// Every other diagnostic passes through untouched.
pub fn drop_proven_used(diagnostics: Vec<Diagnostic>, usage: &ExternUsage) -> Vec<Diagnostic> {
    diagnostics
        .into_iter()
        .filter(|diag| {
            let proven = diag.rule_id == RULE_ID
                && dependency_key(&diag.message)
                    .is_some_and(|key| rustc_proves_used(&diag.path, key, usage));
            !proven
        })
        .collect()
}

/// Dependency tables other than plain `[dependencies]`.
/// Cargo still accepts the underscore spellings.
const OTHER_DEPENDENCY_TABLES: &[&str] = &[
    "dev-dependencies",
    "dev_dependencies",
    "build-dependencies",
    "build_dependencies",
];

/// Whether rustc proves `key` used by `manifest`'s package.
/// Proof: a compiled target where rustc kept the extern.
/// Other dependency tables keep cargo-shear's verdict.
fn rustc_proves_used(manifest: &Path, key: &str, usage: &ExternUsage) -> bool {
    let Some(toml) = fs::read_to_string(manifest)
        .ok()
        .and_then(|content| content.parse::<toml::Value>().ok())
    else {
        return false;
    };
    let Some(package) = usage.package(manifest) else {
        return false;
    };
    is_plain_dependency(&toml, key) && proves_used(package, &declared_externs(&toml), key)
}

/// rustc's half of the proof, once the manifest is read.
/// An undeclared extern name means a renamed library.
/// rustc's silence then proves nothing.
fn proves_used(package: &PackageExterns, declared: &FxHashSet<String>, key: &str) -> bool {
    let all_named = package
        .unused
        .values()
        .flatten()
        .all(|name| declared.contains(name) || package.own_libraries.contains(name));
    if package.unreadable || !all_named {
        return false;
    }
    let name = key.replace('-', "_");
    package.compiled.iter().any(|target| {
        !package
            .unused
            .get(target)
            .is_some_and(|unused| unused.contains(&name))
    })
}

/// `key` sits in `[dependencies]` and in no other dependency table.
/// Only those reach every non-build target.
fn is_plain_dependency(toml: &toml::Value, key: &str) -> bool {
    let in_table = |table: Option<&toml::Value>| {
        table
            .and_then(toml::Value::as_table)
            .is_some_and(|deps| deps.contains_key(key))
    };
    let elsewhere = OTHER_DEPENDENCY_TABLES
        .iter()
        .any(|name| in_table(toml.get(*name)))
        || platform_tables(toml).any(|platform| {
            platform
                .values()
                .any(|table| table.as_table().is_some_and(|deps| deps.contains_key(key)))
        });
    in_table(toml.get("dependencies")) && !elsewhere
}

/// The `[target.'cfg(…)']` tables of a manifest.
fn platform_tables(
    toml: &toml::Value,
) -> impl Iterator<Item = &toml::map::Map<String, toml::Value>> {
    toml.get("target")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|targets| targets.values().filter_map(toml::Value::as_table))
}

/// The extern name of every key in every dependency table.
fn declared_externs(toml: &toml::Value) -> FxHashSet<String> {
    let top = ["dependencies"]
        .iter()
        .chain(OTHER_DEPENDENCY_TABLES)
        .filter_map(|name| toml.get(*name));
    let platform = platform_tables(toml).flat_map(|platform| platform.values());
    top.chain(platform)
        .filter_map(toml::Value::as_table)
        .flat_map(|deps| deps.keys())
        .map(|key| key.replace('-', "_"))
        .collect()
}

/// Convert each shear finding into a comply Diagnostic. We only emit
/// diagnostics for `shear/unused_dependency` findings — shear also surfaces
/// `shear/unlinked_files` (orphan source files) but those are out of scope
/// for a "no unused deps" rule and would muddy the signal. Each unused-dep
/// finding includes a `file` and a byte `offset`; we translate the offset
/// to a 1-based line/column for editor compatibility.
fn convert_findings(findings: Vec<Finding>, workspace: &Path) -> Result<Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    for finding in findings {
        if finding.code != "shear/unused_dependency" {
            continue;
        }
        let Some(file) = finding.file else { continue };
        let manifest_path = workspace.join(&file);
        if is_optional_dep(&manifest_path, &finding.message) {
            continue;
        }
        let offset = finding.location.map(|l| l.offset).unwrap_or(0);
        let (line, column) = byte_offset_to_line_col(&manifest_path, offset).unwrap_or((1, 1));
        diagnostics.push(Diagnostic {
            path: manifest_path.into(),
            line,
            column,
            rule_id: RULE_ID.into(),
            message: format!(
                "{} — every unused dep widens the supply chain, slows builds, \
                 and adds version-bump churn. Remove it from `[dependencies]` \
                 (or run `cargo shear --fix`).",
                finding.message
            ),
            severity: Severity::Error,
            span: None,
        });
    }
    Ok(diagnostics)
}

/// Read `path` and convert a byte `offset_bytes` into a 1-based
/// (line, column). Falls back to (1, 1) if the file can't be read —
/// better to surface the finding at the top of the manifest than to
/// drop it entirely.
fn byte_offset_to_line_col(path: &Path, offset_bytes: usize) -> Option<(usize, usize)> {
    let content = fs::read_to_string(path).ok()?;
    let bytes = content.as_bytes();
    let clamped = offset_bytes.min(bytes.len());
    let mut line = 1usize;
    let mut col = 1usize;
    for &b in &bytes[..clamped] {
        if b == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    Some((line, col))
}

/// External wire format mirror — see comply:rust-serde-deny-unknown-fields.
#[derive(Debug, Deserialize)]
struct ShearReport {
    #[serde(default)]
    findings: Vec<Finding>,
}

/// External wire format mirror — see comply:rust-serde-deny-unknown-fields.
#[derive(Debug, Deserialize)]
struct Finding {
    code: String,
    message: String,
    /// Optional — `shear/unlinked_files` findings have no file field.
    /// We skip those upstream so the absence is fine.
    #[serde(default)]
    file: Option<String>,
    /// Optional for the same reason.
    #[serde(default)]
    location: Option<Location>,
}

/// External wire format mirror — see comply:rust-serde-deny-unknown-fields.
#[derive(Debug, Deserialize)]
struct Location {
    offset: usize,
    #[allow(dead_code)]
    length: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_finding_json() {
        let raw = br#"{"summary":{"errors":1,"warnings":0,"fixed":0},"findings":[{"code":"shear/unused_dependency","severity":"error","message":"unused dependency `serde`","file":"Cargo.toml","location":{"offset":75,"length":5},"help":"remove this dependency","fixable":true}]}"#;
        let report: ShearReport = serde_json::from_slice(raw).unwrap();
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].message, "unused dependency `serde`");
    }

    #[test]
    fn empty_findings_array() {
        let raw = br#"{"summary":{"errors":0,"warnings":0,"fixed":0},"findings":[]}"#;
        let report: ShearReport = serde_json::from_slice(raw).unwrap();
        assert!(report.findings.is_empty());
    }

    fn manifest(text: &str) -> toml::Value {
        text.parse().unwrap()
    }

    /// One compiled `lib` target, with the externs rustc found unused.
    fn lib_leaving_unused(names: &[&str]) -> PackageExterns {
        let lib = (vec!["lib".to_owned()], "app".to_owned());
        let mut package = PackageExterns::default();
        package.compiled.insert(lib.clone());
        package.own_libraries.insert("app".to_owned());
        package
            .unused
            .insert(lib, names.iter().map(|n| (*n).to_owned()).collect());
        package
    }

    #[test]
    fn only_a_plain_dependency_can_be_proven_used() {
        let toml = manifest(
            "[dependencies]\nplain = \"1\"\nboth = \"1\"\n\
             [dev-dependencies]\nboth = \"1\"\ndev = \"1\"\n\
             [build-dependencies]\nbuild = \"1\"\n\
             [target.'cfg(unix)'.dependencies]\nunix = \"1\"\n",
        );
        assert!(is_plain_dependency(&toml, "plain"));
        // A target without the extern stays silent: no proof.
        for key in ["both", "dev", "build", "unix", "absent"] {
            assert!(!is_plain_dependency(&toml, key), "{key}");
        }
    }

    #[test]
    fn a_target_where_rustc_kept_the_extern_proves_it_used() {
        let declared = declared_externs(&manifest(
            "[dependencies]\ntonic-prost = \"1\"\nidle = \"1\"\n",
        ));
        let package = lib_leaving_unused(&["idle"]);
        assert!(proves_used(&package, &declared, "tonic-prost"));
        assert!(!proves_used(&package, &declared, "idle"));
    }

    #[test]
    fn rustc_naming_an_undeclared_extern_proves_nothing() {
        // `md-5` builds a library named `md5`, not `md_5`.
        let declared = declared_externs(&manifest("[dependencies]\nmd-5 = \"1\"\n"));
        let package = lib_leaving_unused(&["md5"]);
        assert!(!proves_used(&package, &declared, "md-5"));
    }

    #[test]
    fn an_unreadable_warning_or_no_compiled_target_proves_nothing() {
        let declared = declared_externs(&manifest("[dependencies]\nprost = \"1\"\n"));
        let mut unreadable = lib_leaving_unused(&[]);
        unreadable.unreadable = true;
        assert!(!proves_used(&unreadable, &declared, "prost"));
        assert!(!proves_used(&PackageExterns::default(), &declared, "prost"));
    }

    #[test]
    fn byte_offset_at_start() {
        let tmp = std::env::temp_dir().join("comply-shear-test.toml");
        fs::write(&tmp, "line1\nline2\nline3").unwrap();
        assert_eq!(byte_offset_to_line_col(&tmp, 0), Some((1, 1)));
        assert_eq!(byte_offset_to_line_col(&tmp, 6), Some((2, 1)));
        assert_eq!(byte_offset_to_line_col(&tmp, 12), Some((3, 1)));
        let _ = fs::remove_file(&tmp);
    }
}
