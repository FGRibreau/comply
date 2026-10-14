//! sql-update-replace-needs-row-check — SQL text backend for .sql migrations.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{CheckCtx, TextCheck};
use crate::rules::sql_helpers::{
    find_word, is_ident_byte, is_migration_path, strip_leading_word, word_followed_by_open_paren,
};

#[derive(Debug)]
pub struct Check;

impl TextCheck for Check {
    fn check(&self, ctx: &CheckCtx) -> Vec<Diagnostic> {
        if !is_migration_path(ctx.path) {
            return vec![];
        }
        let lower = ctx.source.to_ascii_lowercase();
        let mut diagnostics = vec![];
        let mut from = 0;
        while let Some((start, end)) = find_word(&lower, "update", from) {
            from = end;
            if !sets_with_replace(&lower, end) || has_row_check(&lower, start) {
                continue;
            }
            let (line, column) = byte_offset_to_line_col(ctx.source, start);
            diagnostics.push(Diagnostic {
                path: std::sync::Arc::clone(&ctx.path_arc),
                line,
                column,
                rule_id: super::META.id.into(),
                message: "`UPDATE … replace(` without a row check — count rows `INTO expected` before, then `GET DIAGNOSTICS n = ROW_COUNT` and raise when `n <> expected`.".into(),
                severity: Severity::Error,
                span: None,
            });
        }
        diagnostics
    }
}

/// Whether this `UPDATE` statement's `SET` calls `replace(`.
fn sets_with_replace(lower: &str, update_end: usize) -> bool {
    let statement = lower
        .get(update_end..)
        .and_then(|rest| rest.split(';').next())
        .unwrap_or("");
    find_word(statement, "set", 0)
        .and_then(|(_, set_end)| statement.get(set_end..))
        .is_some_and(|assignments| word_followed_by_open_paren(assignments, "replace"))
}

/// Whether a later `ROW_COUNT` matches a prior `INTO` count.
// ponytail: one later check covers every earlier UPDATE.
fn has_row_check(lower: &str, update_start: usize) -> bool {
    let (Some(before), Some(after)) = (lower.get(..update_start), lower.get(update_start..)) else {
        return false;
    };
    let Some(rest) = find_word(after, "diagnostics", 0).and_then(|(_, end)| after.get(end..))
    else {
        return false;
    };
    let Some((target, value)) = rest.split(';').next().and_then(|item| item.split_once('=')) else {
        return false;
    };
    if value.trim() != "row_count" {
        return false;
    }
    let row_count = target.trim().trim_end_matches(':').trim();
    raised_comparisons(rest, row_count).any(|expected| assigned_by_into(before, expected))
}

/// Operands compared to `var` by `<>`, `!=`, `IS DISTINCT FROM`, then raised.
fn raised_comparisons<'a>(text: &'a str, var: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    ["<>", "!=", "is distinct from"]
        .into_iter()
        .flat_map(move |op| {
            text.match_indices(op).filter_map(move |(at, _)| {
                let after = text.get(at.checked_add(op.len())?..)?;
                let left = trailing_ident(text.get(..at)?);
                let right = leading_ident(after);
                let other = if left == var {
                    right
                } else if right == var {
                    left
                } else {
                    return None;
                };
                (!other.is_empty() && raises_exception(after)).then_some(other)
            })
        })
}

/// Identifier at the start of `text`, after whitespace.
fn leading_ident(text: &str) -> &str {
    let text = text.trim_start();
    let len = text.bytes().take_while(|&b| is_ident_byte(b)).count();
    text.get(..len).unwrap_or("")
}

/// Identifier at the end of `text`, before whitespace.
fn trailing_ident(text: &str) -> &str {
    let text = text.trim_end();
    let len = text.bytes().rev().take_while(|&b| is_ident_byte(b)).count();
    text.len()
        .checked_sub(len)
        .and_then(|start| text.get(start..))
        .unwrap_or("")
}

/// Whether the statement starting in `text` says `RAISE EXCEPTION`.
fn raises_exception(text: &str) -> bool {
    let statement = text.split(';').next().unwrap_or("");
    find_word(statement, "raise", 0)
        .and_then(|(_, end)| statement.get(end..))
        .and_then(|rest| strip_leading_word(rest.trim_start(), "exception"))
        .is_some()
}

/// Whether `text` holds an `INTO [STRICT] name`.
fn assigned_by_into(text: &str, name: &str) -> bool {
    let mut from = 0;
    while let Some((_, into_end)) = find_word(text, "into", from) {
        from = into_end;
        let next = text.get(into_end..).unwrap_or("").trim_start();
        let next = strip_leading_word(next, "strict").map_or(next, str::trim_start);
        if find_word(next, name, 0).is_some_and(|(at, _)| at == 0) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn run(src: &str) -> Vec<Diagnostic> {
        Check.check(&CheckCtx::for_test(
            Path::new("/app/migrations/003.sql"),
            src,
        ))
    }

    fn checked_update(check: &str) -> String {
        format!(
            "DO $$\nDECLARE expected int; n int;\nBEGIN\n\
             SELECT count(*) INTO expected FROM t WHERE strpos(a, 'x') > 0;\n\
             UPDATE t SET a = replace(a, 'x', 'y') WHERE strpos(a, 'x') > 0;\n\
             GET DIAGNOSTICS n = ROW_COUNT;\n\
             IF {check} THEN RAISE EXCEPTION 'row mismatch'; END IF;\n\
             END $$;\n"
        )
    }

    #[test]
    fn flags_update_replace_without_row_check() {
        let diagnostics =
            run("SET lock_timeout = '5s';\nUPDATE t SET a = replace(a, 'x', 'y') WHERE id = 1;\n");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 2);
        assert!(diagnostics[0].message.contains("GET DIAGNOSTICS"));
        assert!(run("UPDATE t SET a = regexp_replace(a, 'x', 'y');").is_empty());
    }

    #[test]
    fn allows_update_replace_with_row_count_compared_to_prior_count() {
        assert!(run(&checked_update("n <> expected")).is_empty());
        assert!(run(&checked_update("expected <> n")).is_empty());
        let strict =
            checked_update("n <> expected").replace("INTO expected", "INTO STRICT expected");
        assert!(run(&strict).is_empty());
    }

    #[test]
    fn flags_zero_only_row_checks() {
        let no_prior_count = checked_update("n <> expected").replace("INTO expected", "");
        let variants = [
            checked_update("n = 0"),
            checked_update("n < 1"),
            checked_update("NOT FOUND"),
            checked_update("n <> 1"),
            no_prior_count,
            checked_update("n <> expected").replace("EXCEPTION", "NOTICE"),
        ];
        for src in variants {
            assert_eq!(run(&src).len(), 1, "{src}");
        }
    }
}
