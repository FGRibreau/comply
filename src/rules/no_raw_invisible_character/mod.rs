//! no-raw-invisible-character: a raw invisible character hides from review.
//! It reads TS, JS and Rust strings, and JSX text.
//! Its set is `regex-no-invisible-character`'s, minus variation selectors.

mod oxc_typescript;
mod rust;

use crate::diagnostic::{Diagnostic, Severity};
use crate::files::Language;
use crate::rules::RuleDef;
use crate::rules::backend::{Backend, CheckCtx};
use crate::rules::meta::RuleMeta;
use crate::rules::regex_no_invisible_character::is_invisible_char;
use crate::rules::rendered_copy::{Escapes, escape};
use std::sync::Arc;

pub const META: RuleMeta = RuleMeta {
    id: "no-raw-invisible-character",
    description: "An invisible character written raw in a string cannot be seen in review.",
    remediation: "Write the character as the escape the message names.",
    severity: Severity::Error,
    doc_url: None,
    categories: &["code-quality"],

    skip_in_test_dir: false,
    skip_in_relaxed_dir: false,
};

pub fn register() -> RuleDef {
    RuleDef {
        meta: META,
        backends: vec![
            (
                Language::TypeScript,
                Backend::Oxc(Box::new(oxc_typescript::Check)),
            ),
            (
                Language::JavaScript,
                Backend::Oxc(Box::new(oxc_typescript::Check)),
            ),
            (Language::Tsx, Backend::Oxc(Box::new(oxc_typescript::Check))),
            (Language::Rust, Backend::TreeSitter(Box::new(rust::Check))),
        ],
    }
}

/// Reports each raw invisible of `text`, which starts at `start`.
/// `in_raw_literal`: escapes do not work there, the text moves.
fn check_text(
    ctx: &CheckCtx,
    text: &str,
    start: usize,
    escapes: Escapes,
    in_raw_literal: bool,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let raw = text
        .char_indices()
        .filter(|&(_, c)| is_invisible_char(c) && !matches!(c, '\u{FE00}'..='\u{FE0F}'));
    for (offset, c) in raw {
        let escaped = escape(c, escapes);
        let remedy = if in_raw_literal {
            format!("move the text to a plain literal, where `{escaped}` decodes")
        } else {
            format!("write `{escaped}` instead")
        };
        diagnostics.push(Diagnostic::at_offset(
            Arc::clone(&ctx.path_arc),
            ctx.source,
            (start.saturating_add(offset), c.len_utf8()),
            META.id,
            format!(
                "Invisible character U+{:04X} written raw: {remedy}.",
                u32::from(c)
            ),
            Severity::Error,
        ));
    }
}
