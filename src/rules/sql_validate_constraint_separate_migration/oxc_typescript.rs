//! sql-validate-constraint-separate-migration — oxc backend for TS / JS / TSX.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use crate::rules::sql_helpers::is_migration_path;
use std::sync::Arc;

pub struct Check;

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::StringLiteral, AstType::TemplateLiteral]
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        _semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if !is_migration_path(ctx.path) {
            return;
        }
        let span = match node.kind() {
            AstKind::StringLiteral(lit) => lit.span,
            AstKind::TemplateLiteral(tpl) => tpl.span,
            _ => return,
        };
        // Raw source text keeps offsets aligned with source lines.
        let Ok(start) = usize::try_from(span.start) else {
            return;
        };
        let offsets = super::validated_in_same_text(span.source_text(ctx.source))
            .into_iter()
            .filter_map(|offset| start.checked_add(offset));
        for offset in offsets {
            let (line, column) = byte_offset_to_line_col(ctx.source, offset);
            diagnostics.push(Diagnostic {
                path: Arc::clone(&ctx.path_arc),
                line,
                column,
                rule_id: super::META.id.into(),
                message: super::MESSAGE.into(),
                severity: Severity::Error,
                span: None,
            });
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

    #[test]
    fn flags_add_not_valid_and_validate_in_one_migration_template() {
        let src = "export const up = `\n  ALTER TABLE t ADD CONSTRAINT t_a_chk CHECK (a > 0) NOT VALID;\n  ALTER TABLE t VALIDATE CONSTRAINT t_a_chk;\n`;\n";
        let diagnostics = crate::rules::test_helpers::run_rule(&Check, src, "migrations/001.ts");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 3);
    }
}
