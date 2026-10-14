//! A directive spans from `add_header` to the line ending `;`.
//! A `;` mid-line is ignored, so quoted CSP values pass.

use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::backend::{CheckCtx, TextCheck};

#[derive(Debug)]
pub struct Check;

impl TextCheck for Check {
    fn prefilter(&self) -> Option<&'static [&'static str]> {
        Some(&["add_header"])
    }

    fn check(&self, ctx: &CheckCtx) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let mut open_line: Option<usize> = None;
        let lines = (1..)
            .zip(ctx.source.lines().map(str::trim))
            .filter(|(_, text)| !text.starts_with('#'));
        for (line, text) in lines {
            let opens = text
                .strip_prefix("add_header")
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace));
            if open_line.is_none() && opens {
                open_line = Some(line);
            }
            let (Some(start), Some(body)) = (open_line, text.strip_suffix(';')) else {
                continue;
            };
            open_line = None;
            if body.split_whitespace().last() != Some("always") {
                diagnostics.push(Diagnostic {
                    path: std::sync::Arc::clone(&ctx.path_arc),
                    line: start,
                    column: 1,
                    rule_id: super::META.id.into(),
                    message: "`add_header` without `always` — nginx drops this header on \
                              4xx/5xx responses."
                        .into(),
                    severity: Severity::Error,
                    span: None,
                });
            }
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn run(source: &str) -> Vec<Diagnostic> {
        Check.check(&CheckCtx::for_test(Path::new("nginx.conf"), source))
    }

    #[test]
    fn flags_add_header_without_always() {
        let diagnostics =
            run("add_header X-Frame-Options DENY;\nadd_header X-Frame-Options DENY always;\n");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 1);
        assert_eq!(diagnostics[0].rule_id, "nginx-add-header-always");
    }

    #[test]
    fn reports_multiline_directive_once_on_first_line() {
        let diagnostics = run("server {\n  add_header\n    X-Frame-Options\n    DENY;\n}\n");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 2);
    }

    #[test]
    fn ignores_comment_and_quoted_semicolons() {
        let source = "# add_header X DENY;\n\
                      add_header_inherit merge;\n\
                      add_header Content-Security-Policy \"default-src 'self'; img-src data:\" always;\n";
        assert!(run(source).is_empty());
    }
}
