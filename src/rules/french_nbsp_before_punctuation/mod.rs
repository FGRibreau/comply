//! french-nbsp-before-punctuation: a non-breaking space precedes `:;?!` in French.
//! An escaped non-breaking space passes.
//! In a run of marks, only the first needs it.
//! Off unless `comply.toml` enables it.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "french-nbsp-before-punctuation",
    description: "French copy puts a non-breaking space before `:`, `;`, `?` and `!`.",
    remediation: "Write the escaped non-breaking space the message names before the mark, \
                  replacing a plain space if there is one.",
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
