//! no-undeclared-env-var Rust backend — literal `env::var`/`env::var_os` reads.

use std::sync::Arc;

use crate::diagnostic::Diagnostic;
use crate::rules::backend::{AstCheck, CheckCtx};
use crate::rules::rust_helpers::{is_env_var_call, string_literal_value};

#[derive(Debug)]
pub struct Check;

impl AstCheck for Check {
    fn interested_kinds(&self) -> Option<&'static [&'static str]> {
        Some(&["call_expression"])
    }

    fn prefilter(&self) -> Option<&'static [&'static str]> {
        Some(&["env"])
    }

    fn visit_node(
        &self,
        node: tree_sitter::Node,
        ctx: &CheckCtx,
        _state: Option<&mut dyn std::any::Any>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let source = ctx.source.as_bytes();
        if !is_env_var_call(node, source) {
            return;
        }
        let Some(argument) = node
            .child_by_field_name("arguments")
            .and_then(|arguments| arguments.named_child(0))
            .filter(|argument| argument.kind() == "string_literal")
        else {
            return;
        };
        let Some(name) = string_literal_value(argument, source) else {
            return;
        };
        if !super::is_undeclared(ctx, name) {
            return;
        }
        diagnostics.push(Diagnostic::at_node(
            Arc::clone(&ctx.path_arc),
            &node,
            super::META.id,
            super::message(name),
            super::META.severity,
        ));
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
        crate::rules::test_helpers::run_ast_check(self, src, path, project, file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::test_helpers::run_rule;
    use tempfile::TempDir;

    #[test]
    fn flags_runtime_env_read_missing_from_env_example() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".env.example"), "OTHER=\n").unwrap();
        let src = "fn main() {\n    let _ = std::env::var(\"FOO\");\n    let _ = env::var_os(\"BAR\");\n}\n";
        let diagnostics = run_rule(&Check, src, dir.path().join("src/main.rs"));
        let found: Vec<(usize, String)> = diagnostics
            .into_iter()
            .map(|d| (d.line, d.message))
            .collect();
        let expected = vec![
            (2, super::super::message("FOO")),
            (3, super::super::message("BAR")),
        ];
        assert_eq!(found, expected);
    }

    #[test]
    fn accepts_keys_declared_empty_commented_or_exported() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join(".env.example"),
            "\u{feff}FOO=\r\n# BAR=\r\n#BAZ=\nexport QUX=1\n# Database settings\n",
        )
        .unwrap();
        let src = "fn main() {\n    let _ = env::var(\"FOO\");\n    let _ = env::var(\"BAR\");\n    let _ = env::var(\"BAZ\");\n    let _ = env::var(\"QUX\");\n    let _ = env::var(\"Database\");\n}\n";
        let diagnostics = run_rule(&Check, src, dir.path().join("src/main.rs"));
        let found: Vec<(usize, String)> = diagnostics
            .into_iter()
            .map(|d| (d.line, d.message))
            .collect();
        assert_eq!(found, vec![(6, super::super::message("Database"))]);
    }

    #[test]
    fn nearest_env_example_decides() {
        let src = "fn main() {\n    let _ = env::var(\"FOO\");\n}\n";
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("server/src")).unwrap();
        std::fs::write(dir.path().join(".env.example"), "FOO=\n").unwrap();
        std::fs::write(dir.path().join("server/.env.example"), "OTHER=\n").unwrap();
        let flagged = run_rule(&Check, src, dir.path().join("server/src/main.rs"));
        assert_eq!(flagged.len(), 1);
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("api/src")).unwrap();
        std::fs::write(dir.path().join(".env.example"), "OTHER=\n").unwrap();
        std::fs::write(dir.path().join("api/.env.example"), "FOO=\n").unwrap();
        let clean = run_rule(&Check, src, dir.path().join("api/src/main.rs"));
        assert!(clean.is_empty());
    }

    #[test]
    fn skips_file_without_env_example_above() {
        let dir = TempDir::new().unwrap();
        let src = "fn main() {\n    let _ = std::env::var(\"FOO\");\n}\n";
        let diagnostics = run_rule(&Check, src, dir.path().join("src/main.rs"));
        assert_eq!(diagnostics.len(), 0);
    }

    #[test]
    fn skips_allowlisted_name() {
        use crate::config::Config;
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("comply.toml"),
            "[rules.no-undeclared-env-var]\nallowed = [\"FOO\"]\n",
        )
        .unwrap();
        std::fs::write(dir.path().join(".env.example"), "OTHER=\n").unwrap();
        let cfg = Config::load_from(dir.path()).expect("load cfg");
        let src =
            "fn main() {\n    let _ = env::var(\"FOO\");\n    let _ = env::var(\"BAR\");\n}\n";
        let path = dir.path().join("src/main.rs");
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("grammar");
        let tree = parser.parse(src, None).expect("parse");
        let ctx = CheckCtx {
            config: &cfg,
            ..CheckCtx::for_test(&path, src)
        };
        let found: Vec<(usize, String)> = Check
            .check(&ctx, &tree)
            .into_iter()
            .map(|d| (d.line, d.message))
            .collect();
        assert_eq!(found, vec![(3, super::super::message("BAR"))]);
    }

    #[test]
    fn ignores_compile_time_macros_and_runtime_names() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".env.example"), "OTHER=\n").unwrap();
        let src = "fn main() {\n    let _ = env!(\"CARGO_PKG_VERSION\");\n    let _ = option_env!(\"FOO\");\n    let _ = env::var(name);\n}\n";
        let diagnostics = run_rule(&Check, src, dir.path().join("src/main.rs"));
        assert_eq!(diagnostics.len(), 0);
    }

    #[test]
    fn skips_test_directories() {
        use crate::files::Language;
        use crate::rules::file_ctx::FileCtx;
        let applies = |path: &str| {
            let file = FileCtx::build(
                std::path::Path::new(path),
                "",
                Language::Rust,
                crate::project::default_static_project_ctx(),
            );
            super::super::META.applies_to_file(&file)
        };
        assert!(!applies("tests/config.rs"));
        assert!(!applies("web/src/__tests__/env.ts"));
        assert!(applies("src/main.rs"));
    }
}
