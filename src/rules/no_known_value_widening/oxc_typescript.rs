//! no-known-value-widening oxc backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::{byte_offset_to_line_col, widened_literal_const};
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use std::sync::Arc;

pub struct Check;

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::VariableDeclarator]
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        _semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let AstKind::VariableDeclarator(declarator) = node.kind() else {
            return;
        };
        if !widened_literal_const(declarator) {
            return;
        }
        let Some(annotation) = &declarator.type_annotation else {
            return;
        };
        let (line, column) = byte_offset_to_line_col(ctx.source, annotation.span.start as usize);
        diagnostics.push(Diagnostic {
            path: Arc::clone(&ctx.path_arc),
            line,
            column,
            rule_id: super::META.id.into(),
            message: "this annotation widens a literal value the compiler already knew — \
                      drop it, or write `as const` / `satisfies T`."
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
    fn flags_annotated_literals_only() {
        assert_eq!(run("const o: Record<string, number> = { a: 1 };").len(), 1);
        assert_eq!(run("const s: string = \"x\";").len(), 1);
        assert!(run("const o = { a: 1 } as const;").is_empty());
        assert!(run("const s: \"x\" = \"x\";").is_empty());
        assert!(run("const n: number = f();").is_empty());
        assert!(run("const acc: Record<string, number> = {};").is_empty());
        assert!(run("const xs: string[] = [];").is_empty());
    }
}
