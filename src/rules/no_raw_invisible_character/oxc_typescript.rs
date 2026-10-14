//! no-raw-invisible-character oxc backend for TS / TSX / JS(X).

use crate::diagnostic::Diagnostic;
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use crate::rules::rendered_copy::Escapes;
use oxc_span::Span;

pub struct Check;

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[
            AstType::StringLiteral,
            AstType::TemplateLiteral,
            AstType::JSXText,
        ]
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let mut check = |span: Span, escapes: Escapes, in_raw_literal: bool| {
            let (start, end) = (span.start as usize, span.end as usize);
            if let Some(text) = ctx.source.get(start..end) {
                super::check_text(ctx, text, start, escapes, in_raw_literal, diagnostics);
            }
        };
        let parent = semantic.nodes().parent_kind(node.id());
        match node.kind() {
            AstKind::StringLiteral(lit) => {
                let in_attribute = matches!(parent, AstKind::JSXAttribute(_));
                let escapes = if in_attribute {
                    Escapes::Html
                } else {
                    Escapes::Js
                };
                check(lit.span, escapes, false);
            }
            AstKind::TemplateLiteral(template) => {
                let in_string_raw = matches!(
                    parent,
                    AstKind::TaggedTemplateExpression(tagged)
                        if tagged.tag.is_specific_member_access("String", "raw")
                );
                for quasi in &template.quasis {
                    check(quasi.span, Escapes::Js, in_string_raw);
                }
            }
            AstKind::JSXText(text) => check(text.span, Escapes::Html, false),
            _ => {}
        }
    }
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
    use crate::rules::test_helpers::run_rule;

    #[test]
    fn flags_raw_invisible_in_strings_and_jsx_text() {
        assert_eq!(
            run_rule(&Check, "const s = \"a\u{200b}b\";", "t.ts").len(),
            1
        );
        assert_eq!(run_rule(&Check, "const t = `a\u{200b}b`;", "t.ts").len(), 1);
        assert_eq!(run_rule(&Check, "<p>a\u{a0}b</p>", "Page.tsx").len(), 1);
    }

    #[test]
    fn passes_escaped_forms_comments_and_emoji() {
        for (source, path) in [
            ("const s = \"a\\u200Bb\";", "t.ts"),
            ("<p>a&nbsp;b</p>", "Page.tsx"),
            ("// a\u{200b}b", "t.ts"),
            ("const w = \"⚠\u{fe0f}\";", "t.ts"),
        ] {
            assert!(run_rule(&Check, source, path).is_empty(), "{source}");
        }
    }

    #[test]
    fn message_names_the_escape_of_its_context() {
        for (source, path, escape) in [
            ("<p>a\u{a0}b</p>", "Page.tsx", "&nbsp;"),
            ("<i title=\"a\u{a0}b\" />", "Page.tsx", "&nbsp;"),
            ("const s = \"a\u{a0}b\";", "t.ts", "\\u00A0"),
            ("<p>a\u{200b}b</p>", "Page.tsx", "&#x200B;"),
        ] {
            let diagnostics = run_rule(&Check, source, path);
            assert_eq!(diagnostics.len(), 1, "{source}");
            let message = &diagnostics[0].message;
            assert!(message.contains(escape), "{source}");
            assert!(!message.contains(['\u{a0}', '\u{200b}']), "{source}");
        }
    }
}
