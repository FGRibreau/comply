//! no-restricted-object-keys: configured keys under named contexts.
//! Reached: the context value, or its call's object arguments.
//! Objects nested one level in those are reached too.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "no-restricted-object-keys",
    description: "User-configured keys forbidden in an object literal passed under a named property or JSX prop.",
    remediation: "Follow the entry's message: use the project-approved token or move the value out of this object.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["code-quality"],

    skip_in_test_dir: true,
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
            (
                Language::JavaScript,
                Backend::Oxc(Box::new(oxc_typescript::Check)),
            ),
        ],
    }
}
