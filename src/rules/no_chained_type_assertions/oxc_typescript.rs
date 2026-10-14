//! no-chained-type-assertions oxc backend — flag the outermost `as` of a chain.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::{byte_offset_to_line_col, is_const_assertion, peel_parens};
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use oxc_ast::ast::{Expression, TSAsExpression};
use std::sync::Arc;

/// True when `node` asserts a type other than `const`.
fn is_link(node: &TSAsExpression) -> bool {
    !is_const_assertion(&node.type_annotation)
}

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
        if !is_link(as_expr) {
            return;
        }
        let Expression::TSAsExpression(inner) = peel_parens(&as_expr.expression) else {
            return;
        };
        if !is_link(inner) {
            return;
        }
        let outer = semantic
            .nodes()
            .ancestor_kinds(node.id())
            .find(|kind| !matches!(kind, AstKind::ParenthesizedExpression(_)));
        if matches!(outer, Some(AstKind::TSAsExpression(parent)) if is_link(parent)) {
            return;
        }
        let (line, column) = byte_offset_to_line_col(ctx.source, as_expr.span.start as usize);
        diagnostics.push(Diagnostic {
            path: Arc::clone(&ctx.path_arc),
            line,
            column,
            rule_id: super::META.id.into(),
            message: "a chained type assertion asserts a type nothing checked — \
                      parse the value into the target type instead."
                .into(),
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

    #[test]
    fn flags_chains_once_and_passes_single_or_const() {
        assert_eq!(run("const a = x as unknown as T;").len(), 1);
        assert_eq!(run("const a = (x as A) as B;").len(), 1);
        assert_eq!(run("const a = ((x as A) as B) as C;").len(), 1);
        assert!(run("const a = x as T;").is_empty());
        assert!(run("const a = x as const;").is_empty());
        assert!(run("const a = ({ a: 1 } as const) as T;").is_empty());
    }
}
