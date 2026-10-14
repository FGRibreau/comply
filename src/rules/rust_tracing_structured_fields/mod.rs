//! rust-tracing-structured-fields — a `tracing` event message interpolates values.

mod rust;

use crate::diagnostic::Severity;
use crate::rules::RuleDef;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "rust-tracing-structured-fields",
    description: "`tracing` event messages must not interpolate values; use structured fields.",
    remediation: "Replace `warn!(\"fetch {url} failed: {e}\")` with \
                  `warn!(%url, error = %e, \"fetch failed\")`. Fields stay \
                  queryable; an interpolated message is opaque text.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["rust", "observability"],

    skip_in_test_dir: false,
    skip_in_relaxed_dir: true,
};

pub fn register() -> RuleDef {
    crate::register_rust_only!(META, rust)
}
