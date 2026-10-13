//! jsx-restricted-elements — flag JSX elements a project policy restricts.
//!
//! Each entry selects elements, narrows them, applies one action.
//! Keys and defaults: `src/config/defaults.toml`.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "jsx-restricted-elements",
    description: "User-configured policy over JSX elements: a selector (`tag` regex or `element` name, narrowed by attribute, class and text conditions) and one action (`use` a replacement, `forbid_attr`, `require_attr`, `forbid_class`, `forbid_nested`), each with its own message.",
    remediation: "Use the element, attribute or class the project policy names in the message.",
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
