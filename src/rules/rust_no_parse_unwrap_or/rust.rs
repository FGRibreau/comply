//! rust-no-parse-unwrap-or backend.
//!
//! Flags `.parse()` chained into an `unwrap_or*` fallback.
//! The chain may pass through `.ok()`. Test code is exempt.

use tree_sitter::Node;

use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::backend::{AstCheck, CheckCtx};
use crate::rules::rust_helpers::is_test_code;

const KINDS: &[&str] = &["call_expression"];

const FALLBACKS: &[&str] = &["unwrap_or", "unwrap_or_default", "unwrap_or_else"];

#[derive(Debug)]
pub struct Check;

impl AstCheck for Check {
    fn interested_kinds(&self) -> Option<&'static [&'static str]> {
        Some(KINDS)
    }

    fn visit_node(
        &self,
        node: Node,
        ctx: &CheckCtx,
        _state: Option<&mut dyn std::any::Any>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let source = ctx.source.as_bytes();
        let Some((fallback, field, mut receiver)) = method_call(node, source) else {
            return;
        };
        if !FALLBACKS.contains(&fallback) {
            return;
        }
        if let Some(("ok", _, inner)) = method_call(receiver, source)
            && has_no_args(receiver)
        {
            receiver = inner;
        }
        let is_bare_parse =
            matches!(method_call(receiver, source), Some(("parse", ..))) && has_no_args(receiver);
        if !is_bare_parse || is_test_code(node, source, ctx) {
            return;
        }
        diagnostics.push(Diagnostic::at_node(
            ctx.path,
            &field,
            "rust-no-parse-unwrap-or",
            format!(
                "`.parse()` failure replaced by `.{fallback}(…)` hides a corrupt value. \
                 Propagate it with `?`, or keep a deliberate fallback with \
                 `// comply-ignore: rust-no-parse-unwrap-or — <reason>`."
            ),
            Severity::Error,
        ));
    }
}

/// Splits `<receiver>.<method>(…)`, turbofish included, into method, its node, receiver.
fn method_call<'t>(call: Node<'t>, source: &'t [u8]) -> Option<(&'t str, Node<'t>, Node<'t>)> {
    if call.kind() != "call_expression" {
        return None;
    }
    let mut function = call.child_by_field_name("function")?;
    if function.kind() == "generic_function" {
        function = function.child_by_field_name("function")?;
    }
    if function.kind() != "field_expression" {
        return None;
    }
    let field = function.child_by_field_name("field")?;
    let receiver = function.child_by_field_name("value")?;
    Some((field.utf8_text(source).ok()?, field, receiver))
}

fn has_no_args(call: Node) -> bool {
    call.child_by_field_name("arguments")
        .is_some_and(|args| args.named_child_count() == 0)
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

    fn run(source: &str) -> Vec<Diagnostic> {
        crate::rules::test_helpers::run_rule(&Check, source, "src/t.rs")
    }

    #[test]
    fn flags_parse_fallbacks() {
        let src = r"fn f(s: &str) {
    let a = s.parse::<u32>().unwrap_or(0);
    let b: u32 = s.parse().unwrap_or_default();
    let c: u32 = s.parse().ok().unwrap_or(1);
    let d: u32 = s.parse().unwrap_or_else(|_| 1);
}";
        assert_eq!(run(src).len(), 4);
    }

    #[test]
    fn allows_propagation_and_other_calls() {
        let src = r"fn f(s: &str, p: P) -> Result<(), E> {
    let a: u32 = s.parse()?;
    let b = s.find('x').unwrap_or(0);
    let c = p.parse(input).unwrap_or(d);
    Ok(())
}";
        assert!(run(src).is_empty());
    }
}
