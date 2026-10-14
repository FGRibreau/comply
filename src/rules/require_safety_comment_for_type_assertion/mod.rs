//! require-safety-comment-for-type-assertion — flag an unexplained `as T`.
//!
//! The `marker` comment goes on its line or above.
//! The default marker is `SAFETY:`.
//! `as const` is exempt.
//! Opt-in.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "require-safety-comment-for-type-assertion",
    description: "A type assertion carries no comment saying why it holds.",
    remediation: "Write the marker comment (default `// SAFETY:`) with the reason above the assertion.",
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
