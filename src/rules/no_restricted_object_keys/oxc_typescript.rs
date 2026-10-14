//! no-restricted-object-keys oxc backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{AstKind, CheckCtx, OxcCheck};
use oxc_ast::ast::{
    Expression, JSXAttributeName, JSXAttributeValue, ObjectExpression, ObjectPropertyKind,
};
use serde::Deserialize;
use std::borrow::Cow;
use std::sync::Arc;

pub struct Check;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    contexts: Vec<String>,
    keys: Vec<String>,
    #[serde(default)]
    when_value: Vec<String>,
    #[serde(default)]
    unless_value: Vec<String>,
    message: Option<String>,
}

impl Entry {
    fn restricts(&self, context: &str, key: &str, literal: Option<&str>) -> bool {
        let listed = |literals: &[String]| literal.is_some_and(|l| literals.iter().any(|v| v == l));
        let in_scope =
            self.contexts.iter().any(|c| c == context) && self.keys.iter().any(|k| k == key);
        let value_matches = self.when_value.is_empty() || listed(&self.when_value);
        in_scope && value_matches && !listed(&self.unless_value)
    }
}

impl OxcCheck for Check {
    fn run_on_semantic<'a>(
        &self,
        semantic: &'a oxc_semantic::Semantic<'a>,
        ctx: &CheckCtx,
    ) -> Vec<Diagnostic> {
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
        for node in semantic.nodes() {
            let Some((context, value)) = context_value(node.kind()) else {
                continue;
            };
            let named_properties = reached_objects(value)
                .into_iter()
                .flat_map(|object| &object.properties)
                .filter_map(|property| match property {
                    ObjectPropertyKind::ObjectProperty(p) if !p.computed => {
                        Some((p.key.static_name()?, p))
                    }
                    _ => None,
                });
            for (key, property) in named_properties {
                let literal = literal(&property.value, ctx.source);
                for entry in entries
                    .iter()
                    .filter(|e| e.restricts(&context, &key, literal))
                {
                    let (line, column) =
                        byte_offset_to_line_col(ctx.source, property.span.start as usize);
                    diagnostics.push(Diagnostic {
                        path: Arc::clone(&ctx.path_arc),
                        line,
                        column,
                        rule_id: super::META.id.into(),
                        message: entry.message.clone().unwrap_or_else(|| {
                            format!("Key `{key}` is restricted under `{context}`.")
                        }),
                        severity: Severity::Error,
                        span: None,
                    });
                }
            }
        }
        diagnostics
    }
}

fn parse_entry((index, table): (usize, &toml::Table)) -> Entry {
    toml::Value::Table(table.clone())
        .try_into()
        .unwrap_or_else(|err| panic!("[rules.\"{}\"] entries[{index}]: {err}", super::META.id))
}

fn context_value<'a>(kind: AstKind<'a>) -> Option<(Cow<'a, str>, &'a Expression<'a>)> {
    match kind {
        AstKind::ObjectProperty(property) if !property.computed => {
            Some((property.key.static_name()?, &property.value))
        }
        AstKind::JSXAttribute(attribute) => {
            let JSXAttributeName::Identifier(name) = &attribute.name else {
                return None;
            };
            let Some(JSXAttributeValue::ExpressionContainer(container)) = &attribute.value else {
                return None;
            };
            Some((
                Cow::Borrowed(name.name.as_str()),
                container.expression.as_expression()?,
            ))
        }
        _ => None,
    }
}

fn reached_objects<'a>(value: &'a Expression<'a>) -> Vec<&'a ObjectExpression<'a>> {
    let roots: Vec<&ObjectExpression> = match value.get_inner_expression() {
        Expression::ObjectExpression(object) => vec![object],
        Expression::CallExpression(call) => call
            .arguments
            .iter()
            .filter_map(|argument| as_object(argument.as_expression()?))
            .collect(),
        _ => Vec::new(),
    };
    let nested: Vec<&ObjectExpression> = roots
        .iter()
        .flat_map(|object| &object.properties)
        .filter_map(|property| match property {
            ObjectPropertyKind::ObjectProperty(p) => as_object(&p.value),
            ObjectPropertyKind::SpreadProperty(_) => None,
        })
        .collect();
    roots.into_iter().chain(nested).collect()
}

fn as_object<'a>(expression: &'a Expression<'a>) -> Option<&'a ObjectExpression<'a>> {
    match expression.get_inner_expression() {
        Expression::ObjectExpression(object) => Some(object),
        _ => None,
    }
}

