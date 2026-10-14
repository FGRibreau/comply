//! tailwind-no-transition-all-layout — flag transitions that can animate layout.
//!
//! Flags `transition-all`, or `transition-[…]` naming a layout property.
//! `hint` is appended to the message; defaults: `src/config/defaults.toml`.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "tailwind-no-transition-all-layout",
    description: "Forbid `transition-all` and `transition-[…]` on a layout property (width, height, inset, margin, padding) in every class string — causes layout thrash.",
    remediation: "Animate via `translate-*` + `transition-transform` or `opacity-*` + `transition-opacity`. These are composited and never trigger layout.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["tailwind", "web-performance"],

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
