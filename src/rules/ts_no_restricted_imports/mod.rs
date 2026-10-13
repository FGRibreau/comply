//! ts-no-restricted-imports — ban imports matching configured patterns.
//!
//! Keys `patterns` and `entries` are documented in `defaults.toml`.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "ts-no-restricted-imports",
    description: "Disallow imports, re-exports and literal `import()` whose specifier or resolved project path matches a configured pattern or entry.",
    remediation: "Replace the restricted import with the recommended alternative, or remove the pattern from `[rules.ts-no-restricted-imports] patterns` or `entries` in `comply.toml`.",
    severity: Severity::Error,
    doc_url: Some("https://typescript-eslint.io/rules/no-restricted-imports"),
    categories: &["typescript"],

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
