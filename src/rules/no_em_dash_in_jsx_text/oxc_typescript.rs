//! no-em-dash-in-jsx-text oxc backend for TSX / JS(X).

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{AstKind, CheckCtx, OxcCheck};
use oxc_ast::ast::{JSXAttributeName, JSXAttributeValue, JSXElementName};
use oxc_span::Span;
use oxc_syntax::xml_entities::XML_ENTITIES;
use regex::Regex;
use serde::Deserialize;
use std::sync::Arc;

/// Elements whose children are technical content, not prose. A dash inside
/// `<code>` / `<pre>` is legitimate (a minus sign, a CLI flag, …).
const TECHNICAL_ELEMENTS: &[&str] = &["code", "pre"];

pub struct Check;

/// One `[[rules.no-em-dash-in-jsx-text.entries]]` table.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryToml {
    regex: String,
    message: String,
}

impl OxcCheck for Check {
    fn run_on_semantic<'a>(
        &self,
        semantic: &'a oxc_semantic::Semantic<'a>,
        ctx: &CheckCtx,
    ) -> Vec<Diagnostic> {
        let id = super::META.id;
        let needles = with_entities(ctx.config.required_string_list(id, "characters", ctx.lang));
        let entries: Vec<(Regex, String)> = ctx
            .config
            .table_list(id, "entries", ctx.lang)
            .into_iter()
            .enumerate()
            .map(|(index, table)| compile_entry(index, table))
            .collect();
        let all_strings = match ctx.config.string(id, "scope", ctx.lang) {
            "jsx-copy" => false,
            "all-strings" => true,
            other => panic!(
                "config key `[rules.\"{id}\"] scope` must be \"jsx-copy\" or \"all-strings\", got {other:?}"
            ),
        };
        let mut diagnostics = Vec::new();
        let mut scan = |start: usize, end: usize, is_jsx_text: bool| {
            let text = &ctx.source[start..end];
            let standalone = is_jsx_text && super::is_standalone_needle(&needles, text);
            if let Some((rel, needle)) = super::first_needle_offset(&needles, text)
                && !standalone
            {
                let message = format!(
                    "`{needle}` in user-facing copy reads as AI-generated prose \u{2014} \
                     remove it or rewrite the sentence."
                );
                diagnostics.push(report(ctx, start + rel, message));
            }
            for (regex, message) in &entries {
                if let Some(found) = regex.find(text) {
                    diagnostics.push(report(ctx, start + found.start(), message.clone()));
                }
            }
        };
        for node in semantic.nodes().iter() {
            match node.kind() {
                AstKind::JSXText(text) => {
                    if !inside_technical_element(node.id(), semantic) {
                        scan(text.span.start as usize, text.span.end as usize, true);
                    }
                }
                // Under `all-strings` the attribute's value is visited as a `StringLiteral`.
                AstKind::JSXAttribute(attr) if !all_strings => {
                    let JSXAttributeName::Identifier(name) = &attr.name else {
                        continue;
                    };
                    if let Some(JSXAttributeValue::StringLiteral(lit)) = &attr.value
                        && super::COPY_ATTRS.contains(&name.name.as_str())
                    {
                        let (start, end) = unquoted(lit.span);
                        scan(start, end, false);
                    }
                }
                AstKind::StringLiteral(lit) if all_strings => {
                    let (start, end) = unquoted(lit.span);
                    scan(start, end, false);
                }
                AstKind::TemplateLiteral(template) if all_strings => {
                    for quasi in &template.quasis {
                        scan(quasi.span.start as usize, quasi.span.end as usize, false);
                    }
                }
                _ => {}
            }
        }
        diagnostics
    }
}

/// `characters` plus the named HTML entities rendering one.
fn with_entities(characters: Vec<String>) -> Vec<String> {
    let entities: Vec<String> = XML_ENTITIES
        .entries()
        .filter(|(_, glyph)| characters.iter().any(|c| c.chars().eq([**glyph])))
        .map(|(name, _)| format!("&{name};"))
        .collect();
    characters.into_iter().chain(entities).collect()
}

