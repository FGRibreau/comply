//! Flags forbidden characters and patterns in user-facing copy.
//! `scope = "jsx-copy"` reads JSX text and copy attributes.
//! `"all-strings"` reads every string literal and template piece.

mod oxc_typescript;

use crate::diagnostic::Severity;
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::Backend;
use crate::rules::meta::RuleMeta;

pub const META: RuleMeta = RuleMeta {
    id: "no-em-dash-in-jsx-text",
    description: "Em-dash/en-dash in user-facing JSX copy reads as AI-generated prose.",
    remediation: "Replace the em-dash (\u{2014}) or en-dash (\u{2013}) with a plain hyphen, \
                  or rewrite the sentence to drop the dash.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["react"],

    skip_in_test_dir: true,
    skip_in_relaxed_dir: false,
};

/// Copy-bearing JSX attributes whose string-literal values are user-facing.
/// Anything outside this set (`className`, `id`, data-*, …) is code, not copy.
const COPY_ATTRS: &[&str] = &["title", "label", "placeholder", "alt", "aria-label"];

/// First flagged needle in `text`, with its byte offset.
/// A needle between two digits (`9–5`) is a range, skipped.
pub(crate) fn first_needle_offset<'n>(
    needles: &'n [String],
    text: &str,
) -> Option<(usize, &'n str)> {
    let bytes = text.as_bytes();
    text.char_indices().find_map(|(offset, _)| {
        needles
            .iter()
            .find(|needle| {
                text[offset..].starts_with(needle.as_str())
                    && !is_numeric_range(bytes, offset, needle.len())
            })
            .map(|needle| (offset, needle.as_str()))
    })
}

/// True when `text` trims to one needle: a placeholder glyph.
pub(crate) fn is_standalone_needle(needles: &[String], text: &str) -> bool {
    needles.iter().any(|needle| text.trim() == needle)
}

/// True when the dash at `offset` (spanning `dash_len` bytes) has an ASCII digit
/// immediately on both sides. Both neighbours are single-byte ASCII when present,
/// so the byte-level inspection never splits a multi-byte char.
fn is_numeric_range(bytes: &[u8], offset: usize, dash_len: usize) -> bool {
    let prev_is_digit = offset
        .checked_sub(1)
        .is_some_and(|i| bytes[i].is_ascii_digit());
    let next_is_digit = bytes
        .get(offset + dash_len)
        .is_some_and(u8::is_ascii_digit);
    prev_is_digit && next_is_digit
}

pub fn register() -> RuleDef {
    RuleDef {
        meta: META,
        backends: vec![
            (Language::Tsx, Backend::Oxc(Box::new(oxc_typescript::Check))),
            (
                Language::JavaScript,
                Backend::Oxc(Box::new(oxc_typescript::Check)),
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dashes() -> Vec<String> {
        vec!["\u{2014}".into(), "\u{2013}".into()]
    }

    #[test]
    fn finds_em_dash() {
        assert!(first_needle_offset(&dashes(), "Save time \u{2014} automate.").is_some());
    }

    #[test]
    fn finds_en_dash() {
        assert!(first_needle_offset(&dashes(), "First \u{2013} last").is_some());
    }

    #[test]
    fn ignores_plain_hyphen() {
        assert!(first_needle_offset(&dashes(), "Save time - automate.").is_none());
    }

    #[test]
    fn skips_numeric_range_en_dash() {
        assert!(first_needle_offset(&dashes(), "9\u{2013}5").is_none());
        assert!(first_needle_offset(&dashes(), "2020\u{2013}2024").is_none());
    }

    #[test]
    fn skips_numeric_range_em_dash() {
        assert!(first_needle_offset(&dashes(), "9\u{2014}5").is_none());
    }

    #[test]
    fn flags_dash_when_only_one_side_is_a_digit() {
        // "page 9 — see note" is prose, not a range.
        assert!(first_needle_offset(&dashes(), "9 \u{2014} note").is_some());
        assert!(first_needle_offset(&dashes(), "note \u{2014} 9").is_some());
    }

    #[test]
    fn offset_lands_on_the_dash() {
        let text = "ab\u{2014}cd";
        let (off, _) = first_needle_offset(&dashes(), text).unwrap();
        assert_eq!(&text[off..off + 3], "\u{2014}");
    }

    #[test]
    fn standalone_dash_is_a_placeholder() {
        assert!(is_standalone_needle(&dashes(), "\u{2014}"));
        assert!(is_standalone_needle(&dashes(), "\u{2013}"));
        // JSXText carries surrounding whitespace from formatting.
        assert!(is_standalone_needle(&dashes(), "  \u{2014}  "));
        assert!(is_standalone_needle(&dashes(), "\n  \u{2013}\n"));
    }

    #[test]
    fn dash_with_prose_is_not_standalone() {
        assert!(!is_standalone_needle(&dashes(), "\u{2014} Foo"));
        assert!(!is_standalone_needle(&dashes(), "Foo \u{2014} bar"));
        assert!(!is_standalone_needle(&dashes(), "\u{2014}\u{2014}"));
        assert!(!is_standalone_needle(&dashes(), "-"));
        assert!(!is_standalone_needle(&dashes(), ""));
    }
}
