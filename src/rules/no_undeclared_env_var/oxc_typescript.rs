//! no-undeclared-env-var oxc backend — literal `process.env` reads.

use std::sync::Arc;

use oxc_ast::ast::{BindingPattern, Expression};
use oxc_span::{GetSpan, Span};

use crate::diagnostic::Diagnostic;
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};

pub struct Check;

/// True for the `process.env` expression itself.
fn is_process_env(expression: &Expression) -> bool {
    let Expression::StaticMemberExpression(member) = expression else {
        return false;
    };
    member.property.name == "env"
        && matches!(&member.object, Expression::Identifier(object) if object.name == "process")
}

fn report(ctx: &CheckCtx, span: Span, name: &str, diagnostics: &mut Vec<Diagnostic>) {
    if !super::is_undeclared(ctx, name) {
        return;
    }
    diagnostics.push(Diagnostic::at_offset(
        Arc::clone(&ctx.path_arc),
        ctx.source,
        (span.start as usize, span.end as usize),
        super::META.id,
        super::message(name),
        super::META.severity,
    ));
}

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[
            AstType::StaticMemberExpression,
            AstType::ComputedMemberExpression,
            AstType::VariableDeclarator,
        ]
    }

    fn prefilter(&self) -> Option<&'static [&'static str]> {
        Some(&["env"])
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        _semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        match node.kind() {
            AstKind::StaticMemberExpression(member) if is_process_env(&member.object) => {
                report(ctx, member.span, &member.property.name, diagnostics);
            }
            AstKind::ComputedMemberExpression(member) if is_process_env(&member.object) => {
                if let Expression::StringLiteral(key) = &member.expression {
                    report(ctx, member.span, &key.value, diagnostics);
                }
            }
            AstKind::VariableDeclarator(declarator)
                if declarator.init.as_ref().is_some_and(is_process_env) =>
            {
                let BindingPattern::ObjectPattern(pattern) = &declarator.id else {
                    return;
                };
                for property in &pattern.properties {
                    if let Some(name) = property.key.static_name() {
                        report(ctx, property.span(), &name, diagnostics);
                    }
                }
            }
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
    ) -> Vec<Diagnostic> {
        crate::rules::test_helpers::run_oxc_check(self, src, path, project, file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::test_helpers::run_rule;
    use tempfile::TempDir;

    #[test]
    fn flags_process_env_reads_missing_from_env_example() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".env.example"), "OTHER=\n").unwrap();
        let src = "const bar = process.env.BAR;\nconst baz = process.env[\"BAZ\"];\nconst { QUX, ...rest } = process.env;\n";
        let found: Vec<(usize, String)> = run_rule(&Check, src, dir.path().join("server.ts"))
            .into_iter()
            .map(|d| (d.line, d.message))
            .collect();
        let expected = vec![
            (1, super::super::message("BAR")),
            (2, super::super::message("BAZ")),
            (3, super::super::message("QUX")),
        ];
        assert_eq!(found, expected);
    }

    #[test]
    fn accepts_declared_and_runtime_built_reads() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".env.example"), "BAR=\n# QUX=\n").unwrap();
        let src = "const bar = process.env.BAR;\nconst { QUX } = process.env;\nconst dynamic = process.env[key];\n";
        let diagnostics = run_rule(&Check, src, dir.path().join("server.tsx"));
        assert_eq!(diagnostics.len(), 0);
    }
}
