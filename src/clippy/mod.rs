//! @generated — cargo clippy subprocess — runs clippy on Rust files and converts the
//! `--message-format=json` stream into unified Diagnostic structs.
//!
//! How it works:
//! 1. `is_available()` checks `cargo clippy --version` works. Cached in a
//!    `OnceLock` so we don't fork cargo on every invocation.
//! 2. `lint_files()` collects every `Backend::Clippy` binding from the
//!    rule registry, groups the input files by their containing Cargo
//!    workspace root, and for each workspace runs:
//!
//!        cargo clippy <--workspace | -p <owning package>...> \
//!            --all-targets --all-features --keep-going \
//!            --message-format=json --quiet \
//!            --manifest-path <workspace>/Cargo.toml \
//!            -- -W clippy::lint1 -W clippy::lint2 ...
//!
//!    rustc's `unused_crate_dependencies` lint is forced on as well.
//!    Its verdicts become an [`ExternUsage`] for `rust-unused-dep`.
//!    From cargo 1.100, cargo swallows them for its `unused_dependencies` lint.
//!    That lint's stderr reports then join the [`ExternUsage`].
//!
//!    `--all-features` compiles every feature-gated item, so a test target
//!    that calls one still builds and gets linted.
//!
//!    The [`sqlx`] module picks where `sqlx::query!` reads query types.
//!
//!    Cargo emits one JSON object per line. We parse the stream,
//!    keep only `compiler-message` rows, filter to spans inside the
//!    requested files, and remap each lint code to its comply RuleMeta.
//!    A run stopped by a denied lint gets two more (see [`Pass`]).
//! 3. A file without a Cargo.toml joins a crate whose `mod` reaches it.
//!    The passes' dep-info then shows whether rustc read each file.
//!    A file none read is named as not analyzed.
//!
//! Performance note: clippy compiles the workspace, so the first run on
//! a large project takes seconds-to-minutes. Subsequent runs are
//! incremental (cargo's normal cache). This is unavoidable — clippy
//! has no per-file mode.

mod all_args;
mod all_lints;
pub(crate) mod config_writer;
mod remap;
mod schema;
mod sqlx;

use anyhow::{Context, Result};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::config::Config;
use crate::diagnostic::{Diagnostic, Severity};
use crate::files::SourceFile;
use crate::project::import_index::{ImportIndex, is_conventional_rust_crate_root};
use crate::rules::meta::RuleMeta;
use schema::{CargoMessage, RustcDiagnostic, RustcSpan};

/// Cached availability probe for `cargo clippy`. Rust toolchains usually
/// ship clippy via rustup, but in container builds it can be missing.
pub fn is_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        Command::new("cargo")
            .args(["clippy", "--version"])
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

/// Run clippy on every workspace touched by `files` and return the
/// remapped diagnostics.
/// The [`ExternUsage`] of every crate clippy compiled comes along.
///
/// `config` is the resolved per-project comply.toml. We use it to:
///   - add `-W clippy::xxx` for any lint flagged `enabled = true`
///     in `[rules."clippy::xxx"]` (most clippy lints default to
///     `allow`, so this is how the user opts in to extra ones)
///   - materialize a `clippy.toml` from per-lint thresholds and `[clippy]`
///     (or the project's own clippy.toml), and point cargo at it via `CLIPPY_CONF_DIR`, so something like
///     `[rules."clippy::too_many_lines"] threshold = 50` actually
///     reaches clippy
#[must_use = "diagnostics from clippy must be reported"]
///
/// `is_partial` (a git file scope) limits each run to the packages owning
/// `files`; a plain path scan lints the whole workspace.
///
/// `index` names the crate whose `mod` reaches a file without a Cargo.toml.
///
/// A crate outside the workspace members is named.
/// So is a file no compiled target reads.
/// A git file scope refuses it; a path scan notes it.
pub fn lint_files(
    files: &[&SourceFile],
    config: &Config,
    index: &ImportIndex,
    is_partial: bool,
) -> Result<(Vec<Diagnostic>, ExternUsage)> {
    let mut usage = ExternUsage::default();
    if files.is_empty() {
        return Ok((vec![], usage));
    }
    usage.cargo_lints = cargo_lints_unused_dependencies();
    let bindings = crate::rules::collect_clippy_bindings();
    let remap = remap::build_table(&bindings);
    let lint_args = lint_args(&bindings, config);

    let mut groups = group_by_workspace(files);
    // A file without a Cargo.toml joins a crate whose `mod` reaches it.
    let mut loose = Vec::new();
    for file in groups.remove(&None).unwrap_or_default() {
        match declaring_workspace(index, &file.path) {
            Some(root) => groups.entry(Some(root)).or_default().push(file),
            None => loose.push(file),
        }
    }
    let mut runs = Vec::new();
    // Every root's clippy.toml is built before any clippy run starts.
    for (root, files_in_ws) in groups.into_iter().filter_map(|(ws, f)| Some((ws?, f))) {
        let clippy_conf = config_writer::materialize(config, &root, &files_in_ws)?;
        runs.push((root, files_in_ws, clippy_conf));
    }

    let mut diagnostics = Vec::new();
    let mut compiled = FxHashSet::default();
    let mut roots = FxHashSet::default();
    let mut judged = loose;
    for (root, files_in_ws, clippy_conf) in runs {
        let file_filter: FxHashSet<PathBuf> = files_in_ws
            .iter()
            .map(|f| canonicalize_or_self(&f.path))
            .collect();
        let members = workspace_members(&root)?;
        let (files_in_ws, outside) = split_by_membership(&root, &files_in_ws, &members);
        if !outside.is_empty() {
            let what = outside_message(&root, &outside);
            if is_partial {
                return Err(crate::toolchain::ToolchainError::Unusable { what }.into());
            }
            eprintln!("comply: {what}.");
        }
        let packages = if is_partial {
            crate::runner_helpers::cargo_package_args(&root, &files_in_ws)
        } else {
            vec!["--workspace".to_owned()]
        };
        let mut source = sqlx::Source::first(&root);
        let mut missed = None;
        let mut run = |pass| -> Result<std::process::Output> {
            let invoke = |source| {
                invoke_clippy(&root, &packages, &lint_args, &clippy_conf.dir, source, pass)
            };
            let mut output = invoke(source)?;
            // A rejected `clippy.toml` fails the same way online.
            ensure_config_accepted(&clippy_conf, &output.stdout)?;
            if let Some(miss) = sqlx::cache_miss(&root, source, &output.stdout)? {
                source = sqlx::Source::Database;
                output = invoke(source)?;
                missed = Some(miss);
            }
            ensure_compiled(&root, &output).map_err(|refusal| match &missed {
                None => refusal,
                Some(miss) => miss.explain(refusal),
            })?;
            Ok(output)
        };
        let mut output = run(Pass::Lint)?;
        compiled.extend(compiled_sources(&root, &members, &output.stdout)?);
        roots.extend(target_roots(&output.stdout));
        if denied_lints(&output.stdout).next().is_some() {
            run(Pass::Check)?;
            let capped = run(Pass::Capped)?;
            ensure_packages_linted(&root, &files_in_ws, &output.stdout, &capped.stdout)?;
            compiled.extend(compiled_sources(&root, &members, &capped.stdout)?);
            roots.extend(target_roots(&capped.stdout));
            output = capped;
        }
        ensure_analyzed(&root, &output.stdout)?;
        usage.record(&output.stdout, &output.stderr, &root);
        judged.extend(files_in_ws);
        diagnostics.extend(parse_clippy_jsonl(
            &output.stdout,
            &root,
            &file_filter,
            &remap,
        ));
    }

    // Any pass's dep-info vouches for a file, even across workspaces.
    // It also lists what `include_str!` reads: only a module counts.
    let indexed_roots: Vec<&Path> = index
        .indexed_paths()
        .filter(|path| roots.contains(*path))
        .collect();
    let uncompiled: Vec<&SourceFile> = judged
        .into_iter()
        .filter(|file| {
            let path = canonicalize_or_self(&file.path);
            !compiled.contains(&path) || !is_module(index, &path, &roots, &indexed_roots)
        })
        .collect();
    if !uncompiled.is_empty() {
        let unanalyzed = if is_partial {
            crate::toolchain::Unanalyzed::Refuse
        } else {
            crate::toolchain::Unanalyzed::Note
        };
        crate::toolchain::report_unanalyzed(
            &uncompiled_why(uncompiled.len()),
            &uncompiled,
            unanalyzed,
        )?;
    }

    Ok((diagnostics, usage))
}

