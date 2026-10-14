//! nginx-add-header-always — nginx sends `add_header` headers on 2xx/3xx only.
//! Without `always`, error pages lose their security headers.

mod text;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "nginx-add-header-always",
    description: "nginx `add_header` directive without the `always` flag.",
    remediation: "End the directive with `always`, e.g. \
                  `add_header X-Frame-Options DENY always;`. Without it, nginx omits the \
                  header on 4xx and 5xx responses.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["security"],

    skip_in_test_dir: false,
    skip_in_relaxed_dir: true,
};

pub fn register() -> RuleDef {
    RuleDef {
        meta: META,
        backends: vec![(Language::Nginx, Backend::Text(Box::new(text::Check)))],
    }
}
