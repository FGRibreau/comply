//! todo-needs-issue-link text backend — scan comments for TODO/FIXME/HACK
//! markers that lack an issue reference (#1234, JIRA-456, or URL).

use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::backend::{CheckCtx, TextCheck};

#[derive(Debug)]
pub struct Check;

const MARKERS: [&str; 3] = ["TODO", "FIXME", "HACK"];

/// Find a TODO/FIXME/HACK marker inside a comment on `line`.
/// Returns `(marker, rest_of_line_after_marker)` if found.
fn find_marker_in_comment(line: &str) -> Option<(&'static str, &str)> {
    let comment_start = line.find("//").or_else(|| line.find("/*")).or_else(|| {
        // Handle continuation lines inside block comments (lines starting
        // with optional whitespace then `*`).  We only match `*` that is NOT
        // followed by `/` (that would be the closing `*/`).
        let trimmed = line.trim_start();
        if trimmed.starts_with('*') && !trimmed.starts_with("*/") {
            Some(line.len() - trimmed.len())
        } else {
            None
        }
    })?;
    let after = &line[comment_start..];
    for marker in &MARKERS {
        if let Some(pos) = after.find(marker) {
            return Some((marker, &after[pos + marker.len()..]));
        }
    }
    None
}

/// Check whether `rest` (text after the marker) contains an issue reference.
fn has_issue_ref(rest: &str, allow_url: bool) -> bool {
    if allow_url && ["http://", "https://"].iter().any(|s| rest.contains(s)) {
        return true;
    }
    // A key inside a refused URL is not a reference.
    rest.split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .filter(|word| !word.starts_with("http"))
        .any(|word| {
            let bytes = word.as_bytes();
            bytes
                .windows(2)
                .any(|w| matches!(w, [b'#', d] if d.is_ascii_digit()))
                || bytes.windows(3).any(
                    |w| matches!(w, [k, b'-', d] if k.is_ascii_uppercase() && d.is_ascii_digit()),
                )
        })
}

/// Is `line` a comment line that continues the marker's comment block?
///
/// Matches `//`, `/*`, or a block-comment continuation (`*` not followed by
/// `/`). Leading whitespace is ignored so indented blocks still match.
fn is_comment_continuation(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("//")
        || trimmed.starts_with("/*")
        || (trimmed.starts_with('*') && !trimmed.starts_with("*/"))
}

/// Scan the comment lines immediately following the marker's line for an issue
/// reference. The marker is satisfied when its own contiguous comment block
/// documents the reference on a following line:
///
/// ```text
/// // FIXME: temporary fix for a libc bug
/// // https://github.com/rust-lang/libc/pull/704
/// ```
///
/// The scan starts at `lines[start]` (the line after the marker) and walks
/// forward while each line is a comment continuation. It stops — without
/// inspecting that line — at the first line that is not a comment, is
/// blank/whitespace-only (a gap breaks the block), or carries its own marker
/// (that block belongs to the next TODO, not this one).
fn block_has_ref(lines: &[&str], start: usize, allow_url: bool) -> bool {
    for &line in &lines[start..] {
        if !is_comment_continuation(line) {
            return false;
        }
        if find_marker_in_comment(line).is_some() {
            return false;
        }
        if has_issue_ref(line, allow_url) {
            return true;
        }
    }
    false
}

impl TextCheck for Check {
    fn check(&self, ctx: &CheckCtx) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let lines: Vec<&str> = ctx.source.lines().collect();
        let allow_url = ctx.config.bool_flag(super::META.id, "allow_url", ctx.lang);

        for (idx, line) in lines.iter().enumerate() {
            let Some((marker, rest)) = find_marker_in_comment(line) else {
                continue;
            };

            if has_issue_ref(rest, allow_url) {
                continue;
            }

            if block_has_ref(&lines, idx.saturating_add(1), allow_url) {
                continue;
            }

            let message = if allow_url {
                format!(
                    "{marker} comment is missing an issue reference — \
                     add a ticket number or URL (e.g. `{marker}(#1234)` or `{marker}(https://...)`)."
                )
            } else {
                format!("{marker} comment is missing an issue number: write `{marker}(#1234)`.")
            };

            diagnostics.push(Diagnostic {
                path: std::sync::Arc::clone(&ctx.path_arc),
                line: idx + 1,
                column: 1,
                rule_id: "todo-needs-issue-link".into(),
                message,
                severity: Severity::Error,
                span: None,
            });
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn run(source: &str) -> Vec<Diagnostic> {
        Check.check(&CheckCtx::for_test(Path::new("t.ts"), source))
    }

    // --- Should pass (no diagnostic) ---

    #[test]
    fn todo_with_github_issue() {
        assert!(run("// TODO(#1234): migrate to v2").is_empty());
    }

    #[test]
    fn todo_with_github_issue_no_parens() {
        assert!(run("// TODO #1234 migrate to v2").is_empty());
    }

    #[test]
    fn todo_with_jira_key() {
        assert!(run("// TODO(JIRA-456): migrate to v2").is_empty());
    }

    #[test]
    fn todo_with_jira_key_no_parens() {
        assert!(run("// TODO PROJ-789 migrate to v2").is_empty());
    }

    #[test]
    fn todo_with_url() {
        assert!(run("// TODO(https://github.com/org/repo/issues/42): fix this").is_empty());
    }

    #[test]
    fn todo_with_http_url() {
        assert!(run("// TODO http://tracker.example.com/123 fix this").is_empty());
    }

    #[test]
    fn fixme_with_issue() {
        assert!(run("// FIXME(#99): broken on edge case").is_empty());
    }

    #[test]
    fn hack_with_issue() {
        assert!(run("// HACK(#5): workaround for upstream bug").is_empty());
    }

    #[test]
    fn code_variable_not_flagged() {
        assert!(run("let todo = 5;").is_empty());
    }

    #[test]
    fn code_string_not_flagged() {
        assert!(run(r#"const msg = "TODO: implement";"#).is_empty());
    }

    // --- Should flag (diagnostic emitted) ---

    #[test]
    fn todo_without_ref() {
        let diags = run("// TODO: fix this later");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("TODO"));
    }

    #[test]
    fn fixme_without_ref() {
        let diags = run("// FIXME: broken on edge case");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("FIXME"));
    }

    #[test]
    fn hack_without_ref() {
        let diags = run("// HACK: workaround");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("HACK"));
    }

