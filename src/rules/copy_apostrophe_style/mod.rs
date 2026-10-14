//! copy-apostrophe-style: a copy apostrophe differing from `style`.
//! `typographic` flags `'`; `straight` flags `’`.
//! Elision only: after a letter, before a letter or end.
//! Off unless `comply.toml` enables it.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "copy-apostrophe-style",
    description: "An apostrophe in UI copy that differs from the configured style.",
    remediation: "Write the apostrophe the message names, or change `style` in comply.toml.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["react"],

    skip_in_test_dir: true,
    skip_in_relaxed_dir: false,
};

pub fn register() -> RuleDef {
    RuleDef {
        meta: META,
        backends: vec![
            (Language::Tsx, Backend::Oxc(Box::new(oxc_typescript::Check))),
            (
                Language::JavaScript,
                Backend::Oxc(Box::new(oxc_typescript::Check)),
            ),
        ],
    }
}
