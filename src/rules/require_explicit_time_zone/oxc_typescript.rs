//! require-explicit-time-zone oxc backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::{byte_offset_to_line_col, identifier_is_unshadowed_global};
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use oxc_ast::ast::{Argument, Expression, ObjectPropertyKind};
use oxc_span::Span;
use std::sync::Arc;

pub struct Check;

fn is_global<'a>(
    expr: &Expression<'a>,
    name: &str,
    semantic: &'a oxc_semantic::Semantic<'a>,
) -> bool {
    matches!(expr, Expression::Identifier(ident)
        if ident.name.as_str() == name && identifier_is_unshadowed_global(ident, semantic))
}

/// True when the second argument, the options, lacks `timeZone`.
/// Spread or non-literal options hide their keys, so they pass.
fn lacks_time_zone(arguments: &[Argument]) -> bool {
    if arguments
        .iter()
        .any(|arg| matches!(arg, Argument::SpreadElement(_)))
    {
        return false;
    }
    match arguments.get(1) {
        None => true,
        Some(Argument::ObjectExpression(options)) => options.properties.iter().all(|prop| {
            matches!(prop, ObjectPropertyKind::ObjectProperty(pair)
                if pair.key.static_name().is_none_or(|key| key != "timeZone"))
        }),
        Some(_) => false,
    }
}

fn unzoned_formatting_span<'a>(
    node: &oxc_semantic::AstNode<'a>,
    semantic: &'a oxc_semantic::Semantic<'a>,
) -> Option<Span> {
    match node.kind() {
        AstKind::CallExpression(call) => {
            let Expression::StaticMemberExpression(member) = &call.callee else {
                return None;
            };
            let formats_date = match member.property.name.as_str() {
                "toLocaleDateString" | "toLocaleTimeString" => true,
                "toLocaleString" => matches!(member.object.without_parentheses(),
                    Expression::NewExpression(new) if is_global(&new.callee, "Date", semantic)),
                _ => false,
            };
            (formats_date && lacks_time_zone(&call.arguments)).then_some(call.span)
        }
        AstKind::NewExpression(new) => {
            let Expression::StaticMemberExpression(member) = &new.callee else {
                return None;
            };
            (member.property.name.as_str() == "DateTimeFormat"
                && is_global(&member.object, "Intl", semantic)
                && lacks_time_zone(&new.arguments))
            .then_some(new.span)
        }
        _ => None,
    }
}

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::CallExpression, AstType::NewExpression]
    }

    fn prefilter(&self) -> Option<&'static [&'static str]> {
        Some(&["toLocale", "DateTimeFormat"])
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let Some(span) = unzoned_formatting_span(node, semantic) else {
            return;
        };
        let (line, column) = byte_offset_to_line_col(ctx.source, span.start as usize);
        diagnostics.push(Diagnostic {
            path: Arc::clone(&ctx.path_arc),
            line,
            column,
            rule_id: super::META.id.into(),
            message: "Date formatting without `timeZone` follows the host's time zone — \
                      pass `{ timeZone: \"…\" }`."
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
    fn flags_date_formatting_without_time_zone() {
        assert_eq!(run(r#"d.toLocaleDateString("fr");"#).len(), 1);
        assert_eq!(run("d.toLocaleTimeString();").len(), 1);
        assert_eq!(run("new Date().toLocaleString();").len(), 1);
        assert_eq!(
            run(r#"new Intl.DateTimeFormat("fr", { month: "long" });"#).len(),
            1
        );
    }

    #[test]
    fn allows_time_zone_numbers_and_opaque_options() {
        assert!(run(r#"d.toLocaleDateString("fr", { timeZone: "Europe/Paris" });"#).is_empty());
        assert!(run(r#"new Intl.DateTimeFormat("fr", { timeZone });"#).is_empty());
        assert!(run("n.toLocaleString();").is_empty());
        assert!(run(r#"d.toLocaleDateString("fr", opts);"#).is_empty());
        assert!(run(r#"d.toLocaleDateString("fr", { ...base });"#).is_empty());
    }
}
