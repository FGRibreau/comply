//! tailwind-no-restricted-classes — flag classes a project policy bans.
//!
//! Reads every class string: `className`, `cn`-style helpers, their constants.
//! `classes` bans exact utilities; `entries` hold regex policies.
//! Keys and defaults: `src/config/defaults.toml`.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "tailwind-no-restricted-classes",
    description: "User-configured policy over Tailwind classes in every class string (`className`, `cn`, `cva`, `clsx`, constants): an exact blocklist plus regex entries with allow-lists, variant and context conditions, each with its own message.",
    remediation: "Use the project-approved equivalent. If the class is needed for a one-off, escape via the project's design-token override mechanism.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["tailwind"],

    skip_in_test_dir: true,
    skip_in_relaxed_dir: false,
};

pub fn register() -> RuleDef {
    RuleDef {
        meta: META,
        backends: vec![
            (Language::TypeScript, Backend::Oxc(Box::new(oxc_typescript::Check))),
            (Language::Tsx, Backend::Oxc(Box::new(oxc_typescript::Check))),
            (Language::JavaScript, Backend::Oxc(Box::new(oxc_typescript::Check))),
        ],
    }
}
