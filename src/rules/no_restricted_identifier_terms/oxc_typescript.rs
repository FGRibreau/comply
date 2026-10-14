//! no-restricted-identifier-terms oxc backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{CheckCtx, OxcCheck};
use oxc_ast::AstKind;
use oxc_ast::ast::PropertyKey;
use oxc_semantic::{AstNode, AstNodes, Semantic};
use oxc_span::GetSpan;
use std::borrow::Cow;
use std::sync::Arc;

pub struct Check;

impl OxcCheck for Check {
    fn run_on_semantic<'a>(&self, semantic: &'a Semantic<'a>, ctx: &CheckCtx) -> Vec<Diagnostic> {
        let terms: Vec<String> = ctx
            .config
            .required_string_list(super::META.id, "terms", ctx.lang)
            .iter()
            .map(|term| term.to_lowercase())
            .collect();
        if terms.is_empty() {
            return Vec::new();
        }
        let nodes = semantic.nodes();
        nodes
            .iter()
            .filter_map(|node| {
                let (name, offset) = declared_name(node, nodes)?;
                let lowered = name.to_lowercase();
                let term = terms.iter().find(|term| lowered.contains(term.as_str()))?;
                let (line, column) = byte_offset_to_line_col(ctx.source, offset as usize);
                Some(Diagnostic {
                    path: Arc::clone(&ctx.path_arc),
                    line,
                    column,
                    rule_id: super::META.id.into(),
                    message: format!("Name '{name}' contains the restricted term '{term}'."),
                    severity: Severity::Error,
                    span: None,
                })
            })
            .collect()
    }
}

/// Name and offset this node declares; imports declare none.
fn declared_name<'a>(node: &AstNode<'a>, nodes: &AstNodes<'a>) -> Option<(Cow<'a, str>, u32)> {
    let key: &PropertyKey<'a> = match node.kind() {
        AstKind::BindingIdentifier(id) => {
            let imported = matches!(
                nodes.parent_kind(node.id()),
                AstKind::ImportSpecifier(_)
                    | AstKind::ImportDefaultSpecifier(_)
                    | AstKind::ImportNamespaceSpecifier(_)
            );
            return (!imported).then(|| (Cow::Borrowed(id.name.as_str()), id.span.start));
        }
        AstKind::TSEnumMember(member) => {
            return Some((
                Cow::Borrowed(member.id.static_name().as_str()),
                member.span.start,
            ));
        }
        AstKind::PropertyDefinition(property) => &property.key,
        AstKind::AccessorProperty(property) => &property.key,
        AstKind::MethodDefinition(method) => &method.key,
        AstKind::TSPropertySignature(signature) => &signature.key,
        AstKind::TSMethodSignature(signature) => &signature.key,
        _ => return None,
    };
    Some((key.name()?, key.span().start))
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::diagnostic::Diagnostic;
    use crate::files::Language;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// Lint `src` as `t.ts` through the engine under `toml`.
    fn run_with_config(src: &str, toml: &str) -> Vec<Diagnostic> {
        let project_dir = TempDir::new().expect("tempdir");
        fs::write(project_dir.path().join("comply.toml"), toml).expect("write cfg");
        let config = Config::load_from(project_dir.path()).expect("load cfg");
        crate::engine::lint_in_memory(Path::new("t.ts"), Language::TypeScript, src, &config, None)
            .into_iter()
            .filter(|d| d.rule_id.as_ref() == super::super::META.id)
            .collect()
    }

    const SHAPE: &str = r#"
[rules.no-restricted-identifier-terms]
terms = ["shape"]
"#;

    #[test]
    fn flags_declared_names_containing_term() {
        let findings = run_with_config(
            "const userShape = 1; interface ShapeProps {} function parseShape() {} class C { #shapeKey = 1 }",
            SHAPE,
        );
        assert_eq!(findings.len(), 4, "{findings:?}");
        assert!(
            findings.iter().all(|d| d.message.contains("'shape'")),
            "{findings:?}"
        );
    }

    #[test]
    fn ignores_imported_and_referenced_names() {
        let src = r#"import { Shape } from "./s"; const x: Shape = makeShape(); x.shape;"#;
        let findings = run_with_config(src, SHAPE);
        assert!(findings.is_empty(), "{findings:?}");
        let findings = run_with_config("class Shape {}", SHAPE);
        assert_eq!(findings.len(), 1, "{findings:?}");
    }

    #[test]
    fn empty_terms_emits_nothing() {
        let findings = run_with_config("const userShape = 1;", "");
        assert!(findings.is_empty(), "{findings:?}");
    }
}
