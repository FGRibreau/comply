//! no-unsafe-dictionary-type oxc backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use oxc_ast::ast::{TSType, TSTypeName};
use oxc_span::Span;
use std::sync::Arc;

/// True when `ty` admits any value: `unknown`, `any`, `object` or `{}`.
fn is_untyped(ty: &TSType) -> bool {
    match ty {
        TSType::TSUnknownKeyword(_) | TSType::TSAnyKeyword(_) | TSType::TSObjectKeyword(_) => true,
        TSType::TSTypeLiteral(literal) => literal.members.is_empty(),
        _ => false,
    }
}

/// The span of a dictionary with an untyped value.
fn unsafe_dictionary(kind: AstKind) -> Option<Span> {
    if let AstKind::TSIndexSignature(signature) = kind {
        return is_untyped(&signature.type_annotation.type_annotation).then_some(signature.span);
    }
    let AstKind::TSTypeReference(reference) = kind else {
        return None;
    };
    let TSTypeName::IdentifierReference(name) = &reference.type_name else {
        return None;
    };
    let arguments = reference.type_arguments.as_ref()?;
    let [_, value] = arguments.params.as_slice() else {
        return None;
    };
    (name.name == "Record" && is_untyped(value)).then_some(reference.span)
}

pub struct Check;

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::TSTypeReference, AstType::TSIndexSignature]
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        _semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let Some(span) = unsafe_dictionary(node.kind()) else {
            return;
        };
        let (line, column) = byte_offset_to_line_col(ctx.source, span.start as usize);
        diagnostics.push(Diagnostic {
            path: Arc::clone(&ctx.path_arc),
            line,
            column,
            rule_id: super::META.id.into(),
            message: "this dictionary type leaves every value untyped — \
                      give it a named value type, or model the known keys."
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
    fn flags_unsafe_dictionary_values() {
        assert_eq!(run("type A = Record<string, unknown>;").len(), 1);
        assert_eq!(run("type A = Record<string, {}>;").len(), 1);
        assert_eq!(run("type A = { [k: string]: any };").len(), 1);
        assert!(run("type A = Record<string, number>;").is_empty());
        assert!(run("type A = Record<string, { a: number }>;").is_empty());
    }
}
