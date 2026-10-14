//! no-reduce-accumulator-copy oxc backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::{byte_offset_to_line_col, identifier_is_unshadowed_global};
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use oxc_ast::ast::{
    Argument, BindingPattern, CallExpression, Expression, FormalParameters, IdentifierReference,
};
use oxc_span::GetSpan;
use std::sync::Arc;

pub struct Check;

/// The identifier `call` copies, if it is a copy form.
fn copied_identifier<'a, 'b>(
    call: &'b CallExpression<'a>,
    semantic: &'a oxc_semantic::Semantic<'a>,
) -> Option<&'b IdentifierReference<'a>> {
    let Expression::StaticMemberExpression(member) = &call.callee else {
        return None;
    };
    let method = member.property.name.as_str();
    let Expression::Identifier(object) = &member.object else {
        return None;
    };
    if matches!(method, "concat" | "slice") {
        return Some(object);
    }
    let is_global = |name: &str| {
        object.name.as_str() == name && identifier_is_unshadowed_global(object, semantic)
    };
    let copied = match (method, call.arguments.as_slice()) {
        ("assign", [Argument::ObjectExpression(target), source, ..])
            if target.properties.is_empty() && is_global("Object") =>
        {
            source
        }
        ("from", [source, ..]) if is_global("Array") => source,
        _ => return None,
    };
    match copied {
        Argument::Identifier(ident) => Some(ident),
        _ => None,
    }
}

fn function_params<'a>(kind: AstKind<'a>) -> Option<&'a FormalParameters<'a>> {
    match kind {
        AstKind::Function(f) => Some(&f.params),
        AstKind::ArrowFunctionExpression(f) => Some(&f.params),
        _ => None,
    }
}

/// True when `ident` binds a `reduce` callback's first parameter.
fn is_reduce_accumulator<'a>(
    ident: &IdentifierReference<'a>,
    node: &oxc_semantic::AstNode<'a>,
    semantic: &'a oxc_semantic::Semantic<'a>,
) -> bool {
    let Some(symbol) = ident
        .reference_id
        .get()
        .and_then(|reference| semantic.scoping().get_reference(reference).symbol_id())
    else {
        return false;
    };
    let nodes = semantic.nodes();
    let binding_function = nodes.ancestors(node.id()).find(|ancestor| {
        function_params(ancestor.kind())
            .and_then(|params| params.items.first())
            .is_some_and(|param| {
                matches!(&param.pattern, BindingPattern::BindingIdentifier(id)
                    if id.symbol_id.get() == Some(symbol))
            })
    });
    let Some(function) = binding_function else {
        return false;
    };
    let AstKind::CallExpression(parent_call) = nodes.parent_node(function.id()).kind() else {
        return false;
    };
    let Expression::StaticMemberExpression(member) = &parent_call.callee else {
        return false;
    };
    matches!(member.property.name.as_str(), "reduce" | "reduceRight")
        && parent_call
            .arguments
            .first()
            .is_some_and(|callback| callback.span() == function.kind().span())
}

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::CallExpression]
    }

    fn prefilter(&self) -> Option<&'static [&'static str]> {
        Some(&["reduce"])
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let AstKind::CallExpression(call) = node.kind() else {
            return;
        };
        let Some(copied) = copied_identifier(call, semantic) else {
            return;
        };
        if !is_reduce_accumulator(copied, node, semantic) {
            return;
        }

        let (line, column) = byte_offset_to_line_col(ctx.source, call.span.start as usize);
        diagnostics.push(Diagnostic {
            path: Arc::clone(&ctx.path_arc),
            line,
            column,
            rule_id: super::META.id.into(),
            message: format!(
                "`reduce` callback copies its accumulator `{}` on every step — \
                 mutate it in place or use a `for` loop.",
                copied.name
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
        crate::rules::test_helpers::run_rule(&Check, src, "t.ts")
    }

    #[test]
    fn flags_accumulator_copies() {
        assert_eq!(run("xs.reduce((acc, x) => acc.concat(x), []);").len(), 1);
        assert_eq!(
            run("xs.reduce((acc, x) => Object.assign({}, acc, x), {});").len(),
            1
        );
        assert_eq!(run("xs.reduce((acc, x) => Array.from(acc), []);").len(), 1);
        assert_eq!(
            run("xs.reduce(function (acc, x) { return acc.slice(); }, []);").len(),
            1
        );
    }

    #[test]
    fn allows_mutation_and_copies_outside_reduce() {
        assert!(run("xs.reduce((acc, x) => { acc.push(x); return acc; }, []);").is_empty());
        assert!(run("xs.forEach((acc) => acc.slice());").is_empty());
        let shadowed = "xs.reduce((acc, x) => { ys.map((acc) => acc.slice()); return acc; }, []);";
        assert!(run(shadowed).is_empty());
    }
}
