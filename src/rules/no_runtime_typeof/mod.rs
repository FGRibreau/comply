//! no-runtime-typeof — flag every runtime `typeof` expression.
//!
//! A type-position `typeof` is never flagged.
//! `allow_in_type_guards = true` exempts `x is T` functions.
//! Opt-in.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "no-runtime-typeof",
    description: "A runtime `typeof` re-checks by hand what a parsed type guarantees.",
    remediation: "Parse the value at its boundary into a typed value, then trust the type.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["typescript"],

    skip_in_test_dir: false,
    skip_in_relaxed_dir: false,
};

pub fn register() -> RuleDef {
    RuleDef {
        meta: META,
        backends: vec![
            (
                Language::TypeScript,
                Backend::Oxc(Box::new(oxc_typescript::Check)),
            ),
            (Language::Tsx, Backend::Oxc(Box::new(oxc_typescript::Check))),
        ],
    }
}
