//! no-reflect-apply — flag `Reflect.apply(...)`.
//!
//! A direct call or `f.apply(t, a)` reads plainer.
//! Opt-in: it contradicts `prefer-reflect-apply`.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;
use crate::rules::{RuleDef, TS_FAMILY};

pub const META: RuleMeta = RuleMeta {
    id: "no-reflect-apply",
    description: "`Reflect.apply` is an indirection over a direct call or `Function.prototype.apply`.",
    remediation: "Call the function directly, or use `f.apply(thisArg, args)`.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["code-quality"],

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
