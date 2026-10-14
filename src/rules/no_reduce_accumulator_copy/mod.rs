//! no-reduce-accumulator-copy — flag a `reduce` callback copying its accumulator.
//!
//! Each copy walks the whole accumulator: the reduce turns quadratic.
//! The accumulator is resolved by binding, not by name.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;
use crate::rules::{RuleDef, TS_FAMILY};

pub const META: RuleMeta = RuleMeta {
    id: "no-reduce-accumulator-copy",
    description: "Copying the accumulator inside a `reduce` callback makes the reduce quadratic.",
    remediation: "Mutate the accumulator in place (`acc.push(x); return acc;`) or use a `for` loop.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["performance"],

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