/// Forces rustc to name each dependency a crate target leaves unused.
/// Forced, so a crate's own `allow` or `deny` cannot change it.
/// The lint carries no fix, so `--fix` never edits code for it.
const UNUSED_EXTERNS_LINT: &str = "--force-warn=unused_crate_dependencies";

/// An extern no code can use, so every target reports it unused.
/// A target reporting it proves rustc's verdicts reach comply.
/// cargo 1.100 swallows every such report, the probe's included.
const LIVENESS_PROBE: &str = "comply_liveness_probe";

/// The workspace of a crate whose `mod` items reach `file`.
/// `#[path]` lets a crate read a file outside every package.
fn declaring_workspace(index: &ImportIndex, file: &Path) -> Option<PathBuf> {
    let mut seen = FxHashSet::default();
    let mut pending = vec![file.to_path_buf()];
    while let Some(module) = pending.pop() {
        for declarer in index.rust_mod_declarers(&module) {
            if let Some(root) = crate::runner_helpers::find_cargo_workspace_root(declarer) {
                return Some(root);
            }
            if seen.insert(declarer.to_path_buf()) {
                pending.push(declarer.to_path_buf());
            }
        }
    }
    None
}

/// The root file of each target a pass compiled.
/// A fresh target counts too.
fn target_roots(stdout: &[u8]) -> impl Iterator<Item = PathBuf> + '_ {
    envelopes(stdout)
        .filter(|envelope| envelope.reason == "compiler-artifact")
        .filter_map(|envelope| envelope.target?.src_path)
        .map(|root| canonicalize_or_self(Path::new(&root)))
}

/// True unless the index proves `path` is no module.
/// Only an indexed root of its package rules it out.
/// A file outside every package needs any indexed root.
fn is_module(
    index: &ImportIndex,
    path: &Path,
    roots: &FxHashSet<PathBuf>,
    indexed_roots: &[&Path],
) -> bool {
    let is_root = roots.contains(path) || is_conventional_rust_crate_root(path);
    if is_root || index.is_declared_rust_module(path) {
        return true;
    }
    let package = path
        .parent()
        .and_then(|dir| crate::project::walk_up_finding(dir, "Cargo.toml"));
    let indexes_its_root = indexed_roots.iter().any(|root| {
        package
            .as_ref()
            .is_none_or(|package| root.starts_with(package))
    });
    if !indexes_its_root {
        return true;
    }
    // A macro or an `include!` may declare it unseen.
    let name = crate::rules::rust_orphan_module::module_name(path);
    let dir = crate::rules::rust_orphan_module::declaring_dir(path);
    index.has_unresolved_rust_mod(name) || index.has_opaque_rust_mods_in(dir)
}

/// Why clippy analyzed none of `count` Rust files, with the remedies.
fn uncompiled_why(count: usize) -> String {
    let (host, other) = if cfg!(target_os = "macos") {
        ("macos", "linux")
    } else {
        (std::env::consts::OS, "macos")
    };
    format!(
        "cargo clippy compiled {count} Rust file(s) as no crate's module. \
         A file joins a crate only through a `mod` item; \
         `include_str!` reads it as text, not code. \
         A cfg off on this {host} host leaves it out, as \
         `#[cfg(target_os = \"{other}\")] pub mod {other};` does. \
         List a file no build of this host compiles in .complyignore"
    )
}

/// The `-W`/`-A` flags passed after `--`, shared by lint and `--fix`:
/// clippy records them in each crate's fingerprint, so any difference
/// between the two modes re-lints the workspace when one follows the other.
pub(crate) fn lint_args(
    bindings: &[(&'static str, &'static RuleMeta, Severity)],
    config: &Config,
) -> Vec<String> {
    let mut args = build_lint_args(bindings);
    args.push(UNUSED_EXTERNS_LINT.to_owned());
    // rustc only reads the file of an extern some code uses.
    args.push("--extern".to_owned());
    args.push(format!(
        "{LIVENESS_PROBE}=/nonexistent/lib{LIVENESS_PROBE}.rlib"
    ));
    extend_with_user_enabled(&mut args, config);
    extend_with_user_disabled(&mut args, config);
    args
}

/// Append `-W clippy::xxx` for every clippy lint the user explicitly
/// enabled in their `comply.toml`. We only flip on lints that exist
/// in the `all_lints` registry, so a typo in the rule id becomes a
/// silent no-op rather than a clippy crash.
fn extend_with_user_enabled(args: &mut Vec<String>, config: &Config) {
    for (rule_id, rule) in config.iter_rules() {
        if rule.enabled != Some(true) {
            continue;
        }
        if !rule_id.starts_with("clippy::") {
            continue;
        }
        if !is_known_clippy_lint(rule_id) {
            continue;
        }
        args.push(format!("-W{rule_id}"));
    }
}

/// Append `-A clippy::xxx` for every clippy lint the user disabled in
/// their `comply.toml`. This stops the lint from firing in the first
/// place rather than relying on the post-filter — keeps clippy's
/// output cleaner and saves cycles. The post-filter still runs as a
/// safety net for diagnostics from non-clippy backends.
fn extend_with_user_disabled(args: &mut Vec<String>, config: &Config) {
    for (rule_id, rule) in config.iter_rules() {
        if rule.disabled != Some(true) {
            continue;
        }
        if !rule_id.starts_with("clippy::") {
            continue;
        }
        if !is_known_clippy_lint(rule_id) {
            continue;
        }
        args.push(format!("-A{rule_id}"));
    }
}

/// True if `rule_id` matches a clippy lint we discovered via
/// `cargo clippy -- -W help` and recorded in `all_lints.rs`. Used to
/// drop typos before they become `error: unknown lint` crashes.
fn is_known_clippy_lint(rule_id: &str) -> bool {
    all_lints::ALL_CLIPPY_LINTS
        .iter()
        .any(|(name, _)| *name == rule_id)
}

/// Build the `-W clippy::lint` flag list passed to clippy after `--`.
/// Every lint is added as `-W` (warn), never `-D` (deny): the comply driver
/// owns the final exit code, so clippy must not fail the run itself. The
/// registry only collects lints we *want* enabled, so there's no `-A`.
fn build_lint_args(bindings: &[(&'static str, &'static RuleMeta, Severity)]) -> Vec<String> {
    bindings
        .iter()
        .map(|(lint, _, _)| format!("-W{lint}"))
        .collect()
}

/// Group files by their workspace root. Files outside any Cargo workspace
/// land under the `None` key so the caller can warn the user.
pub(crate) fn group_by_workspace<'a>(
    files: &[&'a SourceFile],
) -> FxHashMap<Option<PathBuf>, Vec<&'a SourceFile>> {
    let mut out: FxHashMap<Option<PathBuf>, Vec<&'a SourceFile>> = FxHashMap::default();
    for f in files {
        let root = crate::runner_helpers::find_cargo_workspace_root(&f.path);
        out.entry(root).or_default().push(*f);
    }
    out
}

/// Build the `sqlx::query!` macros of `workspace` from its `.sqlx/` cache.
/// They reach the database at build time; the cache answers them without
/// one, as in a CI build. A `SQLX_OFFLINE` set by the caller wins.
pub(crate) fn use_sqlx_query_cache(cmd: &mut Command, workspace: &Path) {
    sqlx::Source::first(workspace).apply(cmd);
}

/// One cargo run over a workspace.
///
/// A lint the code denies fails its crate.
/// Cargo then builds nothing that depends on it.
/// The last two passes judge what it hid.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// Clippy, with the lint levels the code sets.
    Lint,
    /// rustc alone, with the levels the code sets.
    Check,
    /// Clippy, with every denied lint lowered to a warning.
    /// It lowers rustc's own lints too, so [`Pass::Check`] judges those.
    Capped,
}

