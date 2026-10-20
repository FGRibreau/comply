//! copy-apostrophe-style oxc backend for TS, TSX and JS(X).

use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::backend::{AstType, CheckCtx, OxcCheck};
use crate::rules::rendered_copy::copy_text;
use std::sync::Arc;

pub struct Check;

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[
            AstType::JSXText,
            AstType::StringLiteral,
            AstType::TemplateElement,
        ]
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let Some((text, start, _, _)) = copy_text(node, semantic, ctx.source) else {
            return;
        };
        let style = ctx.config.string(super::META.id, "style", ctx.lang);
        let (wrong, right) = match style {
            "typographic" => ('\'', '’'),
            "straight" => ('’', '\''),
            other => panic!(
                "config key `[rules.\"{}\"] style` must be `typographic` or `straight`, got `{other}`",
                super::META.id
            ),
        };
        let elisions = text
            .char_indices()
            .filter(|&(offset, c)| c == wrong && is_elision(text, offset));
        for (offset, c) in elisions {
            diagnostics.push(Diagnostic::at_offset(
                Arc::clone(&ctx.path_arc),
                ctx.source,
                (start.saturating_add(offset), c.len_utf8()),
                super::META.id,
                format!("Apostrophe style is `{style}`: write `{right}` here."),
                Severity::Error,
            ));
        }
    }
}

/// True when the apostrophe at `offset` elides (`l'agent`, `d'{x}`).
fn is_elision(text: &str, offset: usize) -> bool {
    let follows_letter = text[..offset]
        .chars()
        .next_back()
        .is_some_and(char::is_alphabetic);
    let precedes_letter_or_end = text[offset..]
        .chars()
        .nth(1)
        .is_none_or(char::is_alphabetic);
    follows_letter && precedes_letter_or_end
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
    use crate::rules::test_helpers::{run_oxc_rule_with_toml, run_rule};

    fn run(source: &str) -> Vec<Diagnostic> {
        run_rule(&Check, source, "Page.tsx")
    }

    #[test]
    fn flags_straight_apostrophe_under_default_style() {
        for source in ["<p>l'agent</p>", "<p>d'{orgName}</p>"] {
            let diagnostics = run(source);
            assert_eq!(diagnostics.len(), 1, "{source}");
            assert!(diagnostics[0].message.contains('’'), "{source}");
        }
        assert!(run("<p>l’agent</p>").is_empty());
    }

    #[test]
    fn straight_style_flags_typographic() {
        let toml = "[rules.copy-apostrophe-style]\nstyle = \"straight\"\n";
        let straight = run_oxc_rule_with_toml(&Check, "<p>l'agent</p>", "Page.tsx", toml);
        assert!(straight.is_empty());
        let typographic = run_oxc_rule_with_toml(&Check, "<p>l’agent</p>", "Page.tsx", toml);
        assert_eq!(typographic.len(), 1);
    }

    #[test]
    fn flags_elision_in_ts_strings_templates_and_attributes() {
        for path in ["Page.ts", "Page.tsx"] {
            for source in ["const m = \"l'agent répond\";", "const m = `d'abord ${x} ici`;"] {
                assert_eq!(run_rule(&Check, source, path).len(), 1, "{path}: {source}");
            }
        }
        let attribute = run("<Banner message=\"l'agent est là\" />");
        assert_eq!(attribute.len(), 1);
        for source in ["const o = { \"l'x\": 1 };", "const id = \"l'x\";", "<i className=\"l'x\" />"] {
            assert!(run(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn ignores_quotes_and_non_copy() {
        for source in [
            "<p>'citation' ok</p>",
            "const s = \"l'agent\";",
            "<i className=\"l'x\" />",
        ] {
            assert!(run(source).is_empty(), "{source}");
        }
    }
}
