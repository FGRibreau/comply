//! no-unknown-parameters oxc backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use crate::rules::unknown_type::resolves_to_unknown;
use oxc_ast::ast::BindingPattern;
use std::sync::Arc;

pub struct Check;

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::FormalParameter]
    }

    fn prefilter(&self) -> Option<&'static [&'static str]> {
        Some(&["unknown"])
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let AstKind::FormalParameter(param) = node.kind() else {
            return;
        };
        let Some(annotation) = &param.type_annotation else {
            return;
        };
        if !resolves_to_unknown(&annotation.type_annotation, semantic) {
            return;
        }
        if let BindingPattern::BindingIdentifier(ident) = &param.pattern {
            let allowed =
                ctx.config
                    .required_string_list(super::META.id, "allowed_names", ctx.lang);
            if allowed.iter().any(|name| name == ident.name.as_str()) {
                return;
            }
        }
        let (line, column) = byte_offset_to_line_col(ctx.source, annotation.span.start as usize);
        diagnostics.push(Diagnostic {
            path: Arc::clone(&ctx.path_arc),
            line,
            column,
            rule_id: super::META.id.into(),
            message: "a parameter declared `unknown` accepts an unparsed value — \
                      accept a named domain type and parse input at its boundary."
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
    fn flags_unknown_parameters_except_cause() {
        assert_eq!(run("const f = (x: unknown) => x;").len(), 1);
        assert_eq!(run("function f(x: unknown) {}").len(), 1);
        assert_eq!(run("interface S { put(x: unknown): void }").len(), 1);
        assert!(run("const f = (cause: unknown) => cause;").is_empty());
        assert!(run("const f = (x: string) => x;").is_empty());
    }
}
