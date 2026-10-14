//! sql-update-replace-needs-row-check
//!
//! Flags a migration `UPDATE … replace(` without a row-count check.
//! Skipped in test dirs.

mod sql;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "sql-update-replace-needs-row-check",
    description: "A migration `UPDATE … SET col = replace(col, …)` must check how many rows it changed.",
    remediation: "Count the target rows `INTO expected` before the UPDATE, then `GET DIAGNOSTICS n = ROW_COUNT;` and `IF n <> expected THEN RAISE EXCEPTION …`. A check on zero rows alone fails on an empty database.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["database", "sql", "migrations"],

    skip_in_test_dir: true,
    skip_in_relaxed_dir: false,
};

pub fn register() -> RuleDef {
    RuleDef {
        meta: META,
        backends: vec![(Language::Sql, Backend::Text(Box::new(sql::Check)))],
    }
}