/// Spawn `cargo clippy` for one workspace and return the captured Output.
/// We pass `--quiet` to suppress cargo's progress noise on stderr, and
/// `--message-format=json` to get structured diagnostics on stdout.
///
/// `workspace` is the workspace root (resolved via `find_cargo_workspace_root`)
/// and `packages` the selection (`--workspace` or `-p` flags), so a root that
/// is itself a package still reaches the other members.
///
/// `clippy_conf_dir` holds comply's generated `clippy.toml` (`CLIPPY_CONF_DIR`),
/// which carries the keys of any project `clippy.toml` it accepted.
/// `source` says where `sqlx::query!` reads query types.
fn invoke_clippy(
    workspace: &Path,
    packages: &[String],
    lint_args: &[String],
    clippy_conf_dir: &Path,
    source: sqlx::Source,
    pass: Pass,
) -> Result<std::process::Output> {
    let manifest = workspace.join("Cargo.toml");
    let mut cmd = Command::new("cargo");
    cmd.arg(if pass == Pass::Check {
        "check"
    } else {
        "clippy"
    });
    cmd.args(packages);
    cmd.args([
        "--all-targets",
        "--all-features",
        // One failed crate must not hide an unrelated one.
        "--keep-going",
        "--message-format=json",
        "--quiet",
        // Both readers of cargo's lint reports expect plain text.
        "--color=never",
        // `build.warnings` would hide the warnings, or fail on them.
        "--config",
        "build.warnings=\"warn\"",
        "--manifest-path",
    ]);
    cmd.arg(&manifest);
    source.apply(&mut cmd);
    if pass != Pass::Check {
        cmd.arg("--").args(lint_args);
    }
    if pass == Pass::Capped {
        cmd.args(["--cap-lints", "warn"]);
    }
    let (key, val) = config_writer::env_var_for_dir(clippy_conf_dir);
    cmd.env(key, val);

    let output = cmd
        .output()
        .with_context(|| format!("failed to invoke `cargo clippy` for {}", manifest.display()))?;

    // The exit code is not read here: clippy exits non-zero both when lints
    // fired and when the build failed, which tells the two apart only in the
    // JSON stream. `ensure_config_accepted` and `ensure_compiled` read it.
    Ok(output)
}

/// Refuse a run whose generated `clippy.toml` clippy rejected (unknown key,
/// wrong type, duplicate): clippy then lints nothing. Each error names the
/// line of the file the key came from, never the generated file in the cache.
fn ensure_config_accepted(conf: &config_writer::ClippyConf, stdout: &[u8]) -> Result<()> {
    let generated = canonicalize_or_self(&conf.dir.join("clippy.toml"));
    let mut rejections: Vec<String> = Vec::new();
    for diag in compiler_messages(stdout).filter(|diag| diag.level == "error") {
        let Some(span) = diag.spans.iter().find(|span| {
            span.is_primary && canonicalize_or_self(Path::new(&span.file_name)) == generated
        }) else {
            continue;
        };
        let line = span.text.first().map_or("", |line| line.text.trim());
        let hint: String = diag
            .children
            .iter()
            .flat_map(|child| &child.spans)
            .filter_map(|span| span.suggested_replacement.as_deref())
            .map(|key| format!(" (did you mean `{key}`?)"))
            .collect();
        let rejection = format!(
            "{}: {}: `{line}`{hint}",
            conf.origin_of(span.line_start, span.column_start),
            diag.message
        );
        // Clippy repeats the error for every crate of the workspace.
        if !rejections.contains(&rejection) {
            rejections.push(rejection);
        }
    }
    if rejections.is_empty() {
        return Ok(());
    }
    Err(crate::toolchain::ToolchainError::Unusable {
        what: format!(
            "clippy rejected its configuration and linted no Rust file:\n{}",
            rejections.join("\n")
        ),
    }
    .into())
}

/// How many build errors the refusal quotes.
const QUOTED_BUILD_ERRORS: usize = 5;

/// Refuse a clippy run that linted nothing while looking successful.
///
/// rustc's own errors carry no `clippy::` code, so `parse_clippy_jsonl`
/// drops them. A workspace that fails to compile therefore yielded zero
/// diagnostics and comply printed "all clear" on code clippy never saw.
/// Only a denied lint explains a failure without a build error.
/// Any other is refused with cargo's stderr.
/// Examples: a bad manifest, a build script that panics, a killed rustc.
fn ensure_compiled(workspace: &Path, output: &std::process::Output) -> Result<()> {
    let errors = build_errors(&output.stdout);
    let is_explained = output.status.success()
        || denied_lints(&output.stdout).next().is_some()
        || is_unused_dependency_denied(output.status.code(), &output.stderr);
    if errors.is_empty() && is_explained {
        return Ok(());
    }
    let detail = if errors.is_empty() {
        String::from_utf8_lossy(&output.stderr).trim().to_owned()
    } else {
        errors
            .into_iter()
            .take(QUOTED_BUILD_ERRORS)
            .collect::<Vec<_>>()
            .join("\n")
    };
    Err(unusable(workspace, "could not compile", &detail))
}

/// Refuse a run that compiled no crate of `workspace`.
/// No Rust rule ran on it then.
fn ensure_analyzed(workspace: &Path, stdout: &[u8]) -> Result<()> {
    if compiled_crates(stdout, workspace) > 0 {
        return Ok(());
    }
    Err(unusable(
        workspace,
        "compiled no crate of",
        "cargo reported no artifact for this workspace. \
         Check that `cargo clippy --workspace` builds there.",
    ))
}

/// A clippy run comply cannot trust, with rustc's own words.
fn unusable(workspace: &Path, what_failed: &str, detail: &str) -> anyhow::Error {
    crate::toolchain::ToolchainError::Unusable {
        what: format!(
            "cargo clippy {what_failed} {}; no Rust file was linted. \
             Repair the build, then re-run comply.\n{detail}",
            workspace.display()
        ),
    }
    .into()
}

/// Cargo's exit code for a build that failed.
const CARGO_FAILED: i32 = 101;

/// Whether cargo's unused-dependency lint alone failed the run.
/// Cargo judges a package once it is built.
/// Any other failure prints an `error` line of its own.
fn is_unused_dependency_denied(code: Option<i32>, stderr: &[u8]) -> bool {
    const FINDING: &str = "error: unused dependency `";
    const TALLY: &str = "error: could not finalize `";
    let stderr = String::from_utf8_lossy(stderr);
    let errors: Vec<&str> = stderr
        .lines()
        .filter(|line| line.starts_with("error"))
        .collect();
    code == Some(CARGO_FAILED)
        && errors.iter().any(|line| line.starts_with(FINDING))
        && errors
            .iter()
            .all(|line| line.starts_with(FINDING) || line.starts_with(TALLY))
}

/// How long `cargo metadata --no-deps` may run.
const METADATA_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// The manifests of `workspace`'s members, as cargo resolves them.
/// Globs, path dependencies and `exclude` all count.
fn workspace_members(workspace: &Path) -> Result<FxHashSet<PathBuf>> {
    let manifest = workspace.join("Cargo.toml");
    let child = Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--format-version=1",
            "--manifest-path",
        ])
        .arg(&manifest)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("failed to invoke `cargo metadata`")?;
    let output = crate::oxlint::drain_and_wait(child, METADATA_TIMEOUT, "cargo metadata")?;
    let what_failed = "could not read the members of";
    let Some(output) = output else {
        let limit = METADATA_TIMEOUT.as_secs();
        let detail = format!("`cargo metadata` ran past {limit}s.");
        return Err(unusable(workspace, what_failed, &detail));
    };
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(unusable(workspace, what_failed, &detail));
    }
    let metadata: schema::Metadata =
        serde_json::from_slice(&output.stdout).context("failed to parse `cargo metadata`")?;
    Ok(metadata
        .packages
        .into_iter()
        .map(|package| canonicalize_or_self(&package.manifest_path))
        .collect())
}

/// The files of `members`' packages, then the others by crate.
/// A file outside every package stays with the first.
fn split_by_membership<'a>(
    workspace: &Path,
    files: &[&'a SourceFile],
    members: &FxHashSet<PathBuf>,
) -> (Vec<&'a SourceFile>, BTreeMap<String, Vec<String>>) {
    let mut inside = Vec::with_capacity(files.len());
    let mut outside: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for &file in files {
        match crate::runner_helpers::owning_manifest(workspace, &file.path) {
            Some((manifest, package)) if !members.contains(&manifest) => outside
                .entry(package)
                .or_default()
                .push(file.path.display().to_string()),
            _ => inside.push(file),
        }
    }
    (inside, outside)
}

/// Names the crates cargo leaves out, and their files.
/// Neither clippy nor cargo-shear sees them.
fn outside_message(workspace: &Path, outside: &BTreeMap<String, Vec<String>>) -> String {
    let crates: Vec<String> = outside
        .iter()
        .map(|(package, files)| format!("{package}: {}", files.join(", ")))
        .collect();
    format!(
        "cargo clippy and cargo-shear did not analyze {} crate(s) outside the cargo \
         workspace {}:\n{}\nMake them members, or give them a [workspace] table",
        outside.len(),
        workspace.display(),
        crates.join("\n")
    )
}

