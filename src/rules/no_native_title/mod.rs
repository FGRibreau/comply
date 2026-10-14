//! no-native-title — flag `title` on an intrinsic HTML element.
//!
//! The native tooltip is unstyled and unreachable by keyboard.
//! `svg` and `iframe` keep it as their accessible name.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;
use crate::rules::{RuleDef, TS_FAMILY};

pub const META: RuleMeta = RuleMeta {
    id: "no-native-title",
    description: "A `title` attribute on an HTML element shows the browser's native tooltip.",
    remediation: "Use the project's tooltip component instead of the native `title` attribute.",
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
