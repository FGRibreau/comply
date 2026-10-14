//! require-safety-comment-for-type-assertion oxc backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::{byte_offset_to_line_col, is_const_assertion};
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use oxc_span::GetSpan;
use std::sync::Arc;

pub struct Check;

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::TSAsExpression]
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let AstKind::TSAsExpression(as_expr) = node.kind() else {
            return;
        };
        if is_const_assertion(&as_expr.type_annotation) {
            return;
        }
        let marker = ctx.config.string(super::META.id, "marker", ctx.lang);
        let (line, column) = byte_offset_to_line_col(ctx.source, as_expr.span.start as usize);
        let (type_line, _) =
            byte_offset_to_line_col(ctx.source, as_expr.type_annotation.span().start as usize);
        let justified = semantic.comments().iter().any(|comment| {
            let text = &ctx.source[comment.span.start as usize..comment.span.end as usize];
            if !text.contains(marker) {
                return false;
            }
            let (start_line, _) = byte_offset_to_line_col(ctx.source, comment.span.start as usize);
            let (end_line, _) = byte_offset_to_line_col(ctx.source, comment.span.end as usize);
            start_line == line || start_line == type_line || end_line.checked_add(1) == Some(line)
        });
        if justified {
            return;
        }
        diagnostics.push(Diagnostic {
            path: Arc::clone(&ctx.path_arc),
            line,
            column,
            rule_id: super::META.id.into(),
            message: format!(
                "this type assertion carries no `{marker}` comment — \
                 write why it holds on its line or the line above."
            ),
            severity: Severity::Error,
            span: None,
        });
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

    fn run(src: &str) -> Vec<Diagnostic> {
        crate::rules::test_helpers::run_rule(&Check, src, "t.ts")
    }

    fn run_with_marker(src: &str, marker: &str) -> Vec<Diagnostic> {
        let toml = format!("[rules.{}]\nmarker = \"{marker}\"\n", super::super::META.id);
        crate::rules::test_helpers::run_oxc_check_with_config(&Check, src, &toml)
    }

    #[test]
    fn requires_marker_above_or_trailing() {
        assert_eq!(run("const a = x as T;").len(), 1);
        assert!(run("// SAFETY: x is a T here.\nconst a = x as T;").is_empty());
        assert!(run("const a = x as T; // SAFETY: x is a T here.").is_empty());
        assert!(run("const a = f(\n  b,\n) as T; // SAFETY: f returns a T.").is_empty());
        assert!(run("const a = x as const;").is_empty());
        assert_eq!(run("const s = \"SAFETY: y\";\nconst a = x as T;").len(), 1);
    }

    #[test]
    fn custom_marker_replaces_default() {
        assert!(run_with_marker("// CAST: x is a T here.\nconst a = x as T;", "CAST:").is_empty());
        assert_eq!(
            run_with_marker("// SAFETY: x is a T here.\nconst a = x as T;", "CAST:").len(),
            1
        );
    }
}
