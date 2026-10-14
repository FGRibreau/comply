//! rust-no-parse-unwrap-or — a `parse()` failure silently replaced by a fallback.

mod rust;

use crate::diagnostic::Severity;
use crate::rules::RuleDef;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "rust-no-parse-unwrap-or",
    description: "`.parse()` chained into `unwrap_or*` turns a corrupt value into a silent default.",
    remediation: "Propagate the parse error with `?` or a typed error. Keep a \
                  deliberate fallback with \
                  `// comply-ignore: rust-no-parse-unwrap-or — <reason>`.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["rust"],

    skip_in_test_dir: false,
    skip_in_relaxed_dir: true,
};

pub fn register() -> RuleDef {
    crate::register_rust_only!(META, rust)
}