/// Refuse a package of `files` that no clippy pass linted.
/// A denied lint stops the first pass.
/// The capped pass lints past it.
fn ensure_packages_linted(
    workspace: &Path,
    files: &[&SourceFile],
    first: &[u8],
    capped: &[u8],
) -> Result<()> {
    let mut linted = linted_manifests(first);
    linted.extend(linted_manifests(capped));
    let hidden: BTreeSet<String> = files
        .iter()
        .filter_map(|file| crate::runner_helpers::owning_manifest(workspace, &file.path))
        .filter(|(manifest, _)| !linted.contains(manifest))
        .map(|(_, package)| package)
        .collect();
    if hidden.is_empty() {
        return Ok(());
    }
    let quoted: Vec<String> = denied_lints(first)
        .filter_map(|envelope| envelope.message)
        .take(QUOTED_BUILD_ERRORS)
        .map(|lint| lint.rendered.unwrap_or(lint.message))
        .collect();
    Err(crate::toolchain::ToolchainError::Unusable {
        what: format!(
            "cargo clippy did not lint {} in {}, even with lints capped after this denied lint:\n{}",
            hidden.into_iter().collect::<Vec<_>>().join(", "),
            workspace.display(),
            quoted.join("\n")
        ),
    }
    .into())
}

/// The manifests of the packages clippy linted, one target at least.
/// A denied lint proves it; so does an artifact.
/// A build script proves nothing of the code it serves.
fn linted_manifests(stdout: &[u8]) -> FxHashSet<PathBuf> {
    envelopes(stdout)
        .filter(|envelope| envelope.reason == "compiler-artifact")
        .chain(denied_lints(stdout))
        .filter(|envelope| {
            !envelope
                .target
                .as_ref()
                .is_some_and(|target| target.kind.iter().any(|kind| kind == "custom-build"))
        })
        .filter_map(|envelope| envelope.manifest_path)
        .map(|manifest| canonicalize_or_self(Path::new(&manifest)))
        .collect()
}

/// How many dep-info files comply reads for one clippy pass.
const DEP_INFO_FILES: usize = 10_000;
/// The largest dep-info file comply reads.
const DEP_INFO_BYTES: u64 = 16 * 1024 * 1024;

/// The sources rustc read for `members`, from the pass's dep-info.
/// Only the units of this pass count, fresh ones included.
fn compiled_sources(
    workspace: &Path,
    members: &FxHashSet<PathBuf>,
    stdout: &[u8],
) -> Result<FxHashSet<PathBuf>> {
    let units = envelopes(stdout)
        .filter(|envelope| envelope.reason == "compiler-artifact")
        .filter(|envelope| {
            envelope.manifest_path.as_ref().is_some_and(|manifest| {
                members.contains(&canonicalize_or_self(Path::new(manifest)))
            })
        })
        .flat_map(|envelope| envelope.filenames)
        .flat_map(|artifact| dep_info_files(Path::new(&artifact)));
    // Distinct files count: a unit's artifacts share one.
    let mut dep_info = BTreeSet::new();
    for file in units {
        dep_info.insert(file);
        if dep_info.len() > DEP_INFO_FILES {
            return Err(crate::toolchain::ToolchainError::Unusable {
                what: format!(
                    "the clippy pass in {} names more than {DEP_INFO_FILES} dep-info files, \
                     so comply could not tell which Rust files it compiled. \
                     Commit the packages in smaller sets, or exclude some from the cargo workspace",
                    workspace.display()
                ),
            }
            .into());
        }
    }
    let mut sources = FxHashSet::default();
    for file in dep_info {
        sources.extend(dep_info_sources(workspace, &file)?);
    }
    Ok(sources)
}

/// The dep-info files of the unit that wrote `artifact`.
/// Each is named after the artifact, without its `lib` prefix.
/// A stale unit's file may share the directory, in any layout.
fn dep_info_files(artifact: &Path) -> Vec<PathBuf> {
    let Some(dir) = artifact.parent() else {
        return Vec::new();
    };
    let stem = artifact
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    [Some(stem.as_str()), stem.strip_prefix("lib")]
        .into_iter()
        .flatten()
        .map(|unit| dir.join(format!("{unit}.d")))
        .filter(|file| file.is_file())
        .collect()
}

/// The sources one dep-info file lists, resolved against `workspace`.
/// rustc writes each on a `path:` line of its own.
fn dep_info_sources(workspace: &Path, file: &Path) -> Result<Vec<PathBuf>> {
    let unreadable = || format!("failed to read the dep-info file {}", file.display());
    let reader = std::fs::File::open(file).with_context(unreadable)?;
    let size = reader.metadata().with_context(unreadable)?.len();
    if size > DEP_INFO_BYTES {
        return Err(crate::toolchain::ToolchainError::Unusable {
            what: format!(
                "the dep-info file {} holds {size} bytes, past the {} MiB comply reads. \
                 If it is stale, run `cargo clean -p` on its package; \
                 a crate that embeds a whole tree can leave the cargo workspace",
                file.display(),
                DEP_INFO_BYTES / (1024 * 1024)
            ),
        }
        .into());
    }
    let mut sources = Vec::new();
    for line in BufReader::new(reader).lines() {
        let line = line.with_context(unreadable)?;
        if line.starts_with('#') || line.contains(": ") {
            continue;
        }
        if let Some(source) = line.strip_suffix(':') {
            let source = workspace.join(source.replace("\\ ", " "));
            sources.push(canonicalize_or_self(&source));
        }
    }
    Ok(sources)
}

/// rustc errors that stopped the build, as rustc rendered them.
/// A denied clippy lint is a finding, not a build error.
/// rustc's own panic has level `error: internal compiler error`.
fn build_errors(stdout: &[u8]) -> Vec<String> {
    compiler_messages(stdout)
        .filter(|diag| diag.level.starts_with("error"))
        .filter(|diag| !is_clippy_lint(diag))
        .filter(|diag| !diag.message.starts_with("aborting due to"))
        .map(|diag| diag.rendered.unwrap_or(diag.message))
        .collect()
}

/// How many crates of `workspace` cargo reported compiling, fresh ones
/// included. Dependencies live outside the workspace, so they are not
/// evidence that the workspace itself was analyzed.
fn compiled_crates(stdout: &[u8], workspace: &Path) -> usize {
    let root = canonicalize_or_self(workspace);
    envelopes(stdout)
        .filter(|envelope| envelope.reason == "compiler-artifact")
        .filter_map(|envelope| envelope.manifest_path)
        .filter(|manifest| canonicalize_or_self(Path::new(manifest)).starts_with(&root))
        .count()
}

/// The clippy lints whose level the code raised to an error.
fn denied_lints(stdout: &[u8]) -> impl Iterator<Item = CargoMessage> + '_ {
    envelopes(stdout).filter(|envelope| {
        envelope
            .message
            .as_ref()
            .is_some_and(|diag| diag.level == "error" && is_clippy_lint(diag))
    })
}

fn is_clippy_lint(diag: &RustcDiagnostic) -> bool {
    diag.code
        .as_ref()
        .is_some_and(|code| code.code.starts_with("clippy::"))
}

fn compiler_messages(stdout: &[u8]) -> impl Iterator<Item = RustcDiagnostic> + '_ {
    envelopes(stdout)
        .filter(|envelope| envelope.reason == "compiler-message")
        .filter_map(|envelope| envelope.message)
}

fn envelopes(stdout: &[u8]) -> impl Iterator<Item = CargoMessage> + '_ {
    BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<CargoMessage>(&line).ok())
}

