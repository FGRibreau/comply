//! tailwind-no-raw-color-utilities oxc backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{AstKind, CheckCtx, OxcCheck};
use crate::rules::tailwind_class_strings::class_strings;
use oxc_ast::ast::{
    Expression, JSXAttributeName, JSXAttributeValue, ObjectExpression, ObjectPropertyKind,
};
use oxc_span::Span;
use std::sync::Arc;

const PALETTE: &[&str] = &[
    "slate", "gray", "zinc", "neutral", "stone", "red", "orange", "amber", "yellow", "lime",
    "green", "emerald", "teal", "cyan", "sky", "blue", "indigo", "violet", "purple", "fuchsia",
    "pink", "rose",
];

const COLOR_PREFIXES: &[&str] = &[
    "bg-",
    "text-",
    "border-",
    "ring-",
    "fill-",
    "stroke-",
    "from-",
    "to-",
    "via-",
    "divide-",
    "outline-",
    "decoration-",
    "placeholder-",
    "caret-",
    "accent-",
    "shadow-",
];

const COLOR_FUNCTIONS: &[&str] = &["rgb(", "rgba(", "hsl(", "hsla(", "oklch("];

fn is_raw_color_class(utility: &str, with_arbitrary: bool, with_absolute: bool) -> bool {
    if with_arbitrary
        && let Some((_, bracketed)) = utility.split_once("-[")
        && (bracketed.starts_with('#') || COLOR_FUNCTIONS.iter().any(|f| bracketed.starts_with(f)))
    {
        return true;
    }
    COLOR_PREFIXES.iter().any(|prefix| {
        let Some(rest) = utility.strip_prefix(prefix) else {
            return false;
        };
        // An opacity modifier (`/50`) keeps the colour raw.
        let rest = rest.split_once('/').map_or(rest, |(color, _)| color);
        if with_absolute && matches!(rest, "white" | "black") {
            return true;
        }
        rest.rsplit_once('-').is_some_and(|(color, shade)| {
            PALETTE.contains(&color)
                && !shade.is_empty()
                && shade.chars().all(|c| c.is_ascii_digit())
        })
    })
}

fn is_color_literal(css: &str) -> bool {
    let css = css.trim();
    if let Some(hex) = css.strip_prefix('#') {
        return (3..=8).contains(&hex.len()) && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    COLOR_FUNCTIONS.iter().any(|f| css.starts_with(f)) && css.ends_with(')')
}

/// The object of a `style={{ … }}` attribute.
fn style_object<'a>(kind: AstKind<'a>) -> Option<&'a ObjectExpression<'a>> {
    let AstKind::JSXAttribute(attr) = kind else {
        return None;
    };
    let JSXAttributeName::Identifier(name) = &attr.name else {
        return None;
    };
    let Some(JSXAttributeValue::ExpressionContainer(container)) = &attr.value else {
        return None;
    };
    match container.expression.as_expression() {
        Some(Expression::ObjectExpression(object)) if name.name == "style" => Some(object),
        _ => None,
    }
}

/// Colour literals among the property values of `style={{ … }}`.
fn style_color_spans(semantic: &oxc_semantic::Semantic) -> Vec<Span> {
    let objects = semantic
        .nodes()
        .iter()
        .filter_map(|node| style_object(node.kind()));
    objects
        .flat_map(|object| &object.properties)
        .filter_map(|property| match property {
            ObjectPropertyKind::ObjectProperty(property) => match &property.value {
                Expression::StringLiteral(lit) if is_color_literal(&lit.value) => Some(lit.span),
                _ => None,
            },
            ObjectPropertyKind::SpreadProperty(_) => None,
        })
        .collect()
}

pub struct Check;

impl OxcCheck for Check {
    fn prefilter(&self) -> Option<&'static [&'static str]> {
        // The class-string entry points of `tailwind_class_strings`, plus `style`.
        Some(&["class", "cn", "cva", "clsx", "tw", "clx", "style"])
    }

    fn run_on_semantic<'a>(
        &self,
        semantic: &'a oxc_semantic::Semantic<'a>,
        ctx: &CheckCtx,
    ) -> Vec<Diagnostic> {
        let key_enabled = |key| ctx.config.bool_flag(super::META.id, key, ctx.lang);
        let with_arbitrary = key_enabled("arbitrary_colors");
        let with_absolute = key_enabled("absolute_colors");
        let mut diagnostics = Vec::new();
        let mut report = |span: Span, message: String| {
            let (line, column) = byte_offset_to_line_col(ctx.source, span.start as usize);
            diagnostics.push(Diagnostic {
                path: Arc::clone(&ctx.path_arc),
                line,
                column,
                rule_id: super::META.id.into(),
                message,
                severity: Severity::Error,
                span: None,
            });
        };
        for string in class_strings(semantic) {
            for class in &string.classes {
                if is_raw_color_class(class.utility, with_arbitrary, with_absolute) {
                    report(
                        string.span,
                        format!(
                            "`{}` is a raw colour — use a semantic token \
                             (bg-background, text-foreground, bg-primary, …).",
                            class.raw
                        ),
                    );
                }
            }
        }
        if key_enabled("style_objects") {
            for span in style_color_spans(semantic) {
                report(
                    span,
                    "Raw colour literal in `style` — use a token, `var(--…)`.".into(),
                );
            }
        }
        diagnostics
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
    use crate::config::Config;
    use crate::files::Language;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn run(src: &str) -> Vec<Diagnostic> {
        crate::rules::test_helpers::run_rule(&Check, src, "t.tsx")
    }

    fn run_with_config(src: &str, toml: &str) -> Vec<Diagnostic> {
        let project_dir = TempDir::new().expect("tempdir");
        fs::write(
            project_dir.path().join("comply.toml"),
            format!("[rules.tailwind-no-raw-color-utilities]\n{toml}\n"),
        )
        .expect("write cfg");
        let config = Config::load_from(project_dir.path()).expect("load cfg");
        crate::engine::lint_in_memory(Path::new("t.tsx"), Language::Tsx, src, &config, None)
            .into_iter()
            .filter(|d| d.rule_id.as_ref() == super::super::META.id)
            .collect()
    }

    #[test]
    fn flags_palette_step_in_every_class_string() {
        let src = r#"cn("bg-red-500/50");
            const C = "text-gray-900";
            const a = <i className={C} />;
            const b = <i className={ok ? "hover:bg-blue-500" : ""} />;"#;
        assert_eq!(run(src).len(), 3);
        assert!(run(r#"cn("bg-primary");"#).is_empty());
    }

    #[test]
    fn opt_in_keys_flag_arbitrary_absolute_and_style() {
        let keys = "arbitrary_colors = true\nabsolute_colors = true\nstyle_objects = true";
        let src = r##"cn("bg-[#fff]");
            const a = <i className="text-white" style={{ color: "#fff" }} />;
            cn("text-[rgb(0,0,0)]");"##;
        assert_eq!(run_with_config(src, keys).len(), 4);
        assert!(run_with_config(src, "").is_empty());
        let clean = r#"const a = <i style={{ color: "var(--fg)" }} />;
            cn("bg-[url(/a.png)] w-[10px]");"#;
        assert!(run_with_config(clean, keys).is_empty());
    }
}
