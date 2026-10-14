//! no-restricted-properties oxc backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{CheckCtx, OxcCheck};
use oxc_ast::AstKind;
use oxc_ast::ast::{BindingPattern, Expression};
use oxc_semantic::Semantic;
use serde::Deserialize;
use std::borrow::Cow;
use std::sync::Arc;

pub struct Check;

impl OxcCheck for Check {
    fn run_on_semantic<'a>(&self, semantic: &'a Semantic<'a>, ctx: &CheckCtx) -> Vec<Diagnostic> {
        let entries: Vec<Entry> = ctx
            .config
            .table_list(super::META.id, "entries", ctx.lang)
            .into_iter()
            .enumerate()
            .map(parse_entry)
            .collect();
        if entries.is_empty() {
            return Vec::new();
        }
        let mut diagnostics = Vec::new();
        for (object, property, offset) in
            semantic.nodes().iter().flat_map(|node| reads(node.kind()))
        {
            for entry in entries
                .iter()
                .filter(|entry| entry.object == object && entry.property == property)
            {
                let (line, column) = byte_offset_to_line_col(ctx.source, offset as usize);
                diagnostics.push(Diagnostic {
                    path: Arc::clone(&ctx.path_arc),
                    line,
                    column,
                    rule_id: super::META.id.into(),
                    message: format!("`{object}.{property}` is restricted: {}", entry.message),
                    severity: Severity::Error,
                    span: None,
                });
            }
        }
        diagnostics
    }
}

/// Each `(object, property, offset)` this node reads.
fn reads<'a>(kind: AstKind<'a>) -> Vec<(&'a str, Cow<'a, str>, u32)> {
    if let Some(member) = kind.as_member_expression_kind() {
        return last_segment(member.object())
            .zip(member.static_property_info())
            .map(|(object, (span, property))| (object, Cow::Borrowed(property), span.start))
            .into_iter()
            .collect();
    }
    let AstKind::VariableDeclarator(declarator) = kind else {
        return Vec::new();
    };
    let BindingPattern::ObjectPattern(pattern) = &declarator.id else {
        return Vec::new();
    };
    let Some(object) = declarator.init.as_ref().and_then(last_segment) else {
        return Vec::new();
    };
    pattern
        .properties
        .iter()
        .filter_map(|property| Some((object, property.key.static_name()?, property.span.start)))
        .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    object: String,
    property: String,
    message: String,
}

/// `navigator` for `navigator`, `window.navigator` or `window["navigator"]`.
fn last_segment<'a>(expression: &Expression<'a>) -> Option<&'a str> {
    match expression.without_parentheses() {
        Expression::Identifier(identifier) => Some(identifier.name.as_str()),
        other => other.as_member_expression()?.static_property_name(),
    }
}

fn parse_entry((index, table): (usize, &toml::Table)) -> Entry {
    toml::Value::Table(table.clone())
        .try_into()
        .unwrap_or_else(|err| panic!("[rules.\"{}\"] entries[{index}]: {err}", super::META.id))
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

    const CLIPBOARD: &str = r#"
[[rules.no-restricted-properties.entries]]
object = "navigator"
property = "clipboard"
message = "Use CopyButton."
"#;

    #[test]
    fn flags_member_access_with_message() {
        let findings = run_with_config("navigator.clipboard.writeText(x);", CLIPBOARD);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(
            findings[0].message.contains("CopyButton"),
            "{}",
            findings[0].message
        );
        let count = |src: &str| run_with_config(src, CLIPBOARD).len();
        assert_eq!(count(r#"navigator["clipboard"].writeText(x);"#), 1);
        assert_eq!(count("navigator?.clipboard;"), 1);
    }

    #[test]
    fn flags_destructuring() {
        let count = |src: &str| run_with_config(src, CLIPBOARD).len();
        assert_eq!(count("const { clipboard } = navigator;"), 1);
        assert_eq!(count("const { clipboard: c } = navigator;"), 1);
    }

    #[test]
    fn matches_object_name_not_any_receiver() {
        let count = |src: &str| run_with_config(src, CLIPBOARD).len();
        assert_eq!(count("window.navigator.clipboard;"), 1);
        assert_eq!(count("foo.clipboard;"), 0);
        assert_eq!(count("navigator.geolocation;"), 0);
        assert_eq!(count("navigator[key];"), 0);
    }

    #[test]
    fn no_entries_emits_nothing() {
        let findings = run_with_config("navigator.clipboard;", "");
        assert!(findings.is_empty(), "{findings:?}");
    }
}
