//! sql-validate-constraint-separate-migration — SQL text backend for .sql migrations.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{CheckCtx, TextCheck};
use crate::rules::sql_helpers::is_migration_path;

#[derive(Debug)]
pub struct Check;

impl TextCheck for Check {
    fn check(&self, ctx: &CheckCtx) -> Vec<Diagnostic> {
        if !is_migration_path(ctx.path) {
            return vec![];
        }
        super::validated_in_same_text(ctx.source)
            .into_iter()
            .map(|offset| {
                let (line, column) = byte_offset_to_line_col(ctx.source, offset);
                Diagnostic {
                    path: std::sync::Arc::clone(&ctx.path_arc),
                    line,
                    column,
                    rule_id: super::META.id.into(),
                    message: super::MESSAGE.into(),
                    severity: Severity::Error,
                    span: None,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn run(src: &str) -> Vec<Diagnostic> {
        Check.check(&CheckCtx::for_test(
            Path::new("/app/migrations/002.sql"),
            src,
        ))
    }

    #[test]
    fn flags_validate_of_constraint_added_not_valid_in_same_migration() {
        let src = "ALTER TABLE t ADD CONSTRAINT t_a_chk CHECK (a > 0) NOT VALID;\n\
                   SELECT 1;\n\
                   ALTER TABLE t validate constraint T_A_CHK;\n";
        let diagnostics = run(src);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 3);
    }

    #[test]
    fn allows_validate_without_not_valid_add_in_same_migration() {
        let only_validate = "ALTER TABLE t VALIDATE CONSTRAINT t_a_chk;";
        let other_name = "ALTER TABLE t ADD CONSTRAINT t_a_chk CHECK (a > 0) NOT VALID;\n\
                          ALTER TABLE t VALIDATE CONSTRAINT t_b_chk;";
        let other_clause = "ALTER TABLE t ADD CONSTRAINT t_a_chk CHECK (a > 0) NOT VALID,\n\
                            ADD CONSTRAINT t_b_chk CHECK (b > 0);\n\
                            ALTER TABLE t VALIDATE CONSTRAINT t_b_chk;";
        for src in [only_validate, other_name, other_clause] {
            assert!(run(src).is_empty(), "{src}");
        }
    }
}
