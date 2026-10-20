//! Rendered copy: the JSX text and the strings a user reads.
//! Copy is read raw, never decoded.
//! A diagnostic offset then points at the character to fix.

use crate::rules::backend::AstKind;
use oxc_ast::ast::{Expression, JSXAttribute, JSXAttributeName};
use oxc_semantic::{AstNode, Semantic};
use oxc_span::Span;

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

/// Raw copy of a `JSXText`, a string literal or a template quasi.
/// Returns the text, its byte offset, its escape context, and whether it is sure prose.
/// A string outside `COPY_ATTRS` counts only when it holds a blank.
#[must_use]
pub fn copy_text<'s>(
    node: &AstNode<'_>,
    semantic: &Semantic<'_>,
    source: &'s str,
) -> Option<(&'s str, usize, Escapes, bool)> {
    let nodes = semantic.nodes();
    let (span, literal) = match node.kind() {
        AstKind::JSXText(text) => return slice(source, text.span, Escapes::Html, true),
        // The literal span includes its quotes.
        AstKind::StringLiteral(lit) => (lit.span.shrink(1), node),
        AstKind::TemplateElement(quasi) => (quasi.span, nodes.parent_node(node.id())),
        _ => return None,
    };
    let context = nodes.parent_node(literal.id());
    let (escapes, is_listed_attr) = match context.kind() {
        AstKind::JSXAttribute(attr) => (Escapes::Html, is_copy_attr(attr)),
        AstKind::JSXExpressionContainer(_) => {
            let is_braced_copy = matches!(
                nodes.parent_node(context.id()).kind(),
                AstKind::JSXAttribute(attr) if is_copy_attr(attr)
            );
            (Escapes::Js, is_braced_copy)
        }
        AstKind::NewExpression(new) if names_error(&new.callee) => return None,
        AstKind::CallExpression(call) if is_console(&call.callee) => return None,
        _ => (Escapes::Js, false),
    };
    let copy = slice(source, span, escapes, is_listed_attr)?;
    (is_listed_attr || copy.0.contains(char::is_whitespace)).then_some(copy)
}

fn slice(
    source: &str,
    span: Span,
    escapes: Escapes,
    is_prose: bool,
) -> Option<(&str, usize, Escapes, bool)> {
    let (start, end) = (span.start as usize, span.end as usize);
    Some((source.get(start..end)?, start, escapes, is_prose))
}

fn is_copy_attr(attr: &JSXAttribute<'_>) -> bool {
    matches!(&attr.name, JSXAttributeName::Identifier(name) if COPY_ATTRS.contains(&name.name.as_str()))
}

/// `new FooError(…)`: developer text, never copy.
fn names_error(callee: &Expression<'_>) -> bool {
    matches!(callee, Expression::Identifier(id) if id.name.ends_with("Error"))
}

/// `console.warn(…)`: developer text, never copy.
fn is_console(callee: &Expression<'_>) -> bool {
    matches!(
        callee,
        Expression::StaticMemberExpression(member)
            if matches!(&member.object, Expression::Identifier(id) if id.name == "console")
    )
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
