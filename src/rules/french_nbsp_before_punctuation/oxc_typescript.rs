//! french-nbsp-before-punctuation oxc backend for TSX / JS(X).

use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::backend::{AstType, CheckCtx, OxcCheck};
use crate::rules::rendered_copy::{Escapes, copy_text, escape};
use std::sync::Arc;

const MARKS: [char; 4] = [':', ';', '?', '!'];
const CLOSING: [char; 7] = [')', ']', '»', '”', '"', '’', '\''];

pub struct Check;

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::JSXText, AstType::JSXAttribute]
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        _semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let Some((text, start, escapes)) = copy_text(&node.kind(), ctx.source) else {
            return;
        };
        for (offset, mark) in unspaced_marks(text, escapes) {
            let nbsp = if mark == ':' { '\u{a0}' } else { '\u{202f}' };
            diagnostics.push(Diagnostic::at_offset(
                Arc::clone(&ctx.path_arc),
                ctx.source,
                (start.saturating_add(offset), mark.len_utf8()),
                super::META.id,
                format!(
                    "French copy needs a non-breaking space before `{mark}`: write `{}` \
                     before it.",
                    escape(nbsp, escapes)
                ),
                Severity::Error,
            ));
        }
    }
}

fn unspaced_marks(text: &str, escapes: Escapes) -> impl Iterator<Item = (usize, char)> + '_ {
    text.char_indices().filter(move |&(offset, mark)| {
        MARKS.contains(&mark) && lacks_nbsp(text, offset, mark, escapes)
    })
}

/// True when the mark at `offset` lacks its non-breaking space.
fn lacks_nbsp(text: &str, offset: usize, mark: char, escapes: Escapes) -> bool {
    let before = &text[..offset];
    let Some(previous) = before.chars().next_back() else {
        return false;
    };
    let is_glued = previous.is_alphabetic() || CLOSING.contains(&previous);
    if !is_glued && previous != ' ' {
        return false;
    }
    let is_entity_end = escapes == Escapes::Html && mark == ';' && closes_entity(before);
    // Only a JS narrow-NBSP escape ends in a letter.
    let narrow_nbsp = "\\u202f";
    let is_escaped_nbsp = escapes == Escapes::Js
        && before
            .get(before.len().saturating_sub(narrow_nbsp.len())..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(narrow_nbsp));
    if is_entity_end || is_escaped_nbsp {
        return false;
    }
    // The rest of a run (`?!`) rides on its first mark.
    let next = text[offset..].trim_start_matches(MARKS).chars().next();
    next.is_none_or(char::is_whitespace)
}

/// True when `before` ends with an unterminated `&name` or `&#123`.
fn closes_entity(before: &str) -> bool {
    let Some((_, name)) = before.rsplit_once('&') else {
        return false;
    };
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '#')
}

#[cfg(test)]
impl crate::rules::test_helpers::RunRule for Check {
    fn meta(&self) -> &'static crate::rules::meta::RuleMeta {
        &super::META
    }
    fn execute_with_ctx(
        &self,
        src: &str,
        path: &std::path::Path,
        project: &crate::project::ProjectCtx,
        file: &crate::rules::file_ctx::FileCtx,
    ) -> Vec<crate::diagnostic::Diagnostic> {
        crate::rules::test_helpers::run_oxc_check(self, src, path, project, file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Vec<Diagnostic> {
        crate::rules::test_helpers::run_rule(&Check, source, "Page.tsx")
    }

    #[test]
    fn flags_space_or_letter_before_high_punctuation() {
        let colon = run("<p>Attention : ok</p>");
        assert_eq!(colon.len(), 1);
        assert_eq!(colon[0].column, 14);
        let question = run("<input title=\"Prêt ?\" />");
        assert_eq!(question.len(), 1);
        assert_eq!(question[0].column, 20);
        let braced = run("<input title={\"Prêt ?\"} />");
        assert_eq!(braced.len(), 1);
        assert_eq!(braced[0].column, 21);
    }

    #[test]
    fn passes_escaped_nbsp_and_non_prose_colons() {
        for source in [
            "<p>Attention&nbsp;: ok</p>",
            "<p>Attention{\"\\u202F\"}: ok</p>",
            "<i title=\"Prêt&#8239;?\" />",
            "<i title={\"Prêt\\u202F?\"} />",
            "<p>12:30</p>",
            "<p>https://x</p>",
        ] {
            assert!(run(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn flags_a_run_of_marks_once_on_the_first() {
        let run_of_marks = run("<p>Quoi ?!</p>");
        assert_eq!(run_of_marks.len(), 1);
        assert_eq!(run_of_marks[0].column, 9);
        assert!(run("<p>Quoi&nbsp;?!</p>").is_empty());
    }

    #[test]
    fn ignores_entity_semicolons_and_non_copy() {
        for source in [
            "<p>caf&eacute; ok</p>",
            "<i className=\"a : b\" />",
            "<input value=\"Prêt ?\" />",
            "const s = \"Prêt ?\";",
            "<p>{/* Prêt ? */}</p>",
        ] {
            assert!(run(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn message_names_the_escape_of_its_context() {
        let text = run("<p>Attention : ok</p>");
        assert!(text[0].message.contains("&nbsp;"));
        let narrow = run("<p>Prêt ?</p>");
        assert!(narrow[0].message.contains("&#8239;"));
        let braced = run("<i title={\"Prêt ?\"} />");
        assert!(braced[0].message.contains("\\u202F"));
    }
}
