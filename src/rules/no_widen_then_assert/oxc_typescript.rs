//! no-widen-then-assert oxc backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::{
    byte_offset_to_line_col, is_const_assertion, peel_parens, widened_literal_const,
};
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use oxc_ast::ast::{Expression, TSType};
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
        let Expression::Identifier(ident) = peel_parens(&as_expr.expression) else {
            return;
        };
        let scoping = semantic.scoping();
        let Some(symbol) = ident
            .reference_id
            .get()
            .and_then(|reference| scoping.get_reference(reference).symbol_id())
        else {
            return;
        };
        let declaration = semantic.nodes().kind(scoping.symbol_declaration(symbol));
        let AstKind::VariableDeclarator(declarator) = declaration else {
            return;
        };
        if !widened_literal_const(declarator) {
            return;
        }
        let Some(annotation) = &declarator.type_annotation else {
            return;
        };
        let target = &as_expr.type_annotation;
        if matches!(target, TSType::TSUnknownKeyword(_) | TSType::TSAnyKeyword(_))
            || target.span().source_text(ctx.source)
                == annotation.type_annotation.span().source_text(ctx.source)
        {
            return;
        }
        let (line, column) = byte_offset_to_line_col(ctx.source, as_expr.span.start as usize);
        diagnostics.push(Diagnostic {
            path: Arc::clone(&ctx.path_arc),
            line,
            column,
            rule_id: super::META.id.into(),
            message: format!(
                "`{}` is a literal `const` widened by its annotation, then asserted back — \
                 drop the annotation so it keeps its literal type.",
                ident.name
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

    #[test]
    fn flags_assertion_of_widened_const_only() {
        assert_eq!(run("const a: string = \"x\"; f(a as \"x\");").len(), 1);
        assert!(run("const a = \"x\"; f(a);").is_empty());
        assert!(run("const a: string = g(); f(a as \"x\");").is_empty());
        assert!(run("let a: string = \"x\"; f(a as \"x\");").is_empty());
        assert!(run("const a: string = \"x\"; f(a as unknown);").is_empty());
        assert!(run("const a: string = \"x\"; f(a as string);").is_empty());
    }
}