/// Parse cargo's JSONL output stream and yield Diagnostic structs for
/// every primary span that lives in `file_filter`.
///
/// We surface every clippy lint, not the ones with an explicit
/// `Backend::Clippy` binding in comply's rule registry:
///
/// - If the lint code is in `remap`, the diagnostic is rewritten with
///   the comply rule_id and remediation severity (this is the "branded"
///   path — the user sees the comply name).
/// - Otherwise, if the lint code starts with `clippy::`, we pass it
///   through as-is. The rule_id stays as `clippy::needless_borrow`
///   (etc.) and the message is clippy's verbatim message. The user
///   doesn't lose the diagnostic because comply doesn't have a
///   first-class binding for it.
/// - Pure rustc warnings without a binding (`dead_code`, `unused_imports`,
///   etc.) are skipped — those are the compiler's job, not comply's.
///   Bind them explicitly via `Backend::Clippy { lint: "..." }` if you
///   want comply to surface them.
///
/// `workspace_root` is the directory containing the `Cargo.toml` we
/// invoked clippy with. Cargo emits span file_name as paths relative
/// to that directory, so we resolve them against the root before
/// matching the user's file filter.
fn parse_clippy_jsonl(
    stdout: &[u8],
    workspace_root: &Path,
    file_filter: &FxHashSet<PathBuf>,
    remap: &FxHashMap<String, &'static RuleMeta>,
) -> Vec<Diagnostic> {
    let reader = BufReader::new(stdout);
    let mut diagnostics = Vec::new();
    // `--all-targets` builds a crate as lib and as test: one finding each.
    let mut seen = FxHashSet::default();

    for line in reader.lines().map_while(Result::ok) {
        let Ok(envelope) = serde_json::from_str::<CargoMessage>(&line) else {
            continue;
        };
        if envelope.reason != "compiler-message" {
            continue;
        }
        let Some(diag) = envelope.message else {
            continue;
        };
        let Some(code) = diag.code.as_ref() else {
            continue;
        };

        // Decide how to surface this lint:
        //   - bound to a comply rule via remap → use the comply meta
        //   - any other clippy::* lint → pass through with raw code
        //   - anything else (rustc warnings without binding) → skip
        let mapped_meta = remap.get(&code.code).copied();
        if mapped_meta.is_none() && !code.code.starts_with("clippy::") {
            continue;
        }

        let Some(span) = diag.spans.iter().find(|s| s.is_primary) else {
            continue;
        };

        let span_path = span_path(span, workspace_root);
        if !file_filter.contains(&span_path) {
            continue;
        }
        let location = (
            span_path.clone(),
            span.line_start,
            span.column_start,
            code.code.clone(),
        );
        if !seen.insert(location) {
            continue;
        }

        let severity = Severity::Error;

        let rule_id = match mapped_meta {
            Some(meta) => std::borrow::Cow::Borrowed(meta.id),
            None => std::borrow::Cow::Owned(code.code.clone()),
        };

        diagnostics.push(Diagnostic {
            path: std::sync::Arc::from(span_path.as_path()),
            line: span.line_start.max(1),
            column: span.column_start.max(1),
            rule_id,
            message: with_disallowed_reason(diag.message, &diag.children, &code.code),
            severity,
            span: None,
        });
    }

    diagnostics
}

/// The file of `span`, in the form `file_filter` holds.
/// Cargo names it relative to the workspace root, or in full.
fn span_path(span: &RustcSpan, workspace_root: &Path) -> PathBuf {
    canonicalize_or_self(&workspace_root.join(&span.file_name))
}

/// Append the `reason` of a `disallowed-*` entry to clippy's message.
/// Provenance notes (`#[warn(clippy::…)]` on by default) name the lint.
fn with_disallowed_reason(message: String, children: &[RustcDiagnostic], code: &str) -> String {
    if !code.starts_with("clippy::disallowed_") {
        return message;
    }
    let reasons: Vec<&str> = children
        .iter()
        .filter(|c| c.level == "note" && !is_lint_level_note(&c.message))
        .map(|c| c.message.as_str())
        .collect();
    if reasons.is_empty() {
        return message;
    }
    format!("{message}: {}", reasons.join("; "))
}

/// Whether a note is rustc's lint-level provenance.
fn is_lint_level_note(message: &str) -> bool {
    [
        "`#[",
        "`-",
        "requested on the command line",
        "the lint level is defined",
    ]
    .iter()
    .any(|prefix| message.starts_with(prefix))
}

/// Which dependencies rustc left unused, per compiled workspace target.
///
/// rustc judges after macro expansion.
/// It sees what a derive emits and what `include!` loads from `OUT_DIR`.
/// cargo-shear reads the sources only, so it misses both.
#[derive(Debug, Default)]
pub struct ExternUsage {
    packages: FxHashMap<PathBuf, PackageExterns>,
    /// cargo runs its `unused_dependencies` lint.
    cargo_lints: bool,
}

/// rustc's verdicts on the targets of one package.
#[derive(Debug, Default)]
pub struct PackageExterns {
    /// Targets cargo reported built, build scripts excluded.
    pub compiled: FxHashSet<(Vec<String>, String)>,
    /// Targets that reported the liveness probe unused.
    /// Only their verdicts reached comply.
    pub live: FxHashSet<(Vec<String>, String)>,
    /// Externs rustc reported unused, per target.
    pub unused: FxHashMap<(Vec<String>, String), FxHashSet<String>>,
    /// rustc reported an unused extern without naming it readably.
    pub unreadable: bool,
    /// The keys cargo's `unused_dependencies` lint reported.
    /// `None` unless that lint judged every target of the package.
    pub cargo_unused: Option<FxHashSet<String>>,
}

impl ExternUsage {
    /// rustc's verdicts on the package of `manifest`, if clippy built it.
    pub fn package(&self, manifest: &Path) -> Option<&PackageExterns> {
        self.packages.get(&canonicalize_or_self(manifest))
    }

    /// Read the artifacts and unused-extern reports of one clippy run.
    /// Crates outside `workspace` are dependencies, so they are left out.
    fn record(&mut self, stdout: &[u8], stderr: &[u8], workspace: &Path) {
        let root = canonicalize_or_self(workspace);
        let mut succeeded = false;
        let mut selected = FxHashSet::default();
        for envelope in envelopes(stdout) {
            if envelope.reason == "build-finished" {
                succeeded = envelope.success == Some(true);
            }
            let (Some(manifest), Some(target)) = (envelope.manifest_path, envelope.target) else {
                continue;
            };
            if target.kind.iter().any(|kind| kind == "custom-build") {
                continue;
            }
            let manifest = canonicalize_or_self(Path::new(&manifest));
            if !manifest.starts_with(&root) {
                continue;
            }
            if envelope.profile.is_some_and(|profile| profile.test) {
                selected.insert(manifest.clone());
            }
            let package = self.packages.entry(manifest).or_default();
            let key = (target.kind, target.name);
            match (envelope.reason.as_str(), envelope.message) {
                ("compiler-artifact", _) => {
                    package.compiled.insert(key);
                }
                ("compiler-message", Some(diag)) if is_unused_extern(&diag) => {
                    match diag.message.split('`').nth(1) {
                        Some(LIVENESS_PROBE) => {
                            package.live.insert(key);
                        }
                        Some(name) if !name.is_empty() => {
                            package
                                .unused
                                .entry(key)
                                .or_default()
                                .insert(name.to_owned());
                        }
                        _ => package.unreadable = true,
                    }
                }
                _ => {}
            }
        }
        // cargo lints the packages it was asked for, once all built.
        let reports = cargo_reports(stderr, &root).filter(|_| self.cargo_lints && succeeded);
        let Some(mut reports) = reports else {
            return;
        };
        for manifest in selected {
            let unused = reports.remove(&manifest).unwrap_or_default();
            self.packages.entry(manifest).or_default().cargo_unused = Some(unused);
        }
    }
}

/// The keys cargo's `unused_dependencies` lint reported, per manifest.
/// Each report reads ``warning: unused dependency `key` ``.
/// Its next line reads ` --> path:line:column`, relative to `root`.
/// `None` when one report is unreadable.
fn cargo_reports(stderr: &[u8], root: &Path) -> Option<FxHashMap<PathBuf, FxHashSet<String>>> {
    let text = String::from_utf8_lossy(stderr);
    let mut lines = text.lines();
    let mut reports: FxHashMap<PathBuf, FxHashSet<String>> = FxHashMap::default();
    while let Some(line) = lines.next() {
        if !line.contains("unused dependency") {
            continue;
        }
        let key = ["warning: ", "error: "]
            .iter()
            .find_map(|level| line.strip_prefix(level))?
            .strip_prefix("unused dependency `")?
            .strip_suffix('`')?;
        let path = lines.next()?.trim_start().strip_prefix("--> ")?;
        let manifest = canonicalize_or_self(&root.join(without_position(path)));
        reports.entry(manifest).or_default().insert(key.to_owned());
    }
    Some(reports)
}

/// `path` without a trailing `:line:column`.
fn without_position(path: &str) -> &str {
    without_number(without_number(path))
}

/// `text` without a trailing `:` and number.
fn without_number(text: &str) -> &str {
    match text.rsplit_once(':') {
        Some((head, number))
            if !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) =>
        {
            head
        }
        _ => text,
    }
}

