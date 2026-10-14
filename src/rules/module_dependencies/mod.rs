//! module-dependencies — enforce the module matrix of `comply.toml`.
//!
//! Module: first folder under `root`, or one `wildcard` subfolder.
//! Re-export findings sit on line 1. A `comply-ignore` there covers them all.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "module-dependencies",
    description: "An import between two source modules must be allowed by the project's declared module matrix.",
    remediation: "Import through an allowed module, or declare the dependency in `[rules.module-dependencies.modules]`.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["architecture"],

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
            (
                Language::JavaScript,
                Backend::Oxc(Box::new(oxc_typescript::Check)),
            ),
            (Language::Tsx, Backend::Oxc(Box::new(oxc_typescript::Check))),
        ],
    }
}