fn compile_entry(index: usize, table: &toml::Table) -> (Regex, String) {
    let at = format!("[rules.\"{}\"] entries[{index}]", super::META.id);
    let entry: EntryToml = toml::Value::Table(table.clone())
        .try_into()
        .unwrap_or_else(|err| panic!("{at}: {err}"));
    let regex = Regex::new(&entry.regex).unwrap_or_else(|err| panic!("{at}: {err}"));
    (regex, entry.message)
}

fn unquoted(span: Span) -> (usize, usize) {
    (span.start as usize + 1, span.end as usize - 1)
}

fn report(ctx: &CheckCtx, offset: usize, message: String) -> Diagnostic {
    let (line, column) = byte_offset_to_line_col(ctx.source, offset);
    Diagnostic {
        path: Arc::clone(&ctx.path_arc),
        line,
        column,
        rule_id: super::META.id.into(),
        message,
        severity: Severity::Error,
        span: None,
    }
}

/// True when `node_id` lives inside a `<code>` or `<pre>` element.
fn inside_technical_element(node_id: oxc_semantic::NodeId, semantic: &oxc_semantic::Semantic) -> bool {
    for ancestor in semantic.nodes().ancestors(node_id) {
        if let AstKind::JSXElement(element) = ancestor.kind()
            && let JSXElementName::Identifier(tag) = &element.opening_element.name
            && TECHNICAL_ELEMENTS.contains(&tag.name.as_str())
        {
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
        crate::rules::test_helpers::run_oxc_check(self, src, path, project, file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Diagnostic;

    fn run(source: &str) -> Vec<Diagnostic> {
        crate::rules::test_helpers::run_rule(&Check, source, "Page.tsx")
    }

    /// Run the rule under `[rules.no-em-dash-in-jsx-text]` holding `rule_toml`.
    fn run_configured(rule_toml: &str, source: &str) -> Vec<Diagnostic> {
        let config_dir = tempfile::TempDir::new().expect("tempdir");
        std::fs::write(
            config_dir.path().join("comply.toml"),
            format!("[rules.no-em-dash-in-jsx-text]\n{rule_toml}\n"),
        )
        .expect("write cfg");
        let config = crate::config::Config::load_from(config_dir.path()).expect("load cfg");
        let path = std::path::Path::new("Page.tsx");
        let allocator = oxc_allocator::Allocator::default();
        let source_type = crate::oxc_helpers::source_type_for_path(path);
        let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();
        let semantic = oxc_semantic::SemanticBuilder::new()
            .build(&parsed.program)
            .semantic;
        let ctx = CheckCtx {
            path,
            path_arc: Arc::from(path),
            source,
            config: &config,
            project: crate::project::default_static_project_ctx(),
            file: crate::rules::file_ctx::default_static_file_ctx(),
            lang: crate::files::Language::Tsx,
        };
        Check.run_on_semantic(&semantic, &ctx)
    }

    #[test]
    fn configured_middle_dot_and_its_entity_are_flagged_in_jsx_text() {
        let characters = "characters = [\"\u{2014}\", \"\u{00B7}\"]";
        assert_eq!(run_configured(characters, "<p>a \u{00B7} b</p>").len(), 1);
        assert_eq!(run_configured(characters, "<p>a &middot; b</p>").len(), 1);
    }

    #[test]
    fn scope_decides_string_literal_and_template_piece() {
        let characters = "characters = [\"\u{2014}\", \"\u{00B7}\"]";
        let wide = format!("{characters}\nscope = \"all-strings\"");
        let literal = "const s = \"a \u{00B7} b\";";
        let template = "const t = `x \u{2014} ${y}`;";
        assert_eq!(run_configured(&wide, literal).len(), 1);
        assert_eq!(run_configured(&wide, template).len(), 1);
        assert!(run_configured(characters, literal).is_empty());
        assert!(run_configured(characters, template).is_empty());
    }

    #[test]
    fn entry_regex_flags_rating_with_its_message() {
        let entry = "[[rules.no-em-dash-in-jsx-text.entries]]\nregex = '(^|\\d|\\s)/\\s?5(\\D|$)'\nmessage = \"formatScore\"";
        let hits = run_configured(entry, "<p>Note 4/5</p>");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message, "formatScore");
        assert!(run_configured(entry, "<p>Note 4/50</p>").is_empty());
    }

    #[test]
    #[should_panic(expected = "entries[0]")]
    fn invalid_entry_regex_panics_naming_the_entry() {
        let entry =
            "[[rules.no-em-dash-in-jsx-text.entries]]\nregex = \"(\"\nmessage = \"formatScore\"";
        run_configured(entry, "<p>Note 4/5</p>");
    }

    #[test]
    fn all_strings_reports_copy_attribute_once() {
        let wide = "scope = \"all-strings\"";
        assert_eq!(
            run_configured(wide, "<img alt=\"A \u{2014} B\" />").len(),
            1
        );
    }

    #[test]
    fn default_characters_count_their_entity() {
        assert_eq!(run("<p>a &mdash; b</p>").len(), 1);
    }

    #[test]
    fn flags_em_dash_in_text() {
        assert_eq!(run("<p>Save time \u{2014} automate.</p>").len(), 1);
    }

    #[test]
    fn flags_en_dash_in_text() {
        assert_eq!(run("<p>First \u{2013} last</p>").len(), 1);
    }

    #[test]
    fn flags_em_dash_in_alt_attribute() {
        assert_eq!(run("<img alt=\"A \u{2014} B\" />").len(), 1);
    }

    #[test]
    fn flags_en_dash_in_placeholder_attribute() {
        assert_eq!(run("<input placeholder=\"First \u{2013} last\" />").len(), 1);
    }

    #[test]
    fn flags_title_and_label_and_aria_label() {
        assert_eq!(run("<a title=\"Go \u{2014} home\">x</a>").len(), 1);
        assert_eq!(run("<button label=\"Save \u{2014} now\" />").len(), 1);
        assert_eq!(run("<div aria-label=\"Menu \u{2014} open\" />").len(), 1);
    }

    #[test]
    fn allows_plain_hyphen_in_text() {
        assert!(run("<p>Save time - automate.</p>").is_empty());
    }

    #[test]
    fn allows_numeric_range_in_text() {
        assert!(run("<span>9\u{2013}5</span>").is_empty());
        assert!(run("<span>2020\u{2013}2024</span>").is_empty());
    }

    #[test]
    fn ignores_dash_inside_code_element() {
        assert!(run("<code>a \u{2014} b</code>").is_empty());
    }

    #[test]
    fn ignores_dash_inside_pre_element() {
        assert!(run("<pre>a \u{2014} b</pre>").is_empty());
    }

    #[test]
    fn ignores_dash_inside_nested_code() {
        assert!(run("<p><code>git log \u{2014}oneline</code></p>").is_empty());
    }

    #[test]
    fn ignores_non_copy_attribute() {
        assert!(run("<div className=\"a\u{2014}b\" />").is_empty());
        assert!(run("<div id=\"a\u{2014}b\" />").is_empty());
        assert!(run("<div data-key=\"a\u{2014}b\" />").is_empty());
    }

    #[test]
    fn ignores_dash_in_expression_attribute() {
        // `title={label}` is an expression container, not a string literal —
        // comply can't see the runtime value, so it must not fire.
        assert!(run("<div title={label} />").is_empty());
    }

    #[test]
    fn ignores_dash_in_arbitrary_expression_child() {
        // `{label}` is an expression, not JSXText copy.
        assert!(run("<p>{label}</p>").is_empty());
    }

    #[test]
    fn ignores_dash_in_code_string_literal() {
        // A string literal that is plain code (not a copy attribute) is invisible
        // to this rule: it only inspects JSXText and copy attributes.
        assert!(run("const s = \"a \u{2014} b\";").is_empty());
    }

    #[test]
    fn ignores_standalone_em_dash_placeholder() {
        // A JSXText node that is entirely a single dash is an empty-value
        // placeholder glyph, not prose.
        assert!(run("<span>\u{2014}</span>").is_empty());
        assert!(run("<td>\u{2013}</td>").is_empty());
        assert!(run("<span className=\"text-muted-foreground\">\u{2014}</span>").is_empty());
    }

    #[test]
    fn still_flags_standalone_dash_followed_by_prose() {
        assert_eq!(run("<span>\u{2014} Foo</span>").len(), 1);
    }

    #[test]
    fn column_points_at_the_dash() {
        let d = run("<p>Save \u{2014} now</p>");
        assert_eq!(d.len(), 1);
        // "<p>Save " is 8 chars before the dash on line 1 → column 9.
        assert_eq!(d[0].line, 1);
        assert_eq!(d[0].column, 9);
    }
}
