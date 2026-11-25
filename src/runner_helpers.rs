//! Shared helpers for the external-tool subprocess runners.
//!
//! Every runner (clippy, cargo-shear, …) needs a binary probe ("is this CLI
//! tool installed?") and the cargo workspace root of a source file.
//!
//! Before this module existed each runner reimplemented all three.
//! Five runners × ~36 lines each was the dominant clone-detection
//! signal in `comply` itself, plus a real Rule of Three violation.
//!
//! The runners still own their availability `OnceLock` because each
//! tool needs its own cached probe — but the BODY of that probe is
//! now `runner_helpers::probe_binary("cargo", &["shear", "--version"])`,
//! which is one line.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::files::SourceFile;

/// True if running `<cmd> <args...>` succeeds. Used by every runner's
/// `is_available()` to test for the presence of the underlying CLI tool.
/// Errors (binary not on PATH, exec failure) all collapse to `false`.
#[must_use]
pub fn probe_binary(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Walk parents of `path` to the root of the *cargo workspace* it
/// belongs to: the nearest ancestor `Cargo.toml` that declares a
/// `[workspace]` table. Falls back to the nearest ancestor `Cargo.toml`
/// (a standalone crate) when none declares a workspace. Returns the
/// directory, not the manifest path.
///
/// The first `Cargo.toml` of a workspace member is the member's own —
/// grouping by it makes a runner compile each member separately instead of
/// once per workspace. Resolving to the workspace root lets one `cargo`
/// invocation cover every member with a shared build.
///
/// Detection is a textual `[workspace]` probe.
/// A crate in `workspace.exclude` resolves to itself.
/// Cargo builds such a crate on its own.
///
/// Pick this resolver for runners that accept `--workspace` and benefit
/// from a shared build (clippy lint, cargo-shear).
#[must_use]
pub fn find_cargo_workspace_root(path: &Path) -> Option<PathBuf> {
    let canonical = path.canonicalize().ok()?;
    let mut current = canonical.parent();
    let mut nearest = None;
    while let Some(dir) = current {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.is_file() {
            if nearest.is_none() {
                nearest = Some(dir.to_path_buf());
            }
            let workspace = std::fs::read_to_string(&cargo_toml)
                .ok()
                .filter(|content| content.contains("[workspace]"));
            if let Some(content) = workspace {
                let excluded_crate = nearest.filter(|crate_dir| {
                    crate_dir.as_path() != dir && excludes(&content, dir, crate_dir)
                });
                return excluded_crate.or_else(|| Some(dir.to_path_buf()));
            }
        }
        current = dir.parent();
    }
    nearest
}

/// Whether `workspace.exclude` of the `root` manifest covers `crate_dir`.
/// An excluded ancestor under `root` covers it too.
/// Runs from `root` with `--package` or `--workspace` miss it.
fn excludes(content: &str, root: &Path, crate_dir: &Path) -> bool {
    let Ok(manifest) = content.parse::<toml::Table>() else {
        return false;
    };
    let Some(excluded) = manifest
        .get("workspace")
        .and_then(|w| w.get("exclude"))
        .and_then(toml::Value::as_array)
    else {
        return false;
    };
    excluded
        .iter()
        .filter_map(toml::Value::as_str)
        .any(|entry| crate_dir.starts_with(root.join(entry.trim_end_matches('/'))))
}

/// Cargo's package selection for a clippy run over `files` in `workspace`:
/// one `-p <name>` per package owning a file, so a scoped run builds only
/// those crates. Falls back to `--workspace` when a file has no package.
#[must_use]
pub fn cargo_package_args(workspace: &Path, files: &[&SourceFile]) -> Vec<String> {
    let names: Option<std::collections::BTreeSet<String>> = files
        .iter()
        .map(|f| owning_manifest(workspace, &f.path).map(|(_, name)| name))
        .collect();
    match names {
        Some(names) if !names.is_empty() => names
            .into_iter()
            .flat_map(|name| ["-p".to_owned(), name])
            .collect(),
        _ => vec!["--workspace".to_owned()],
    }
}

/// The nearest `[package]` manifest above `file`, with the package's name.
/// Searched up to `workspace` (canonical) inclusive.
#[must_use]
pub fn owning_manifest(workspace: &Path, file: &Path) -> Option<(PathBuf, String)> {
    let canonical = file.canonicalize().ok()?;
    canonical
        .ancestors()
        .skip(1)
        .take_while(|dir| dir.starts_with(workspace))
        .find_map(|dir| {
            let path = dir.join("Cargo.toml");
            let manifest: toml::Value = std::fs::read_to_string(&path).ok()?.parse().ok()?;
            let name = manifest.get("package")?.get("name")?.as_str()?.to_owned();
            Some((path, name))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_unknown_binary_returns_false() {
        assert!(!probe_binary(
            "definitely-not-a-real-binary-xyz",
            &["--version"]
        ));
    }

    #[test]
    fn workspace_root_resolves_member_to_root_with_workspace_table() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"member\"]\n",
        )
        .unwrap();
        let member_src = root.join("member").join("src");
        std::fs::create_dir_all(&member_src).unwrap();
        std::fs::write(
            root.join("member").join("Cargo.toml"),
            "[package]\nname = \"member\"\n",
        )
        .unwrap();
        let file = member_src.join("lib.rs");
        std::fs::write(&file, "").unwrap();

        // Resolves to the workspace root, NOT the member manifest, so one
        // clippy run covers every member.
        assert_eq!(
            find_cargo_workspace_root(&file),
            Some(std::fs::canonicalize(root).unwrap())
        );
    }

    #[test]
    fn workspace_root_of_an_excluded_crate_is_the_crate_itself() {
        let fixture = tempfile::TempDir::new().unwrap();
        let root = std::fs::canonicalize(fixture.path()).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"member\"]\nexclude = [\"tools/excl\"]\n",
        )
        .unwrap();
        for (dir, name) in [
            ("member", "member"),
            ("tools/excl", "excl"),
            ("tools/other", "other"),
        ] {
            std::fs::create_dir_all(root.join(dir).join("src")).unwrap();
            std::fs::write(
                root.join(dir).join("Cargo.toml"),
                format!("[package]\nname = \"{name}\"\n"),
            )
            .unwrap();
            std::fs::write(root.join(dir).join("src/lib.rs"), "").unwrap();
        }
        let of = |dir: &str| find_cargo_workspace_root(&root.join(dir).join("src/lib.rs"));
        assert_eq!(of("tools/excl"), Some(root.join("tools/excl")));
        assert_eq!(of("member"), Some(root.clone()));
        assert_eq!(of("tools/other"), Some(root));
    }

    #[test]
    fn workspace_root_is_none_when_no_ancestor_manifest() {
        let tmp = tempfile::TempDir::new().unwrap();
        let file = tmp.path().join("loose.rs");
        std::fs::write(&file, "").unwrap();
        // No Cargo.toml in this branch (TempDir lives under the system temp
        // dir, which has no manifest) → neither a workspace nor a crate.
        assert_eq!(find_cargo_workspace_root(&file), None);
    }

    #[test]
    fn workspace_root_is_noop_for_standalone_crate() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"solo\"\n").unwrap();
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let file = src.join("main.rs");
        std::fs::write(&file, "").unwrap();

        // No `[workspace]` anywhere: falls back to the crate's own dir,
        // matching the previous nearest-Cargo.toml behavior.
        assert_eq!(
            find_cargo_workspace_root(&file),
            Some(std::fs::canonicalize(root).unwrap())
        );
    }

    /// Writes `files` (path, content) under a fresh temp dir.
    fn fixture(files: &[(&str, &str)]) -> tempfile::TempDir {
        let tmp = tempfile::TempDir::new().unwrap();
        for (path, content) in files {
            let full = tmp.path().join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, content).unwrap();
        }
        tmp
    }

    fn package_args_for(tmp: &tempfile::TempDir, paths: &[&str]) -> Vec<String> {
        let files: Vec<SourceFile> = paths
            .iter()
            .map(|p| SourceFile {
                path: tmp.path().join(p),
                language: crate::files::Language::Rust,
            })
            .collect();
        let refs: Vec<&SourceFile> = files.iter().collect();
        let root = find_cargo_workspace_root(&refs[0].path).unwrap();
        cargo_package_args(&root, &refs)
    }

    #[test]
    fn package_args_select_only_the_members_owning_the_files() {
        let tmp = fixture(&[
            ("Cargo.toml", "[workspace]\nmembers = [\"a\", \"b\"]\n"),
            ("a/Cargo.toml", "[package]\nname = \"a\"\n"),
            ("a/src/lib.rs", ""),
            ("a/tests/it.rs", ""),
            ("b/Cargo.toml", "[package]\nname = \"b\"\n"),
            ("b/src/lib.rs", ""),
        ]);
        assert_eq!(
            package_args_for(&tmp, &["a/src/lib.rs", "a/tests/it.rs"]),
            ["-p", "a"]
        );
        assert_eq!(
            package_args_for(&tmp, &["b/src/lib.rs", "a/src/lib.rs"]),
            ["-p", "a", "-p", "b"]
        );
    }

    #[test]
    fn package_args_select_a_root_package() {
        let tmp = fixture(&[
            (
                "Cargo.toml",
                "[workspace]\nmembers = [\"a\"]\n\n[package]\nname = \"root\"\n",
            ),
            ("src/main.rs", ""),
            ("a/Cargo.toml", "[package]\nname = \"a\"\n"),
        ]);
        assert_eq!(package_args_for(&tmp, &["src/main.rs"]), ["-p", "root"]);
    }

    #[test]
    fn package_args_fall_back_to_workspace_for_a_file_outside_any_package() {
        let tmp = fixture(&[
            ("Cargo.toml", "[workspace]\nmembers = [\"a\"]\n"),
            ("a/Cargo.toml", "[package]\nname = \"a\"\n"),
            ("a/src/lib.rs", ""),
            ("scripts/loose.rs", ""),
        ]);
        assert_eq!(
            package_args_for(&tmp, &["a/src/lib.rs", "scripts/loose.rs"]),
            ["--workspace"]
        );
    }
}
