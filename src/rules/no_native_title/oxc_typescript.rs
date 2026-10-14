//! no-native-title oxc backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::{byte_offset_to_line_col, jsx_element_name_is_component};
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use oxc_ast::ast::{JSXAttributeItem, JSXAttributeName, JSXElementName};
use std::sync::Arc;

/// Elements whose `title` names them for assistive technology.
const TITLED_ELEMENTS: &[&str] = &["svg", "iframe"];

pub struct Check;

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::JSXOpeningElement]
    }

    fn prefilter(&self) -> Option<&'static [&'static str]> {
        Some(&["title"])
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        _semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let AstKind::JSXOpeningElement(opening) = node.kind() else {
            return;
        };
        if jsx_element_name_is_component(&opening.name) {
            return;
        }
        if let JSXElementName::Identifier(tag) = &opening.name
            && TITLED_ELEMENTS.contains(&tag.name.as_str())
        {
            return;
        }
        let Some(title) = opening.attributes.iter().find_map(|item| match item {
            JSXAttributeItem::Attribute(attr)
                if matches!(&attr.name, JSXAttributeName::Identifier(name) if name.name == "title") =>
            {
                Some(attr)
            }
            _ => None,
        }) else {
            return;
        };

        let hint = ctx.config.string(super::META.id, "hint", ctx.lang);
        let replacement = if hint.is_empty() {
            String::new()
        } else {
            format!(" — use {hint} instead")
        };
        let (line, column) = byte_offset_to_line_col(ctx.source, title.span.start as usize);
        diagnostics.push(Diagnostic {
            path: Arc::clone(&ctx.path_arc),
            line,
            column,
            rule_id: super::META.id.into(),
            message: format!("Native `title` tooltip on an HTML element{replacement}."),
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
    fn flags_title_on_intrinsic_element() {
        let diagnostics = run(r#"const a = <span title="x" />;"#);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "Native `title` tooltip on an HTML element."
        );
    }

    #[test]
    fn allows_iframe_svg_and_components() {
        assert!(run(r#"const a = <iframe title="x" />;"#).is_empty());
        assert!(run(r#"const a = <svg title="x" />;"#).is_empty());
        assert!(run(r#"const a = <Tooltip title="x" />;"#).is_empty());
    }
}
