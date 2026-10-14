//! ui-no-layout-property-animation OXC backend — flag inline `transition` /
//! `transitionProperty` styles that animate layout-triggering properties.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use oxc_ast::ast::{
    Expression, JSXAttributeItem, JSXAttributeName, JSXExpression, ObjectPropertyKind,
    PropertyKey,
};
use std::sync::Arc;

pub struct Check;

const TIMING_KEYS: &[&str] = &["transition", "transitionProperty"];

const LAYOUT_PROPERTIES: &[&str] = &[
    "width",
    "height",
    "min-width",
    "max-width",
    "min-height",
    "max-height",
    "top",
    "left",
    "right",
    "bottom",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "padding",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "border",
    "border-width",
    "border-top",
    "border-right",
    "border-bottom",
    "border-left",
];

fn first_layout_property(value: &str) -> Option<&'static str> {
    let lower = value.to_ascii_lowercase();
    for token in lower.split(|c: char| !c.is_ascii_lowercase() && c != '-') {
        if token.is_empty() {
            continue;
        }
        for &prop in LAYOUT_PROPERTIES {
            if token == prop {
                return Some(prop);
            }
        }
    }
    None
}

/// The layout property a camelCase motion key names (`marginTop` → `margin-top`).
fn layout_property_of_key(key: &str) -> Option<&'static str> {
    LAYOUT_PROPERTIES.iter().copied().find(|prop| {
        prop.chars()
            .filter(|&c| c != '-')
            .eq(key.chars().map(|c| c.to_ascii_lowercase()))
    })
}

/// Motion props holding a target: `animate`, `initial`, `exit`, `while*`.
fn is_motion_prop(name: &str) -> bool {
    matches!(name, "animate" | "initial" | "exit") || name.starts_with("while")
}

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::JSXOpeningElement]
    }

    fn prefilter(&self) -> Option<&'static [&'static str]> {
        Some(&[
            "transition",
            "transitionProperty",
            "animate",
            "initial",
            "exit",
            "while",
        ])
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        _semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let AstKind::JSXOpeningElement(opening) = node.kind() else { return };

        for attr_item in &opening.attributes {
            let JSXAttributeItem::Attribute(attr) = attr_item else { continue };
            let JSXAttributeName::Identifier(attr_name) = &attr.name else { continue };
            let is_motion = is_motion_prop(attr_name.name.as_str());
            if !is_motion && attr_name.name.as_str() != "style" {
                continue;
            }

            let Some(ref value) = attr.value else { continue };
            let oxc_ast::ast::JSXAttributeValue::ExpressionContainer(container) = value else {
                continue;
            };
            let JSXExpression::ObjectExpression(obj) = &container.expression else {
                continue;
            };

            for prop in &obj.properties {
                let ObjectPropertyKind::ObjectProperty(pair) = prop else { continue };
                if is_motion {
                    let Some(layout_prop) = pair
                        .key
                        .static_name()
                        .and_then(|key| layout_property_of_key(&key))
                    else {
                        continue;
                    };
                    report(ctx, pair.span.start, layout_prop, diagnostics);
                    continue;
                }
                let key = match &pair.key {
                    PropertyKey::StaticIdentifier(id) => id.name.as_str(),
                    _ => continue,
                };
                if !TIMING_KEYS.contains(&key) {
                    continue;
                }

                let value_str = match &pair.value {
                    Expression::StringLiteral(s) => s.value.as_str(),
                    _ => continue,
                };

                let Some(layout_prop) = first_layout_property(value_str) else { continue };
                report(ctx, pair.span.start, layout_prop, diagnostics);
            }
        }
    }
}

fn report(ctx: &CheckCtx, start: u32, layout_prop: &str, diagnostics: &mut Vec<Diagnostic>) {
    let (line, column) = byte_offset_to_line_col(ctx.source, start as usize);
    diagnostics.push(Diagnostic {
        path: Arc::clone(&ctx.path_arc),
        line,
        column,
        rule_id: super::META.id.into(),
        message: format!(
            "Animating layout property `{layout_prop}` triggers layout recalculation every frame — \
             prefer `transform`, `opacity`, `color`, `background`, or `filter`."
        ),
        severity: Severity::Error,
        span: None,
    });
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
    fn flags_layout_keys_in_motion_props() {
        assert_eq!(
            run("const a = <motion.div animate={{ height: 100 }} />;").len(),
            1
        );
        assert_eq!(
            run("const a = <motion.div whileHover={{ width: 10 }} />;").len(),
            1
        );
        assert_eq!(
            run("const a = <motion.div initial={{ marginTop: 4 }} />;").len(),
            1
        );
        assert_eq!(
            run("const a = <div style={{ transition: \"width 1s\" }} />;").len(),
            1
        );
    }

    #[test]
    fn allows_compositor_keys_and_opaque_motion_values() {
        assert!(run("const a = <motion.div animate={{ opacity: 1, x: 10 }} />;").is_empty());
        assert!(run("const a = <motion.div animate={variants} />;").is_empty());
    }
}
