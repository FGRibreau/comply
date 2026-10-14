//! banned-copy-words: a word the project bans from UI copy.
//! A word matches whole and case-insensitively.
//! A phrase of `allowed` exempts the words it holds.
//! Off unless `comply.toml` enables it.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "banned-copy-words",
    description: "A word the project bans from its UI copy.",
    remediation: "Rewrite the copy without the word, or add the phrase to `allowed` in comply.toml.",
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
