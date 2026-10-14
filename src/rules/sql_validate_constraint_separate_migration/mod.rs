//! Flags validating a constraint the same migration adds `NOT VALID`.
//! Migrations only, skipped in test dirs.

mod oxc_typescript;
mod rust;
mod sql;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;
use crate::rules::sql_helpers::{find_word, is_ident_byte};

pub const META: RuleMeta = RuleMeta {
    id: "sql-validate-constraint-separate-migration",
    description: "A constraint added `NOT VALID` must not be validated in the same migration.",
    remediation: "Move `ALTER TABLE t VALIDATE CONSTRAINT t_a_chk` to a later migration. In one transaction, the VALIDATE holds the lock that `NOT VALID` was meant to avoid.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["database", "sql", "migrations"],

    skip_in_test_dir: true,
    skip_in_relaxed_dir: false,
};

const MESSAGE: &str = "`VALIDATE CONSTRAINT` of a constraint this migration adds `NOT VALID` runs in the same transaction — move it to a later migration.";

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
            (Language::Sql, Backend::Text(Box::new(sql::Check))),
        ],
    }
}

/// Where constraints added `NOT VALID` here are validated.
pub(super) fn validated_in_same_text(sql: &str) -> Vec<usize> {
    let lower = sql.to_ascii_lowercase();
    let added_not_valid: Vec<&str> = constraint_clauses(&lower, "add")
        .filter(|&(_, _, name_end)| clause_is_not_valid(&lower, name_end))
        .map(|(_, name, _)| name)
        .collect();
    constraint_clauses(&lower, "validate")
        .filter(|(_, name, _)| added_not_valid.contains(name))
        .map(|(start, _, _)| start)
        .collect()
}

/// Verb start, name and name end of each `<verb> CONSTRAINT <name>`.
fn constraint_clauses<'a>(
    lower: &'a str,
    verb: &'a str,
) -> impl Iterator<Item = (usize, &'a str, usize)> + 'a {
    let mut from = 0;
    std::iter::from_fn(move || {
        loop {
            let (start, verb_end) = find_word(lower, verb, from)?;
            from = verb_end;
            if let Some(("constraint", keyword_end)) = next_token(lower, verb_end)
                && let Some((name, name_end)) = next_token(lower, keyword_end)
            {
                return Some((start, name.trim_matches('"'), name_end));
            }
        }
    })
}

/// Next identifier or quoted name after `from`, with its end.
fn next_token(lower: &str, from: usize) -> Option<(&str, usize)> {
    let rest = lower.get(from..)?;
    let trimmed = rest.trim_start();
    let start = from.checked_add(rest.len().checked_sub(trimmed.len())?)?;
    let token_len = if let Some(quoted) = trimmed.strip_prefix('"') {
        quoted.find('"')?.checked_add(2)?
    } else {
        trimmed.bytes().take_while(|&b| is_ident_byte(b)).count()
    };
    let end = start.checked_add(token_len)?;
    let token = lower.get(start..end).filter(|t| !t.is_empty())?;
    Some((token, end))
}

/// Whether this `ADD` clause, up to `;`, says `NOT VALID`.
fn clause_is_not_valid(lower: &str, name_end: usize) -> bool {
    let Some(rest) = lower.get(name_end..) else {
        return false;
    };
    let clause_end = [find_word(rest, "add", 0).map(|(at, _)| at), rest.find(';')]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(rest.len());
    let words: Vec<&str> = rest
        .get(..clause_end)
        .unwrap_or("")
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .collect();
    words.windows(2).any(|pair| pair == ["not", "valid"])
}