/// A number keeps its source text: `0.30` stays `0.30`.
fn literal<'a>(value: &'a Expression<'a>, source: &'a str) -> Option<&'a str> {
    match value.get_inner_expression() {
        Expression::StringLiteral(string) => Some(string.value.as_str()),
        Expression::NumericLiteral(number) => Some(number.span.source_text(source)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::files::Language;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn config_from(toml: &str) -> Config {
        let project_dir = TempDir::new().expect("tempdir");
        fs::write(project_dir.path().join("comply.toml"), toml).expect("write cfg");
        Config::load_from(project_dir.path()).expect("load cfg")
    }

    fn run_with_config(src: &str, toml: &str) -> Vec<Diagnostic> {
        let config = config_from(toml);
        crate::engine::lint_in_memory(Path::new("t.tsx"), Language::Tsx, src, &config, None)
            .into_iter()
            .filter(|d| d.rule_id.as_ref() == super::super::META.id)
            .collect()
    }

    #[test]
    fn flags_key_in_jsx_prop_and_property() {
        let toml = r#"
[[rules.no-restricted-object-keys.entries]]
contexts = ["transition"]
keys = ["stiffness"]
"#;
        let src = r"
const a = <m.div transition={{ stiffness: 300 }} />;
const t = { transition: { stiffness: 300 } };
";
        let lines: Vec<usize> = run_with_config(src, toml).iter().map(|d| d.line).collect();
        assert_eq!(lines, [2, 3]);
    }

    #[test]
    fn unless_value_lets_listed_literal_pass() {
        let toml = r#"
[[rules.no-restricted-object-keys.entries]]
contexts = ["transition"]
keys = ["duration"]
unless_value = ["0"]
"#;
        let src = r"
const a = { transition: { duration: 0.3 } };
const b = { transition: { duration: 0 } };
const c = { transition: { duration: d } };
";
        let lines: Vec<usize> = run_with_config(src, toml).iter().map(|d| d.line).collect();
        assert_eq!(lines, [2, 4]);
    }

    #[test]
    fn when_value_flags_only_listed_literal() {
        let toml = r#"
[[rules.no-restricted-object-keys.entries]]
contexts = ["transition"]
keys = ["type"]
when_value = ["spring"]
"#;
        let src = r#"
const a = { transition: { type: "spring" } };
const b = { transition: { type: "spring" as const } };
const c = { transition: { type: "tween" } };
const d = { transition: { type: kind } };
"#;
        let lines: Vec<usize> = run_with_config(src, toml).iter().map(|d| d.line).collect();
        assert_eq!(lines, [2, 3]);
    }

    #[test]
    fn flags_style_prop_key() {
        let toml = r#"
[[rules.no-restricted-object-keys.entries]]
contexts = ["style"]
keys = ["fontSize"]
"#;
        let src = r"
const a = <p style={{ fontSize: 14 }} />;
";
        let lines: Vec<usize> = run_with_config(src, toml).iter().map(|d| d.line).collect();
        assert_eq!(lines, [2]);
    }

    #[test]
    fn flags_key_in_call_argument_of_context() {
        let toml = r#"
[[rules.no-restricted-object-keys.entries]]
contexts = ["validateSearch"]
keys = ["tab"]
"#;
        let src = r"
const r = { validateSearch: z.object({ tab: z.string() }) };
";
        let lines: Vec<usize> = run_with_config(src, toml).iter().map(|d| d.line).collect();
        assert_eq!(lines, [2]);
    }

    #[test]
    fn reaches_one_nesting_level_only() {
        let toml = r#"
[[rules.no-restricted-object-keys.entries]]
contexts = ["transition"]
keys = ["duration"]
"#;
        let src = r"
const a = <m.div transition={{ opacity: { duration: 0.2 } }} />;
const b = <m.div transition={{ a: { b: { duration: 0.2 } } }} />;
";
        let lines: Vec<usize> = run_with_config(src, toml).iter().map(|d| d.line).collect();
        assert_eq!(lines, [2]);
    }

    #[test]
    fn ignores_keys_outside_contexts_computed_or_spread() {
        let toml = r#"
[[rules.no-restricted-object-keys.entries]]
contexts = ["transition"]
keys = ["stiffness"]
"#;
        let src = r#"
const a = { stiffness: 300 };
const b = <m.div animate={{ stiffness: 300 }} />;
const c = <m.div transition={{ [k]: 1, ["stiffness"]: 1, ...base }} />;
"#;
        assert!(run_with_config(src, toml).is_empty());
    }

    #[test]
    fn no_entries_emits_nothing() {
        let src = r"
const a = <m.div transition={{ stiffness: 300 }} />;
";
        assert!(run_with_config(src, "").is_empty());
    }

    #[test]
    fn finding_shows_entry_message() {
        let toml = r#"
[[rules.no-restricted-object-keys.entries]]
contexts = ["transition"]
keys = ["stiffness"]
message = "Use spring tokens."

[[rules.no-restricted-object-keys.entries]]
contexts = ["transition"]
keys = ["damping"]
"#;
        let src = r"
const a = <m.div transition={{ stiffness: 300, damping: 20 }} />;
";
        let messages: Vec<String> = run_with_config(src, toml)
            .into_iter()
            .map(|d| d.message)
            .collect();
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().any(|m| m == "Use spring tokens."));
        assert!(
            messages
                .iter()
                .any(|m| m.contains("damping") && m.contains("transition"))
        );
    }

    #[test]
    #[should_panic(expected = "entries[0]")]
    fn bad_entry_panics_naming_index() {
        let config = config_from(
            r#"
[[rules.no-restricted-object-keys.entries]]
contexts = ["transition"]
keys = ["type"]
when_values = ["x"]
"#,
        );
        let path = Path::new("t.tsx");
        let src = "const t = {};";
        let ctx = CheckCtx {
            path,
            path_arc: Arc::from(path),
            source: src,
            config: &config,
            project: crate::project::default_static_project_ctx(),
            file: crate::rules::file_ctx::default_static_file_ctx(),
            lang: Language::Tsx,
        };
        crate::oxc_helpers::with_oxc_parse(src, path, |semantic| {
            Check.run_on_semantic(semantic, &ctx)
        });
    }
}
