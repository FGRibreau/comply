//! no-runtime-typeof oxc backend — flag the `typeof` unary operator.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::{byte_offset_to_line_col, is_inside_type_predicate_fn};
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use oxc_ast::ast::UnaryOperator;
use std::sync::Arc;

pub struct Check;

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::UnaryExpression]
    }

    fn prefilter(&self) -> Option<&'static [&'static str]> {
        Some(&["typeof"])
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let AstKind::UnaryExpression(unary) = node.kind() else {
            return;
        };
        if unary.operator != UnaryOperator::Typeof {
            return;
        }
        if ctx
            .config
            .bool_flag(super::META.id, "allow_in_type_guards", ctx.lang)
            && is_inside_type_predicate_fn(node.id(), semantic)
        {
            return;
        }
        let (line, column) = byte_offset_to_line_col(ctx.source, unary.span.start as usize);
        diagnostics.push(Diagnostic {
            path: Arc::clone(&ctx.path_arc),
            line,
            column,
            rule_id: super::META.id.into(),
            message: "a runtime `typeof` re-checks by hand what a parsed type guarantees — \
                      parse the value at its boundary, then trust the type."
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

    const GUARD: &str = "const isText = (x: unknown): x is string => typeof x === \"string\";";

    fn run(src: &str) -> Vec<Diagnostic> {
        crate::rules::test_helpers::run_rule(&Check, src, "t.ts")
    }

    fn run_allowing_guards(src: &str) -> Vec<Diagnostic> {
        let toml = format!("[rules.{}]\nallow_in_type_guards = true\n", super::super::META.id);
        crate::rules::test_helpers::run_oxc_check_with_config(&Check, src, &toml)
    }

    #[test]
    fn flags_runtime_typeof_not_type_position() {
        assert_eq!(run("if (typeof x === \"string\") {}").len(), 1);
        assert_eq!(run(GUARD).len(), 1);
        assert!(run("type T = typeof x;").is_empty());
    }

    #[test]
    fn allow_in_type_guards_exempts_only_guards() {
        assert!(run_allowing_guards(GUARD).is_empty());
        assert_eq!(
            run_allowing_guards("if (typeof x === \"string\") {}").len(),
            1
        );
    }
}
