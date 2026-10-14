//! rust-tracing-structured-fields backend.
//!
//! The message is the first whole string literal argument.
//! Fields and `target: "…"` are never whole literals.

use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::backend::{AstCheck, CheckCtx};
use crate::rules::rust_helpers::{
    file_binds_name_to_module, macro_body, split_top_level_args, string_literal_content,
};

const KINDS: &[&str] = &["macro_invocation"];

const EVENT_MACROS: &[&str] = &["trace", "debug", "info", "warn", "error", "event"];

#[derive(Debug)]
pub struct Check;

impl AstCheck for Check {
    fn interested_kinds(&self) -> Option<&'static [&'static str]> {
        Some(KINDS)
    }

    fn visit_node(
        &self,
        node: tree_sitter::Node,
        ctx: &CheckCtx,
        _state: Option<&mut dyn std::any::Any>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let source = ctx.source.as_bytes();
        let Some(Ok(path)) = node
            .child_by_field_name("macro")
            .map(|m| m.utf8_text(source))
        else {
            return;
        };
        let segments: Vec<&str> = path.trim_start_matches("::").split("::").collect();
        let Some((&name, module)) = segments.split_last() else {
            return;
        };
        if !EVENT_MACROS.contains(&name) {
            return;
        }
        let is_tracing = match module {
            [] => file_binds_name_to_module(node, source, name, |m| {
                m.first().is_some_and(|root| root == "tracing")
            }),
            [root, ..] => *root == "tracing",
        };
        if !is_tracing {
            return;
        }
        let Some(body) = node.utf8_text(source).ok().and_then(macro_body) else {
            return;
        };
        let message = split_top_level_args(body)
            .into_iter()
            .find_map(|arg| string_literal_content(arg.trim()));
        if !message.is_some_and(|m| has_placeholder(&m)) {
            return;
        }
        diagnostics.push(Diagnostic::at_node(
            ctx.path,
            &node,
            "rust-tracing-structured-fields",
            format!(
                "`{name}!` interpolates values into its message. \
                 Pass them as structured fields: `{name}!(error = %e, \"failed\")`."
            ),
            Severity::Error,
        ));
    }
}

/// True if `fmt` holds a `{…}` placeholder; `{{` is a literal brace.
fn has_placeholder(fmt: &str) -> bool {
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c == '{' && chars.next() != Some('{') {
            return true;
        }
    }
    false
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
        crate::rules::test_helpers::run_rule(&Check, source, "t.rs")
    }

    #[test]
    fn flags_interpolated_messages() {
        let src = r#"use tracing::{info, warn};
fn f() {
    warn!("failed {e}");
    info!("x {}", y);
    tracing::event!(Level::WARN, "x {:?}", y);
}"#;
        assert_eq!(run(src).len(), 3);
    }

    #[test]
    fn allows_fields_escaped_braces_and_non_tracing() {
        let src = r#"use tracing::{info, warn};
fn f() {
    warn!(error = %e, "failed");
    info!("literal {{braces}}");
    tracing::info!(target: "x {y}", name = "a {b}", "msg");
    log::warn!("x {e}");
}"#;
        assert!(run(src).is_empty());
        assert!(run("fn f() { warn!(\"x {e}\"); }").is_empty());
    }
}
