//! migration-needs-lock-timeout — SQL backend.
//!
//! For `.sql` migration files: check the raw content for DDL keywords.
//! No AST needed — the whole file is SQL.

use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::backend::{CheckCtx, TextCheck};

#[derive(Debug)]
pub struct Check;

impl TextCheck for Check {
    fn check(&self, ctx: &CheckCtx) -> Vec<Diagnostic> {
        if !crate::rules::sql_helpers::is_migration_path(ctx.path) {
            return vec![];
        }
        if crate::rules::sql_helpers::is_clickhouse_ddl(ctx.source) {
            return vec![];
        }
        if !super::contains_ddl(ctx.source) {
            return vec![];
        }
        if super::declares_lock_timeout(ctx.source) {
            return vec![];
        }
        vec![Diagnostic {
            path: std::sync::Arc::clone(&ctx.path_arc),
            line: 1,
            column: 1,
            rule_id: super::META.id.into(),
            message: "DDL without `SET lock_timeout` — add `SET lock_timeout = '5s';` at the top to prevent write queue pileups.".into(),
            severity: Severity::Error,
            span: None,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn run(path: &str, src: &str) -> Vec<Diagnostic> {
        let ctx = CheckCtx::for_test(Path::new(path), src);
        Check.check(&ctx)
    }

    #[test]
    fn flags_ddl_without_lock_timeout() {
        let diags = run(
            "/app/migrations/001_add_col.sql",
            "ALTER TABLE users ADD COLUMN age INT;",
        );
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn allows_ddl_with_lock_timeout() {
        let diags = run(
            "/app/migrations/001_add_col.sql",
            "SET lock_timeout = '5s';\nALTER TABLE users ADD COLUMN age INT;",
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn skips_non_migration_path() {
        let diags = run(
            "/app/src/schema.sql",
            "ALTER TABLE users ADD COLUMN age INT;",
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn skips_non_ddl_sql() {
        let diags = run(
            "/app/migrations/002_seed.sql",
            "INSERT INTO users (name) VALUES ('alice');",
        );
        assert!(diags.is_empty());
    }

    // A `-- no-transaction` file holding one concurrent index build.
    // sqlx sends it as one simple query.
    // A `SET` in front opens a transaction, which `CONCURRENTLY` refuses.
    const CONCURRENT_BUILD: &str = "-- no-transaction.\n\
        -- atlas:txmode none\n\n\
        -- A failed build leaves it INVALID. The next file repairs it.\n\
        CREATE INDEX CONCURRENTLY IF NOT EXISTS \"idx_execution_clear\"\n  \
        ON \"workflow\".\"execution\" (\"execution_id\")\n  \
        WHERE jsonb_typeof(definition->'secret_data') = 'object';";

    #[test]
    fn skips_file_with_only_a_concurrent_index_build() {
        assert!(run("/app/migrations/20261117120000_index.sql", CONCURRENT_BUILD).is_empty());
    }

    #[test]
    fn skips_concurrent_drop_and_reindex() {
        for src in [
            "-- no-transaction.\nDROP INDEX CONCURRENTLY IF EXISTS \"invitations_one_accepted_idx\";",
            "-- no-transaction.\nREINDEX INDEX CONCURRENTLY \"workflow\".\"idx_clear\";",
            "CREATE INDEX CONCURRENTLY a ON t (x); -- speeds up x lookups\nDROP INDEX CONCURRENTLY b;",
        ] {
            assert!(run("/app/migrations/002.sql", src).is_empty(), "{src}");
        }
    }

    #[test]
    fn flags_blocking_ddl_next_to_a_concurrent_build() {
        for src in [
            "CREATE INDEX CONCURRENTLY a ON t (x); -- speeds up x lookups\nALTER TABLE t ADD COLUMN y INT;",
            "CREATE INDEX CONCURRENTLY a ON t (x); -- speeds up x lookups\nCREATE INDEX b ON t (y); -- speeds up y lookups",
            "DROP INDEX CONCURRENTLY a;\nDROP INDEX b;",
            "-- built concurrently by the next file\nCREATE INDEX b ON t (y);",
            "/* concurrently */ CREATE INDEX b ON t (y); -- speeds up y lookups",
        ] {
            assert_eq!(run("/app/migrations/003.sql", src).len(), 1, "{src}");
        }
    }

    #[test]
    fn skips_clickhouse_migration_issue_7765() {
        let diags = run(
            "/app/migrations/003_ch.sql",
            "CREATE TABLE Events (id UInt64) ENGINE = MergeTree ORDER BY id;\nALTER TABLE Events ADD COLUMN name String;",
        );
        assert!(diags.is_empty());
    }
}
