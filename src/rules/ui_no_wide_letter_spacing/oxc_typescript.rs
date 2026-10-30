//! OxcCheck backend for ui-no-wide-letter-spacing.
//!
//! Flags inline `letterSpacing` string values in `em` above 0.05 inside
//! JSX `style` attributes, unless the same style sets
//! `textTransform: 'uppercase'`.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use std::sync::Arc;

pub struct Check;

fn parse_em(raw: &str) -> Option<f64> {
    let cleaned = raw.trim_matches(|c| c == '"' || c == '\'').trim();
    let stripped = cleaned.strip_suffix("em")?;
    if stripped.ends_with('r') {
        return None;
    }
    stripped.trim().parse::<f64>().ok()
}

/// True when the style object sets `textTransform: 'uppercase'`.
fn is_uppercase_style(style: &oxc_ast::ast::ObjectExpression) -> bool {
    use oxc_ast::ast::{Expression, ObjectPropertyKind, PropertyKey};
    style.properties.iter().any(|prop| {
        let ObjectPropertyKind::ObjectProperty(pair) = prop else {
            return false;
        };
        let key = match &pair.key {
            PropertyKey::StaticIdentifier(id) => id.name.as_str(),
            PropertyKey::StringLiteral(s) => s.value.as_str(),
            _ => return false,
        };
        key == "textTransform"
            && matches!(&pair.value, Expression::StringLiteral(val) if val.value == "uppercase")
    })
}

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::JSXOpeningElement]
    }

    fn prefilter(&self) -> Option<&'static [&'static str]> {
        Some(&["letterSpacing"])
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        _semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        use oxc_ast::ast::*;

        let AstKind::JSXOpeningElement(el) = node.kind() else {
            return;
        };

        // Find the `style` attribute
        for item in el.attributes.iter() {
            let JSXAttributeItem::Attribute(attr) = item else {
                continue;
            };
            let JSXAttributeName::Identifier(id) = &attr.name else {
                continue;
            };
            if id.name.as_str() != "style" {
                continue;
            }

            // style={{ letterSpacing: '0.1em' }}
            let Some(JSXAttributeValue::ExpressionContainer(container)) = &attr.value else {
                continue;
            };
            let JSXExpression::ObjectExpression(obj) =
                &container.expression
            else {
                continue;
            };
            // Wide tracking is the remediation's own advice for uppercase labels.
            if is_uppercase_style(obj) {
                continue;
            }

            for prop in &obj.properties {
                let ObjectPropertyKind::ObjectProperty(pair) = prop else {
                    continue;
                };

                // Check key is `letterSpacing`
                let key_name = match &pair.key {
                    PropertyKey::StaticIdentifier(id) => id.name.as_str(),
                    PropertyKey::StringLiteral(s) => s.value.as_str(),
                    _ => continue,
                };
                if key_name != "letterSpacing" {
                    continue;
                }

                // Value must be a string literal
                let Expression::StringLiteral(val) = &pair.value else {
                    continue;
                };
                let raw = val.value.as_str();
                let Some(num) = parse_em(raw) else {
                    continue;
                };

                let max_spacing = ctx.config.float(
                    "ui-no-wide-letter-spacing",
                    "max_letter_spacing_em",
                    ctx.lang,
                );
                if num <= max_spacing {
                    continue;
                }

                let (line, column) =
                    byte_offset_to_line_col(ctx.source, pair.span.start as usize);
                diagnostics.push(Diagnostic {
                    path: Arc::clone(&ctx.path_arc),
                    line,
                    column,
                    rule_id: super::META.id.into(),
                    message: format!(
                        "`letterSpacing: \"{raw}\"` — values above {max_spacing}em hurt readability."
                    ),
                    severity: Severity::Error,
                    span: None,
                });
            }
        }
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
    use crate::diagnostic::Diagnostic;

    fn run(source: &str) -> Vec<Diagnostic> {
        crate::rules::test_helpers::run_rule(&Check, source, "t.tsx")
    }

    #[test]
    fn flags_0_1_em() {
        assert_eq!(run(r#"<p style={{ letterSpacing: '0.1em' }} />"#).len(), 1);
    }

    #[test]
    fn flags_0_2_em() {
        assert_eq!(run(r#"<p style={{ letterSpacing: '0.2em' }} />"#).len(), 1);
    }

    #[test]
    fn allows_0_03_em() {
        assert!(run(r#"<p style={{ letterSpacing: '0.03em' }} />"#).is_empty());
    }

    #[test]
    fn allows_pixel_value() {
        assert!(run(r#"<p style={{ letterSpacing: '2px' }} />"#).is_empty());
    }

    #[test]
    fn allows_non_style_object() {
        assert!(run(r#"const config = { letterSpacing: '0.2em' };"#).is_empty());
    }

    // The remediation reserves wide tracking for uppercase labels.
    // An uppercase eyebrow is that exact case.
    #[test]
    fn allows_wide_tracking_on_uppercase_label() {
        let src = r#"<div style={{ fontSize: 26, letterSpacing: '0.22em', textTransform: 'uppercase' }} />"#;
        assert!(run(src).is_empty());
    }

    #[test]
    fn flags_wide_tracking_on_mixed_case_text() {
        let src = r#"<span style={{ letterSpacing: '0.14em', textTransform: 'capitalize' }} />"#;
        assert_eq!(run(src).len(), 1);
    }
}
