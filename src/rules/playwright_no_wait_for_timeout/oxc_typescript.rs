//! playwright-no-wait-for-timeout oxc backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use oxc_ast::ast::Expression;
use std::sync::Arc;

const TEST_MARKERS: &[&str] = &[".test.", ".spec.", "__tests__", "_test.", ".e2e."];

fn is_test_file(path: &std::path::Path) -> bool {
    let s = path.to_string_lossy();
    TEST_MARKERS.iter().any(|m| s.contains(m))
}

pub struct Check;

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::CallExpression]
    }

    fn prefilter(&self) -> Option<&'static [&'static str]> {
        Some(&["waitForTimeout"])
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        _semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if !is_test_file(ctx.path) {
            return;
        }
        if !crate::rules::playwright::is_playwright_context(ctx) {
            return;
        }

        let AstKind::CallExpression(call) = node.kind() else {
            return;
        };

        let Expression::StaticMemberExpression(member) = &call.callee else {
            return;
        };

        if member.property.name.as_str() != "waitForTimeout" {
            return;
        }

        let (line, column) =
            byte_offset_to_line_col(ctx.source, call.span.start as usize);
        diagnostics.push(Diagnostic {
            path: Arc::clone(&ctx.path_arc),
            line,
            column,
            rule_id: super::META.id.into(),
            message: "Unexpected use of page.waitForTimeout(); prefer web-first assertions."
                .into(),
            severity: Severity::Error,
            span: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The spec imports `test` from a local fixture module.
    const FIXTURE_SPEC: &str = "import { test } from './fixtures';\n\
                                test('a', async ({ page }) => {\n\
                                  await page.waitForTimeout(1000);\n\
                                });\n";

    fn run_at(path: &str) -> Vec<Diagnostic> {
        crate::rules::test_helpers::run_oxc_check(
            &Check,
            FIXTURE_SPEC,
            std::path::Path::new(path),
            &crate::project::ProjectCtx::for_test_with_framework("playwright"),
            crate::rules::file_ctx::default_static_file_ctx(),
        )
    }

    // `--staged` lists a root `e2e/` spec without a leading `./`.
    #[test]
    fn flags_wait_for_timeout_in_a_root_e2e_dir() {
        let d = run_at("e2e/login.spec.ts");
        assert_eq!(d.len(), 1, "{d:?}");
    }

    // Nothing else marks this spec as a Playwright file.
    #[test]
    fn ignores_a_spec_near_an_e2e_substring() {
        let d = run_at("src/e2e-utils/login.spec.ts");
        assert!(d.is_empty(), "{d:?}");
    }
}
