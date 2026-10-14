//! no-conditional-empty-object-spread — flag a conditional object spread.
//!
//! Flags `...(c ? { a } : {})` and `...(c && { a })`.
//! The empty branch only exists to satisfy the spread.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;
use crate::rules::{RuleDef, TS_FAMILY};

pub const META: RuleMeta = RuleMeta {
    id: "no-conditional-empty-object-spread",
    description: "A conditional spread with an empty-object branch hides an optional key behind a trick.",
    remediation: "Set the key explicitly (`a: c ? value : undefined`) or assign it after building the object.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["code-quality"],

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
