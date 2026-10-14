//! no-escape-in-jsx-text oxc backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use oxc_ast::ast::JSXAttributeValue;
use regex::Regex;
use std::sync::{Arc, LazyLock};

static RE_ESCAPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\(u[0-9A-Fa-f]{4}|x[0-9A-Fa-f]{2})").expect("valid regex"));

pub struct Check;

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::JSXText, AstType::JSXAttribute]
    }

    fn prefilter(&self) -> Option<&'static [&'static str]> {
        Some(&["\\u", "\\x"])
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        _semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let span = match node.kind() {
            AstKind::JSXText(text) => text.span,
            AstKind::JSXAttribute(attr) => {
                let Some(JSXAttributeValue::StringLiteral(lit)) = &attr.value else {
                    return;
                };
                lit.span
            }
            _ => return,
        };
        let Some(escape) = RE_ESCAPE.find(span.source_text(ctx.source)) else {
            return;
        };

        let (line, column) = byte_offset_to_line_col(ctx.source, span.start as usize);
        diagnostics.push(Diagnostic {
            path: Arc::clone(&ctx.path_arc),
            line,
            column,
            rule_id: super::META.id.into(),
            message: format!(
                "`{}` in JSX text or a quoted attribute renders literally — \
                 write the character itself, or wrap the string in `{{\"…\"}}`.",
                escape.as_str()
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
        crate::rules::test_helpers::run_rule(&Check, src, "t.tsx")
    }

    #[test]
    fn flags_escape_in_jsx_text_and_quoted_attribute() {
        assert_eq!(run(r#"const a = <p>a\u00A0b</p>;"#).len(), 1);
        assert_eq!(run(r#"const a = <p>a\x41b</p>;"#).len(), 1);
        assert_eq!(run(r#"const a = <p title="\u2014" />;"#).len(), 1);
    }

    #[test]
    fn allows_escape_in_expression_container() {
        assert!(run(r#"const a = <p>{"\u00A0"}</p>;"#).is_empty());
        assert!(run(r#"const a = <p title={"\u2014"} />;"#).is_empty());
    }
}
