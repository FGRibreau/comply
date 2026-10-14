//! testing-no-mocking-internal-modules — flag mocks of relative internal paths.
//!
//! Covers `mock`, `doMock` and `unstable_mockModule` on `vi`/`jest`.
//! Tests should mock boundaries, not internals.
//! With `include_packages`, package mocks are flagged too.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "testing-no-mocking-internal-modules",
    description: "Mocking a relative internal module (or any package, under `include_packages`) \
                  couples tests to implementation details.",
    remediation: "Mock only external boundaries (HTTP, DB, third-party SDKs). Refactor so the collaborator is injected, or rely on the real internal module.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["testing"],

    skip_in_test_dir: false,
    skip_in_relaxed_dir: false,
};

pub fn register() -> RuleDef {
    RuleDef {
        meta: META,
        backends: vec![
            (Language::TypeScript, Backend::Oxc(Box::new(oxc_typescript::Check))),
            (Language::JavaScript, Backend::Oxc(Box::new(oxc_typescript::Check))),
            (Language::Tsx, Backend::Oxc(Box::new(oxc_typescript::Check))),
        ],
    }
}
