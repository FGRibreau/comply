//! tailwind-no-raw-color-utilities — flag raw palette colours in class strings.
//!
//! Reads every class string: `className`, `cn`-style helpers, their constants.
//! Opt-in keys add arbitrary colours, white/black and `style` literals.
//! Keys and defaults: `src/config/defaults.toml`.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "tailwind-no-raw-color-utilities",
    description: "Forbid raw palette color utilities (text-gray-900, bg-blue-500) in every class string; opt-in keys add arbitrary colors (bg-[#fff]), white/black and color literals in `style` objects.",
    remediation: "Use semantic design tokens (bg-background, text-foreground, bg-primary, text-muted-foreground) so theming and dark mode stay centralized.",
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
            (Language::JavaScript, Backend::Oxc(Box::new(oxc_typescript::Check))),
            (Language::Tsx, Backend::Oxc(Box::new(oxc_typescript::Check))),
        ],
    }
}
