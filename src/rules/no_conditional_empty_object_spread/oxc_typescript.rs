//! no-conditional-empty-object-spread oxc backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use oxc_ast::ast::{Expression, LogicalOperator};
use std::sync::Arc;

pub struct Check;

fn is_empty_object(expr: &Expression) -> bool {
    matches!(expr.without_parentheses(), Expression::ObjectExpression(obj) if obj.properties.is_empty())
}

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::SpreadElement]
    }

    fn prefilter(&self) -> Option<&'static [&'static str]> {
        Some(&["..."])
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let AstKind::SpreadElement(spread) = node.kind() else {
            return;
        };
        if !matches!(
            semantic.nodes().parent_node(node.id()).kind(),
            AstKind::ObjectExpression(_)
        ) {
            return;
        }
        let is_conditional_spread = match spread.argument.without_parentheses() {
            Expression::ConditionalExpression(cond) => {
                is_empty_object(&cond.consequent) || is_empty_object(&cond.alternate)
            }
            Expression::LogicalExpression(logical) => {
                logical.operator == LogicalOperator::And
                    && matches!(
                        logical.right.without_parentheses(),
                        Expression::ObjectExpression(_)
                    )
            }
            _ => false,
        };
        if !is_conditional_spread {
            return;
        }

        let (line, column) = byte_offset_to_line_col(ctx.source, spread.span.start as usize);
        diagnostics.push(Diagnostic {
            path: Arc::clone(&ctx.path_arc),
            line,
            column,
            rule_id: super::META.id.into(),
            message: "Conditional object spread — set the key explicitly \
                      (`a: c ? value : undefined`) or assign it afterwards."
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
    fn flags_conditional_object_spread() {
        assert_eq!(run("const o = { ...(c ? { a } : {}) };").len(), 1);
        assert_eq!(run("const o = { ...(c ? {} : { a }) };").len(), 1);
        assert_eq!(run("const o = { ...(c && { a }) };").len(), 1);
    }

    #[test]
    fn allows_rest_and_two_filled_branches() {
        assert!(run("const o = { ...rest };").is_empty());
        assert!(run("const o = { ...(c ? { a } : { b }) };").is_empty());
        assert!(run("const xs = [...(c ? [a] : [])];").is_empty());
    }
}