    #[test]
    fn todo_in_block_comment() {
        let diags = run("/* TODO: fix this */");
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn todo_in_block_comment_with_ref() {
        assert!(run("/* TODO(#42): fix this */").is_empty());
    }

    #[test]
    fn multiple_todos() {
        let src = "// TODO: first\n// TODO(#1): second\n// FIXME: third\n";
        let diags = run(src);
        assert_eq!(diags.len(), 2); // first and third flagged
    }

    #[test]
    fn line_numbers_correct() {
        let src = "let x = 1;\n// TODO: fix\nlet y = 2;\n";
        let diags = run(src);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 2);
    }

    #[test]
    fn ref_on_following_comment_line() {
        // The issue shape: marker on one line, URL on the next comment line.
        let src = "// FIXME: temporary fix for a libc bug\n\
                   // https://github.com/rust-lang/libc/pull/704\n";
        assert!(run(src).is_empty());
    }

    #[test]
    fn ref_on_following_comment_line_issue_number() {
        let src = "// TODO: migrate to v2\n// see #1234 for context\n";
        assert!(run(src).is_empty());
    }

    #[test]
    fn ref_two_comment_lines_below() {
        // The reference may sit further down the contiguous block.
        let src = "// FIXME: this is a long explanation\n\
                   // that wraps across several comment lines\n\
                   // https://github.com/org/repo/issues/9\n";
        assert!(run(src).is_empty());
    }

    #[test]
    fn no_ref_in_block_still_flagged() {
        // A multi-line comment block with no reference anywhere is flagged.
        let src = "// FIXME: this is broken\n// and we should fix it someday\n";
        let diags = run(src);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 1);
    }

    #[test]
    fn blank_gap_breaks_the_block() {
        // A blank line separates the marker from the later comment with the
        // URL — they are not the same block, so the marker is still flagged.
        let src = "// TODO: fix this\n\n// https://github.com/org/repo/issues/1\n";
        let diags = run(src);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 1);
    }

    #[test]
    fn non_comment_line_breaks_the_block() {
        // Code follows the marker before any reference — still flagged.
        let src = "// TODO: fix this\nlet x = 1; // https://example.com/1\n";
        let diags = run(src);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 1);
    }

    #[test]
    fn next_marker_does_not_satisfy_previous() {
        // The second marker carries the URL, but it is its own TODO; the first
        // marker's block stops at it and the first is still flagged.
        let src = "// TODO: first thing\n// TODO: second thing #42\n";
        let diags = run(src);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 1);
    }

    #[test]
    fn no_panic_on_multibyte_chars() {
        // Emoji before the URL pattern — used to panic on char boundary
        assert!(run("// TODO 💡 https://github.com/org/repo/issues/1").is_empty());
        let diags = run("// TODO 💡💡💡 fix this later");
        assert_eq!(diags.len(), 1);
    }

    fn run_without_url(source: &str) -> Vec<Diagnostic> {
        let project_dir = tempfile::TempDir::new().expect("tempdir");
        std::fs::write(
            project_dir.path().join("comply.toml"),
            "[rules.todo-needs-issue-link]\nallow_url = false\n",
        )
        .expect("write cfg");
        let config = crate::config::Config::load_from(project_dir.path()).expect("load cfg");
        let ctx = CheckCtx {
            config: &config,
            ..CheckCtx::for_test(Path::new("t.ts"), source)
        };
        Check.check(&ctx)
    }

    #[test]
    fn url_disallowed_flags_url_and_names_hash_form() {
        let diags = run_without_url(
            "// TODO(https://x/1)\n// TODO: x\n// https://x/1\n// TODO(https://jira.x/browse/ABC-12)\n",
        );
        assert_eq!(diags.len(), 3);
        assert!(diags[0].message.contains("TODO(#1234)"));
        assert!(!diags[0].message.contains("https"));
    }

    #[test]
    fn url_disallowed_accepts_issue_number() {
        assert!(run_without_url("// TODO(#12)").is_empty());
    }
}
