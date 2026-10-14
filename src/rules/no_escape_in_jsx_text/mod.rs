//! no-escape-in-jsx-text — flag a `\uXXXX` or `\xXX` escape in JSX text.
//!
//! JSX text and quoted attributes render escapes literally.
//! An expression container interprets it: that form passes.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;
use crate::rules::{RuleDef, TS_FAMILY};

pub const META: RuleMeta = RuleMeta {
    id: "no-escape-in-jsx-text",
    description: "A JavaScript escape in JSX text or a quoted JSX attribute renders literally.",
    remediation: "Write the character itself, or wrap the string in an expression container: `{\"\\u00A0\"}`.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["react"],

    skip_in_test_dir: false,
    skip_in_relaxed_dir: false,
};

pub fn register() -> RuleDef {
    RuleDef {
        meta: META,
        backends: TS_FAMILY
            .iter()
            .map(|&lang| (lang, Backend::Oxc(Box::new(oxc_typescript::Check))))
            .collect(),
    }
}
