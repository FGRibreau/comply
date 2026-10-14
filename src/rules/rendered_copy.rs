//! Rendered copy: the JSX text a user reads.
//! Copy is read raw, never decoded.
//! A diagnostic offset then points at the character to fix.

use crate::rules::backend::AstKind;
use oxc_ast::ast::{JSXAttributeName, JSXAttributeValue, JSXExpression};

/// JSX attributes whose string value is user-facing copy.
pub const COPY_ATTRS: &[&str] = &[
    "title",
    "label",
    "placeholder",
    "alt",
    "aria-label",
    "aria-description",
];

/// The syntax that decodes a piece of source text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Escapes {
    /// JSX text or a JSX attribute string: HTML entities.
    Html,
    /// A JS string or template literal: `\u` escapes.
    Js,
    /// A Rust string or char literal: `\u{..}` escapes.
    Rust,
}

/// Raw copy of a `JSXText` or a copy attribute value.
/// Returns the text, its byte offset and its escape context.
/// A string or substitution-free template expression also counts.
#[must_use]
pub fn copy_text<'a>(kind: &AstKind<'_>, source: &'a str) -> Option<(&'a str, usize, Escapes)> {
    let (span, escapes) = match kind {
        AstKind::JSXText(text) => (text.span, Escapes::Html),
        AstKind::JSXAttribute(attr) => {
            let JSXAttributeName::Identifier(name) = &attr.name else {
                return None;
            };
            if !COPY_ATTRS.contains(&name.name.as_str()) {
                return None;
            }
            // The spans below include the quotes or backticks.
            match attr.value.as_ref()? {
                JSXAttributeValue::StringLiteral(lit) => (lit.span.shrink(1), Escapes::Html),
                JSXAttributeValue::ExpressionContainer(container) => match &container.expression {
                    JSXExpression::StringLiteral(lit) => (lit.span.shrink(1), Escapes::Js),
                    JSXExpression::TemplateLiteral(template) if template.expressions.is_empty() => {
                        (template.span.shrink(1), Escapes::Js)
                    }
                    _ => return None,
                },
                _ => return None,
            }
        }
        _ => return None,
    };
    let (start, end) = (span.start as usize, span.end as usize);
    Some((source.get(start..end)?, start, escapes))
}

/// How `c` is written escaped in `escapes`: `&nbsp;`, `\u00A0` or `\u{a0}`.
#[must_use]
pub fn escape(c: char, escapes: Escapes) -> String {
    let code = u32::from(c);
    match (escapes, c) {
        (Escapes::Html, '\u{a0}') => "&nbsp;".into(),
        (Escapes::Html, '\u{202f}') => "&#8239;".into(),
        (Escapes::Html, _) => format!("&#x{code:X};"),
        (Escapes::Js, _) => format!("\\u{code:04X}"),
        (Escapes::Rust, _) => format!("\\u{{{code:x}}}"),
    }
}
