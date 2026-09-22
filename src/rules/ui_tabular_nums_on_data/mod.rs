//! ui-tabular-nums-on-data — JSX elements displaying numeric data
//! (counters, prices, metrics) should use `tabular-nums`.

mod css;
mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "ui-tabular-nums-on-data",
    description: "Elements rendering numeric data (counters, prices, metrics) should use `tabular-nums` so digits don't shift width between ticks.",
    remediation: "Add `font-variant-numeric: tabular-nums` (or Tailwind `tabular-nums`) to the element's className.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["ui"],

    skip_in_test_dir: true,
    skip_in_relaxed_dir: false,
};

/// Words hinting at numeric data, singular form.
const DATA_HINTS: &[&str] = &["counter", "count", "price", "amount", "metric", "stat", "number"];

/// True when `text` (a className or a CSS selector) contains a data hint as a
/// whole word (plural allowed), or the `value-display` pair. Words split on any
/// non-alphanumeric character, so `stat-card` and `.prices` match while
/// `group-data-[state=collapsed]:hidden` does not.
fn has_data_hint(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let is_hint = |word: &str| {
        let singular = word.strip_suffix('s').unwrap_or(word);
        DATA_HINTS.contains(&word) || DATA_HINTS.contains(&singular)
    };
    words.iter().any(|word| is_hint(word))
        || words.windows(2).any(|pair| pair == ["value", "display"])
}

pub fn register() -> RuleDef {
    RuleDef {
        meta: META,
        backends: vec![
            (Language::TypeScript, Backend::Oxc(Box::new(oxc_typescript::Check))),
            (Language::JavaScript, Backend::Oxc(Box::new(oxc_typescript::Check))),
            (Language::Tsx, Backend::Oxc(Box::new(oxc_typescript::Check))),
            (Language::Css, Backend::TreeSitter(Box::new(css::Check))),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::has_data_hint;

    #[test]
    fn matches_hints_on_word_boundaries() {
        for text in ["stat-card", "text-2xl stats", ".price", "order-count", "metrics", "value-display"] {
            assert!(has_data_hint(text), "{text}");
        }
    }

    #[test]
    fn ignores_hints_inside_other_words() {
        // natalia v3 `team-switcher.tsx`: `state` in a data-attribute variant
        // is not the `stat` hint.
        assert!(!has_data_hint(
            "text-muted-foreground ms-auto size-4 shrink-0 group-data-[state=collapsed]:hidden"
        ));
        assert!(!has_data_hint("status-badge"));
    }
}
