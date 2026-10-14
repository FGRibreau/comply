//! banned-copy-words oxc backend for TSX / JS(X).

use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::backend::{AstType, CheckCtx, OxcCheck};
use crate::rules::rendered_copy::copy_text;
use std::ops::Range;
use std::sync::Arc;

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
        let Some((text, start, _)) = copy_text(&node.kind(), ctx.source) else {
            return;
        };
        let config = ctx.config;
        let banned = config.required_string_list(super::META.id, "banned", ctx.lang);
        if banned.is_empty() {
            return;
        }
        let lower = text.to_lowercase();
        let allowed_phrases: Vec<String> = config
            .required_string_list(super::META.id, "allowed", ctx.lang)
            .iter()
            .map(|phrase| phrase.to_lowercase())
            .collect();
        let allowed: Vec<Range<usize>> = allowed_phrases
            .iter()
            .flat_map(|phrase| occurrences(&lower, phrase))
            .collect();
        // Lowercasing may shift offsets; then report at the text start.
        let same_offsets = lower.len() == text.len();
        for word in &banned {
            let word = word.to_lowercase();
            let flagged = occurrences(&lower, &word).filter(|found| {
                let is_allowed = allowed
                    .iter()
                    .any(|phrase| phrase.start <= found.start && found.end <= phrase.end);
                is_whole_word(&lower, found) && !is_allowed
            });
            for found in flagged {
                let span = if same_offsets {
                    (start.saturating_add(found.start), word.len())
                } else {
                    (start, 0)
                };
                diagnostics.push(Diagnostic::at_offset(
                    Arc::clone(&ctx.path_arc),
                    ctx.source,
                    span,
                    super::META.id,
                    format!("`{word}` is banned from UI copy by comply.toml."),
                    Severity::Error,
                ));
            }
        }
    }
}

fn occurrences<'a>(text: &'a str, needle: &'a str) -> impl Iterator<Item = Range<usize>> + 'a {
    text.match_indices(needle)
        .map(|(index, found)| index..index.saturating_add(found.len()))
}

fn is_whole_word(text: &str, found: &Range<usize>) -> bool {
    let before = text[..found.start].chars().next_back();
    let after = text[found.end..].chars().next();
    !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
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
    use crate::rules::test_helpers::run_oxc_rule_with_toml;

    fn run_banning(words: &str, source: &str) -> Vec<Diagnostic> {
        let toml = format!("[rules.banned-copy-words]\nbanned = {words}\nallowed = []\n");
        run_oxc_rule_with_toml(&Check, source, "Page.tsx", &toml)
    }

    #[test]
    fn flags_listed_word_in_copy_case_insensitively() {
        for source in [
            "<p>Nous sommes Désolé.</p>",
            "<i title=\"désolé\" />",
            "<i title={\"désolé\"} />",
        ] {
            let diagnostics = run_banning("[\"désolé\"]", source);
            assert_eq!(diagnostics.len(), 1, "{source}");
            assert!(diagnostics[0].message.contains("désolé"), "{source}");
        }
    }

    #[test]
    fn passes_allowlisted_phrase() {
        let toml = "[rules.banned-copy-words]\nbanned = [\"simplement\"]\n\
                    allowed = [\"tout simplement\"]\n";
        let allowed =
            run_oxc_rule_with_toml(&Check, "<p>C'est tout simplement ça</p>", "Page.tsx", toml);
        assert!(allowed.is_empty());
        let bare = run_oxc_rule_with_toml(&Check, "<p>Simplement ça</p>", "Page.tsx", toml);
        assert_eq!(bare.len(), 1);
    }

    #[test]
    fn ignores_comments_non_text_attributes_and_partial_words() {
        for source in [
            "<p>{/* désolé */}</p>",
            "<i className=\"désolé\" />",
            "<input value=\"désolé\" />",
            "<p>désolée</p>",
        ] {
            assert!(run_banning("[\"désolé\"]", source).is_empty(), "{source}");
        }
    }

    #[test]
    fn empty_list_is_inert() {
        let diagnostics =
            crate::rules::test_helpers::run_rule(&Check, "<p>désolé</p>", "Page.tsx");
        assert!(diagnostics.is_empty());
    }
}