/// Whether cargo runs its own `unused_dependencies` lint.
/// It does from 1.100, swallowing rustc's unused-extern warnings.
fn cargo_lints_unused_dependencies() -> bool {
    Command::new("cargo")
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|text| {
            text.split_whitespace()
                .nth(1)
                .is_some_and(|version| version_at_least(version, 1, 100))
        })
}

/// Whether the `major.minor` that `version` starts with reaches the given one.
/// `1.79`, `1.100.0` and `1.101.0-nightly` all parse.
pub(crate) fn version_at_least(version: &str, major: u64, minor: u64) -> bool {
    let mut parts = version.split(['.', '-']).map(str::parse::<u64>);
    match (parts.next(), parts.next()) {
        (Some(Ok(found_major)), Some(Ok(found_minor))) => {
            (found_major, found_minor) >= (major, minor)
        }
        _ => false,
    }
}

/// An `unused_crate_dependencies` warning.
/// rustc words it ``extern crate `name` is unused in crate `krate` ``.
fn is_unused_extern(diag: &RustcDiagnostic) -> bool {
    diag.code
        .as_ref()
        .is_some_and(|code| code.code == "unused_crate_dependencies")
}

/// Canonicalize the path if possible, otherwise return it as-is.
/// We need canonical paths so that filter matches work even when the
/// user passed a relative path on the CLI but cargo emits absolute.
pub(crate) fn canonicalize_or_self(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create each of `files` under `dir`, empty.
    fn touch(dir: &Path, files: &[&str]) {
        for file in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
    }

    #[test]
    fn dep_info_files_name_only_the_unit_of_the_artifact() {
        // Flat layouts up to 1.99, then the per-unit one.
        let layouts = [
            ("deps/libcore_x-1.rmeta", "deps/core_x-1.d"),
            ("examples/libex-1.rmeta", "examples/ex-1.d"),
            ("examples/ex-1", "examples/ex-1.d"),
            ("debug/comply", "debug/comply.d"),
            (
                "build/comply/1/out/comply-1",
                "build/comply/1/out/comply-1.d",
            ),
        ];
        for (artifact, own) in layouts {
            let dir = tempfile::TempDir::new().unwrap();
            let artifact = dir.path().join(artifact);
            // A stale unit of a removed target shares the directory.
            let stale = artifact.with_file_name("stale-9.d");
            touch(dir.path(), &[own]);
            std::fs::write(&stale, "").unwrap();
            std::fs::write(&artifact, "").unwrap();
            let found = dep_info_files(&artifact);
            assert_eq!(found, vec![dir.path().join(own)], "{artifact:?}");
        }
    }

    /// One `compiler-artifact` line per artifact, from `manifest`.
    fn artifact_lines(manifest: &Path, artifacts: &[PathBuf]) -> Vec<u8> {
        artifacts
            .iter()
            .map(|artifact| {
                let line = serde_json::json!({
                    "reason": "compiler-artifact",
                    "manifest_path": manifest,
                    "filenames": [artifact],
                });
                format!("{line}\n")
            })
            .collect::<String>()
            .into_bytes()
    }

    #[test]
    fn compiled_sources_reads_only_the_own_dep_info_of_member_units() {
        let dir = tempfile::TempDir::new().unwrap();
        let ws = canonicalize_or_self(dir.path());
        let manifest = ws.join("Cargo.toml");
        let examples = ws.join("target/debug/examples");
        std::fs::create_dir_all(&examples).unwrap();
        // Flat layout: a stale example's dep-info shares the directory.
        std::fs::write(examples.join("ex-1.d"), "examples/ex/main.rs:\n").unwrap();
        std::fs::write(examples.join("gone-9.d"), "examples/gone/helper.rs:\n").unwrap();
        let mut stdout = artifact_lines(&manifest, &[examples.join("ex-1")]);
        let other = ws.join("vendor/Cargo.toml");
        stdout.extend(artifact_lines(&other, &[examples.join("gone-9")]));
        let members = FxHashSet::from_iter([manifest]);
        let sources = compiled_sources(&ws, &members, &stdout).unwrap();
        let main = ws.join("examples/ex/main.rs");
        assert_eq!(sources, FxHashSet::from_iter([main]));
    }

    #[test]
    fn compiled_sources_counts_distinct_dep_info_files_up_to_its_bound() {
        let dir = tempfile::TempDir::new().unwrap();
        let ws = canonicalize_or_self(dir.path());
        let manifest = ws.join("Cargo.toml");
        std::fs::create_dir_all(ws.join("deps")).unwrap();
        let over = DEP_INFO_FILES.saturating_add(1);
        let units: Vec<PathBuf> = (0..over).map(|n| ws.join(format!("deps/u-{n}"))).collect();
        for unit in &units {
            std::fs::write(unit.with_extension("d"), "").unwrap();
        }
        let members = FxHashSet::from_iter([manifest.clone()]);
        let run =
            |units: &[PathBuf]| compiled_sources(&ws, &members, &artifact_lines(&manifest, units));
        assert!(run(&units[..DEP_INFO_FILES.saturating_sub(1)]).is_ok());
        assert!(run(&units[..DEP_INFO_FILES]).is_ok());
        let error = run(&units).expect_err("one dep-info file past the bound");
        assert!(error.to_string().contains("smaller sets"), "{error}");
        // A unit named twice counts once, before the bound applies.
        let first = &units[..1];
        let twice: Vec<PathBuf> = first
            .iter()
            .chain(&units[..DEP_INFO_FILES])
            .cloned()
            .collect();
        assert!(run(&twice).is_ok());
        let twice_over: Vec<PathBuf> = first.iter().chain(&units).cloned().collect();
        assert!(run(&twice_over).is_err());
    }

    #[test]
    fn dep_info_sources_lists_the_path_lines_only() {
        let dir = tempfile::TempDir::new().unwrap();
        let ws = canonicalize_or_self(dir.path());
        let file = ws.join("ex-1.d");
        let dep_info = "/t/ex-1: src/main.rs src/a\\ b.rs\n\nsrc/main.rs:\nsrc/a\\ b.rs:\n\n\
                        # env-dep:X=1\n";
        std::fs::write(&file, dep_info).unwrap();
        let sources = dep_info_sources(&ws, &file).unwrap();
        assert_eq!(sources, [ws.join("src/main.rs"), ws.join("src/a b.rs")]);
    }

    #[test]
    fn dep_info_sources_reads_files_up_to_its_bound() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("big.d");
        let sized = |len: u64| {
            std::fs::File::create(&file).unwrap().set_len(len).unwrap();
            dep_info_sources(dir.path(), &file)
        };
        assert!(sized(DEP_INFO_BYTES.saturating_sub(1)).is_ok());
        assert!(sized(DEP_INFO_BYTES).is_ok());
        let error = sized(DEP_INFO_BYTES.saturating_add(1)).expect_err("one byte past it");
        assert!(error.to_string().contains("16 MiB"), "{error}");
    }

    #[test]
    fn extern_usage_reads_artifacts_and_unused_extern_warnings() {
        let ws = std::env::temp_dir().join("comply-extern-usage");
        let app = ws.join("app/Cargo.toml");
        let line = |reason: &str, kind: &str, message: &str| {
            format!(
                r#"{{"reason":"{reason}","manifest_path":"{}","target":{{"name":"app","kind":["{kind}"]}}{message}}}"#,
                app.display()
            )
        };
        let unused = |text: &str| {
            format!(
                r#","message":{{"message":"{text}","level":"warning","code":{{"code":"unused_crate_dependencies"}}}}"#
            )
        };
        let stream = [
            line("compiler-message", "lib", &unused("extern crate `idle` is unused in crate `app`")),
            line("compiler-message", "lib", &unused("extern crate `comply_liveness_probe` is unused in crate `app`")),
            line("compiler-artifact", "lib", ""),
            line("compiler-artifact", "custom-build", ""),
            r#"{"reason":"compiler-artifact","manifest_path":"/registry/dep/Cargo.toml","target":{"name":"dep","kind":["lib"]}}"#.to_owned(),
        ]
        .join("\n");
        let mut usage = ExternUsage::default();
        usage.record(stream.as_bytes(), b"", &ws);

        let package = usage.package(&app).expect("app is a workspace crate");
        let lib = (vec!["lib".to_owned()], "app".to_owned());
        assert_eq!(
            package.compiled.len(),
            1,
            "a build script is no target to prove with"
        );
        assert!(package.compiled.contains(&lib));
        assert!(package.live.contains(&lib));
        assert!(package.unused[&lib].contains("idle"));
        assert!(!package.unused[&lib].contains(LIVENESS_PROBE));
        assert!(!package.unreadable);
        assert!(
            usage
                .package(Path::new("/registry/dep/Cargo.toml"))
                .is_none()
        );

        let mut garbled = ExternUsage::default();
        garbled.record(
            line("compiler-message", "lib", &unused("unused crate")).as_bytes(),
            b"",
            &ws,
        );
        assert!(garbled.package(&app).unwrap().unreadable);
    }

    /// Cargo names the crate it built; a dependency sits outside the
    /// workspace, so its artifact proves nothing about this one.
    #[test]
    fn compiled_crates_counts_only_the_artifacts_of_this_workspace() {
        let json = br#"{"reason":"compiler-artifact","manifest_path":"/ws/member/Cargo.toml","fresh":true}
{"reason":"compiler-artifact","manifest_path":"/elsewhere/serde/Cargo.toml","fresh":true}
{"reason":"compiler-message","manifest_path":"/ws/member/Cargo.toml","message":{"message":"x","level":"warning","spans":[]}}
{"reason":"build-finished","success":true}"#;
        assert_eq!(compiled_crates(json, Path::new("/ws")), 1);
        assert_eq!(compiled_crates(json, Path::new("/other")), 0);
    }

    #[test]
    fn only_a_denied_unused_dependency_explains_a_cargo_failure() {
        let lint = "error: unused dependency `idle`\n  --> Cargo.toml:9:1\n   |\n   \
                    = note: `cargo::unused_dependencies` is set to `deny` in `[lints]`\n\
                    error: could not finalize `app` (manifest) due to 1 previous error\n";
        assert!(is_unused_dependency_denied(Some(101), lint.as_bytes()));
        // A killed cargo judged nothing for sure.
        assert!(!is_unused_dependency_denied(None, lint.as_bytes()));
        for other in [
            "error: could not compile `b` (lib)\n",
            "error: failed to run custom build command for `b v0.1.0`\n",
            "error[E0583]: file not found for module `gone`\n",
        ] {
            let stderr = format!("{lint}{other}");
            assert!(
                !is_unused_dependency_denied(Some(101), stderr.as_bytes()),
                "{other}"
            );
        }
        assert!(!is_unused_dependency_denied(Some(101), b""));
        assert!(!is_unused_dependency_denied(
            Some(101),
            b"error: could not finalize `app` (manifest) due to 1 previous error\n"
        ));
    }

    #[test]
    fn build_errors_keeps_rustc_errors_and_leaves_denied_clippy_lints() {
        let json = br#"{"reason":"compiler-message","message":{"message":"mismatched types","code":{"code":"E0308"},"level":"error","rendered":"error[E0308]: mismatched types","spans":[]}}
{"reason":"compiler-message","message":{"message":"error communicating with database","code":null,"level":"error","spans":[]}}
{"reason":"compiler-message","message":{"message":"used unwrap","code":{"code":"clippy::unwrap_used"},"level":"error","spans":[]}}
{"reason":"compiler-message","message":{"message":"no errors encountered","code":null,"level":"error: internal compiler error","spans":[]}}
{"reason":"compiler-message","message":{"message":"aborting due to 2 previous errors","code":null,"level":"error","spans":[]}}
{"reason":"compiler-message","message":{"message":"unused variable","code":{"code":"unused_variables"},"level":"warning","spans":[]}}
{"reason":"build-finished","success":false}"#;
        assert_eq!(
            build_errors(json),
            vec![
                "error[E0308]: mismatched types".to_owned(),
                "error communicating with database".to_owned(),
                "no errors encountered".to_owned()
            ]
        );
    }

    /// What cargo 1.100 prints for an unused `idle` in `app`.
    const CARGO_REPORT: &str = "warning: unused dependency `idle`\n \
        --> app/Cargo.toml:6:1\n  |\n6 | idle = { path = \"../idle\" }\n  | ^^^^\n  |\n  \
        = note: `cargo::unused_dependencies` is set to `warn` by default\n\
        help: consider removing the dependency on `idle`\n";

    /// The `app` package's cargo verdict after one clippy run.
    fn cargo_verdict(lints: bool, stdout: &str, stderr: &str) -> Option<FxHashSet<String>> {
        let ws = std::env::temp_dir().join("comply-cargo-reports");
        let mut usage = ExternUsage {
            cargo_lints: lints,
            ..ExternUsage::default()
        };
        let stdout = stdout.replace("APP", &ws.join("app/Cargo.toml").display().to_string());
        usage.record(stdout.as_bytes(), stderr.as_bytes(), &ws);
        usage
            .package(&ws.join("app/Cargo.toml"))
            .and_then(|package| package.cargo_unused.clone())
    }

    #[test]
    fn cargo_reports_count_only_where_cargo_judged_every_target() {
        let artifact = |test: bool| {
            format!(
                r#"{{"reason":"compiler-artifact","manifest_path":"APP","target":{{"name":"app","kind":["lib"]}},"profile":{{"test":{test}}}}}"#
            )
        };
        let done = |success: bool| format!(r#"{{"reason":"build-finished","success":{success}}}"#);
        let judged = format!("{}\n{}\n{}", artifact(false), artifact(true), done(true));
        let verdict = cargo_verdict(true, &judged, CARGO_REPORT).expect("cargo judged app");
        assert_eq!(verdict, ["idle".to_owned()].into_iter().collect());
        assert_eq!(cargo_verdict(true, &judged, ""), Some(FxHashSet::default()));
        // Before 1.100, cargo has no such lint: its silence is empty.
        assert_eq!(cargo_verdict(false, &judged, ""), None);
        // A failed unit never reported its unused externs to cargo.
        let failed = format!("{}\n{}\n{}", artifact(false), artifact(true), done(false));
        assert_eq!(cargo_verdict(true, &failed, ""), None);
        // Built only as a dependency, the package went unlinted.
        let dependency = format!("{}\n{}", artifact(false), done(true));
        assert_eq!(cargo_verdict(true, &dependency, ""), None);
        let garbled = "warning: unused dependency idle\n";
        assert_eq!(cargo_verdict(true, &judged, garbled), None);
    }

    #[test]
    fn version_at_least_reads_major_and_minor() {
        assert!(version_at_least("1.100.0", 1, 100));
        assert!(version_at_least("1.101.0-nightly", 1, 100));
        assert!(version_at_least("1.79", 1, 79));
        assert!(!version_at_least("1.99.1", 1, 100));
        assert!(!version_at_least("1.75", 1, 79));
        assert!(!version_at_least("stable", 1, 79));
    }

    /// `ensure_packages_linted` over `files` of members `a` and `b`.
    /// The first pass stops at a denied lint of `a`.
    fn judge_packages(files: &[&str], capped: &[&str]) -> Result<()> {
        let dir = tempfile::TempDir::new().unwrap();
        let root = canonicalize_or_self(dir.path());
        let manifest = |name: &str| root.join(name).join("Cargo.toml");
        for name in ["a", "b"] {
            std::fs::create_dir_all(root.join(name).join("src")).unwrap();
            let package = format!("[package]\nname = \"{name}\"\n");
            std::fs::write(manifest(name), package).unwrap();
            std::fs::write(root.join(name).join("src/lib.rs"), "").unwrap();
        }
        let first = format!(
            r#"{{"reason":"compiler-message","manifest_path":"{}","message":{{"message":"x","code":{{"code":"clippy::absurd_extreme_comparisons"}},"level":"error","rendered":"error: x < 0","spans":[]}}}}"#,
            manifest("a").display()
        );
        let capped: Vec<String> = capped
            .iter()
            .map(|name| {
                format!(
                    r#"{{"reason":"compiler-artifact","manifest_path":"{}","target":{{"kind":["lib"]}}}}"#,
                    manifest(name).display()
                )
            })
            .collect();
        let files: Vec<SourceFile> = files
            .iter()
            .map(|name| SourceFile {
                path: root.join(name).join("src/lib.rs"),
                language: crate::files::Language::Rust,
            })
            .collect();
        let files: Vec<&SourceFile> = files.iter().collect();
        ensure_packages_linted(
            &root,
            &files,
            first.as_bytes(),
            capped.join("\n").as_bytes(),
        )
    }

    #[test]
    fn a_package_the_capped_pass_lints_is_judged() {
        assert!(judge_packages(&["b"], &["a", "b"]).is_ok());
        // The denied lint proves `a` was linted.
        assert!(judge_packages(&["a"], &[]).is_ok());
    }

    #[test]
    fn a_package_no_pass_lints_is_refused() {
        let refusal = judge_packages(&["a", "b"], &["a"]).unwrap_err().to_string();
        assert!(refusal.contains("did not lint b in"), "{refusal}");
        assert!(refusal.contains("x < 0"), "{refusal}");
    }

    #[test]
    fn files_of_a_crate_outside_the_members_are_set_apart() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = canonicalize_or_self(dir.path());
        for (path, content) in [
            ("a/Cargo.toml", "[package]\nname = \"a\"\n"),
            ("a/src/lib.rs", ""),
            ("tools/x/Cargo.toml", "[package]\nname = \"x\"\n"),
            ("tools/x/src/lib.rs", ""),
            ("tools/x/src/main.rs", ""),
            ("scripts/loose.rs", ""),
        ] {
            let full = root.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, content).unwrap();
        }
        let files: Vec<SourceFile> = [
            "a/src/lib.rs",
            "tools/x/src/lib.rs",
            "scripts/loose.rs",
            "tools/x/src/main.rs",
        ]
        .iter()
        .map(|path| SourceFile {
            path: root.join(path),
            language: crate::files::Language::Rust,
        })
        .collect();
        let files: Vec<&SourceFile> = files.iter().collect();
        let members = FxHashSet::from_iter([root.join("a/Cargo.toml")]);

        let (inside, outside) = split_by_membership(&root, &files, &members);

        let inside: Vec<&Path> = inside.iter().map(|f| f.path.as_path()).collect();
        assert_eq!(
            inside,
            [root.join("a/src/lib.rs"), root.join("scripts/loose.rs")]
        );
        let x = [
            root.join("tools/x/src/lib.rs").display().to_string(),
            root.join("tools/x/src/main.rs").display().to_string(),
        ];
        assert_eq!(outside, BTreeMap::from([("x".to_owned(), x.to_vec())]));
        let message = outside_message(&root, &outside);
        assert!(message.contains("1 crate(s) outside"), "{message}");
        assert!(message.ends_with("[workspace] table"), "{message}");
        assert!(
            message.contains(&format!("x: {}, {}", x[0], x[1])),
            "{message}"
        );
    }

    #[test]
    fn parse_clippy_jsonl_extracts_diagnostics_for_filtered_files() {
        const META: RuleMeta = RuleMeta {
            id: "rust-no-unwrap",
            description: "no unwrap",
            remediation: "use ?",
            severity: Severity::Error,
            doc_url: None,
            categories: &[],
            skip_in_test_dir: false,
            skip_in_relaxed_dir: false,
        };
        let mut remap: FxHashMap<String, &'static RuleMeta> = FxHashMap::default();
        remap.insert("clippy::unwrap_used".to_string(), &META);

        let json = br#"{"reason":"compiler-message","message":{"message":"used unwrap","code":{"code":"clippy::unwrap_used"},"level":"warning","spans":[{"file_name":"/abs/src/main.rs","line_start":10,"column_start":5,"is_primary":true}]}}
{"reason":"build-finished","success":true}"#;

        let mut filter = FxHashSet::default();
        filter.insert(PathBuf::from("/abs/src/main.rs"));

        let diagnostics = parse_clippy_jsonl(json, Path::new("/abs"), &filter, &remap);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule_id, "rust-no-unwrap");
        assert_eq!(diagnostics[0].line, 10);
        assert_eq!(diagnostics[0].column, 5);
    }

    #[test]
    fn parse_clippy_jsonl_filters_out_unrelated_files() {
        const META: RuleMeta = RuleMeta {
            id: "rust-no-unwrap",
            description: "no unwrap",
            remediation: "use ?",
            severity: Severity::Error,
            doc_url: None,
            categories: &[],
            skip_in_test_dir: false,
            skip_in_relaxed_dir: false,
        };
        let mut remap: FxHashMap<String, &'static RuleMeta> = FxHashMap::default();
        remap.insert("clippy::unwrap_used".to_string(), &META);

        let json = br#"{"reason":"compiler-message","message":{"message":"used unwrap","code":{"code":"clippy::unwrap_used"},"level":"warning","spans":[{"file_name":"/abs/other.rs","line_start":1,"column_start":1,"is_primary":true}]}}"#;

        let mut filter = FxHashSet::default();
        filter.insert(PathBuf::from("/abs/wanted.rs"));

        let diagnostics = parse_clippy_jsonl(json, Path::new("/abs"), &filter, &remap);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn parse_clippy_jsonl_skips_non_compiler_messages() {
        let remap: FxHashMap<String, &'static RuleMeta> = FxHashMap::default();
        let filter: FxHashSet<PathBuf> = FxHashSet::default();
        let json = br#"{"reason":"build-finished","success":true}
{"reason":"compiler-artifact","package_id":"x"}"#;
        let diagnostics = parse_clippy_jsonl(json, Path::new("/abs"), &filter, &remap);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn parse_clippy_jsonl_passes_through_unbound_clippy_lints() {
        // No remap entry — but the lint code starts with `clippy::`,
        // so we keep the diagnostic with its raw rule_id.
        let remap: FxHashMap<String, &'static RuleMeta> = FxHashMap::default();
        let json = br#"{"reason":"compiler-message","message":{"message":"needless borrow","code":{"code":"clippy::needless_borrow"},"level":"warning","spans":[{"file_name":"/abs/src/main.rs","line_start":4,"column_start":2,"is_primary":true}]}}"#;
        let mut filter = FxHashSet::default();
        filter.insert(PathBuf::from("/abs/src/main.rs"));

        let diagnostics = parse_clippy_jsonl(json, Path::new("/abs"), &filter, &remap);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule_id, "clippy::needless_borrow");
        assert_eq!(diagnostics[0].message, "needless borrow");
        assert_eq!(diagnostics[0].severity, Severity::Error);
    }

    #[test]
    fn parse_clippy_jsonl_skips_unbound_rustc_warnings() {
        // `dead_code` is a rustc lint, not clippy, and it has no
        // explicit binding — comply leaves it to the compiler.
        let remap: FxHashMap<String, &'static RuleMeta> = FxHashMap::default();
        let json = br#"{"reason":"compiler-message","message":{"message":"function `foo` is never used","code":{"code":"dead_code"},"level":"warning","spans":[{"file_name":"/abs/src/main.rs","line_start":1,"column_start":1,"is_primary":true}]}}"#;
        let mut filter = FxHashSet::default();
        filter.insert(PathBuf::from("/abs/src/main.rs"));

        let diagnostics = parse_clippy_jsonl(json, Path::new("/abs"), &filter, &remap);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn parse_clippy_jsonl_appends_disallowed_reason() {
        let remap: FxHashMap<String, &'static RuleMeta> = FxHashMap::default();
        let json = br#"{"reason":"compiler-message","message":{"message":"use of a disallowed method `std::env::var`","code":{"code":"clippy::disallowed_methods"},"level":"warning","spans":[{"file_name":"/abs/src/main.rs","line_start":3,"column_start":5,"is_primary":true}],"children":[{"message":"read Config","level":"note","spans":[],"children":[]},{"message":"for further information visit https://rust-lang.github.io/rust-clippy","level":"help","spans":[],"children":[]},{"message":"`#[warn(clippy::disallowed_methods)]` on by default","level":"note","spans":[],"children":[]}]}}"#;
        let mut filter = FxHashSet::default();
        filter.insert(PathBuf::from("/abs/src/main.rs"));

        let diagnostics = parse_clippy_jsonl(json, Path::new("/abs"), &filter, &remap);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "use of a disallowed method `std::env::var`: read Config"
        );
    }

    #[test]
    fn parse_clippy_jsonl_reports_a_span_once_across_targets() {
        let remap: FxHashMap<String, &'static RuleMeta> = FxHashMap::default();
        let line = r#"{"reason":"compiler-message","message":{"message":"length comparison to zero","code":{"code":"clippy::len_zero"},"level":"warning","spans":[{"file_name":"/abs/src/lib.rs","line_start":2,"column_start":8,"is_primary":true}]}}"#;
        let json = format!("{line}\n{line}\n");
        let mut filter = FxHashSet::default();
        filter.insert(PathBuf::from("/abs/src/lib.rs"));

        let diagnostics = parse_clippy_jsonl(json.as_bytes(), Path::new("/abs"), &filter, &remap);
        assert_eq!(diagnostics.len(), 1);
    }
}
