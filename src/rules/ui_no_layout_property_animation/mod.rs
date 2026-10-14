//! ui-no-layout-property-animation — flag animated layout properties.
//!
//! Checks inline `transition` and `transitionProperty` styles.
//! Checks motion `animate`, `initial`, `exit` and `while*` props.
//! Animating layout recalculates it every frame.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "ui-no-layout-property-animation",
    description: "Inline `transition` or a motion `animate`/`initial`/`exit`/`while*` prop \
                  references a layout-triggering property — animating layout causes per-frame \
                  layout recalculation.",
    remediation: "Animate `transform`, `opacity`, `color`, `background`, or `filter` instead — \
                  they don't trigger layout.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["ui"],

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
