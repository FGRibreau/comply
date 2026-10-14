//! require-explicit-time-zone — flag date formatting without a `timeZone` option.
//!
//! Covers `toLocaleDateString`, `toLocaleTimeString` and `new Intl.DateTimeFormat`.
//! `toLocaleString` counts only on a `new Date(…)` receiver.
//! Non-literal or spread options are left alone.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;
use crate::rules::{RuleDef, TS_FAMILY};

pub const META: RuleMeta = RuleMeta {
    id: "require-explicit-time-zone",
    description: "Date formatting without a `timeZone` option follows the host's time zone.",
    remediation: "Pass `{ timeZone: \"…\" }` in the options argument.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["correctness"],

    skip_in_test_dir: false,
    skip_in_relaxed_dir: false,
};

pub fn register() -> RuleDef {
    RuleDef {
        meta: META,
        backends: TS_FAMILY
            .iter()
            .map(|&lang| (lang, Backend::Oxc(Box::new(oxc_typescript::Check))))
            .collect(),
    }
}
