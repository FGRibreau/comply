//! no-raw-invisible-character tree-sitter backend for Rust literals.

use crate::rules::rendered_copy::Escapes;

crate::ast_check! { on ["string_literal", "raw_string_literal", "char_literal"] => |node, source, ctx, diagnostics|
    let range = node.byte_range();
    let Some(text) = ctx.source.get(range.clone()) else { return };
    let in_raw_literal = node.kind() == "raw_string_literal";
    super::check_text(ctx, text, range.start, Escapes::Rust, in_raw_literal, diagnostics);
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
        crate::rules::test_helpers::run_ast_check(self, src, path, project, file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::test_helpers::run_rule;

    #[test]
    fn flags_raw_invisible_in_rust_literals_only() {
        let cases = [
            ("fn f() { let _ = \"a\u{a0}b\"; }", "`\\u{a0}`"),
            (
                "fn f() { let _ = r\"a\u{200b}b\"; }",
                "move the text to a plain literal and write `\\u{200b}` there",
            ),
            ("fn f() { let _ = '\u{a0}'; }", "`\\u{a0}`"),
        ];
        for (source, remedy) in cases {
            let diagnostics = run_rule(&Check, source, "lib.rs");
            assert_eq!(diagnostics.len(), 1, "{source:?}");
            assert!(diagnostics[0].message.contains(remedy), "{source:?}");
        }
        assert!(run_rule(&Check, r#"fn f() { let _ = "\u{a0}"; }"#, "lib.rs").is_empty());
        assert!(run_rule(&Check, "// a\u{a0}b\nfn f() {}", "lib.rs").is_empty());
    }
}
