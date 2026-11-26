//! tailwind-read-theme-before-classes OXC backend — flag arbitrary Tailwind
//! values in className/class attributes without theme references.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use oxc_ast::ast::{JSXAttributeName, JSXAttributeValue};
use std::sync::Arc;

/// Markers that indicate the file already reads the Tailwind theme/config.
const THEME_MARKERS: &[&str] = &[
    "tailwind.config",
    "tailwindConfig",
    "resolveConfig",
    "theme(",
    "from 'tailwindcss/",
    "from \"tailwindcss/",
];

/// Tailwind utility prefixes that accept arbitrary values worth flagging.
const ARBITRARY_PREFIXES: &[&str] = &[
    "p-[",
    "px-[",
    "py-[",
    "pt-[",
    "pb-[",
    "pl-[",
    "pr-[",
    "m-[",
    "mx-[",
    "my-[",
    "mt-[",
    "mb-[",
    "ml-[",
    "mr-[",
    "gap-[",
    "gap-x-[",
    "gap-y-[",
    "space-x-[",
    "space-y-[",
    "w-[",
    "h-[",
    "min-w-[",
    "min-h-[",
    "max-w-[",
    "max-h-[",
    "text-[",
    "bg-[",
    "border-[",
    "rounded-[",
    "ring-[",
    "shadow-[",
    "leading-[",
    "tracking-[",
];

fn class_contains_arbitrary(text: &str) -> Option<usize> {
    for prefix in ARBITRARY_PREFIXES {
        let mut search_from = 0;
        while let Some(rel) = text[search_from..].find(prefix) {
            let start = search_from + rel;
            if start > 0
                && text
                    .as_bytes()
                    .get(start - 1)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'-')
            {
                search_from = start + 1;
                continue;
            }
            if let Some(close) = text[start + prefix.len()..].find(']') {
                let value = &text[start + prefix.len()..start + prefix.len() + close];
                if value.contains("var(--") || value.starts_with("--") {
                    search_from = start + prefix.len() + close + 1;
                    continue;
                }
                return Some(start);
            }
            search_from = start + prefix.len();
        }
    }
    None
}

fn file_reads_theme(source: &str) -> bool {
    THEME_MARKERS.iter().any(|m| crate::oxc_helpers::source_contains(source, m))
}

pub struct Check;

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::JSXAttribute]
    }

    fn prefilter(&self) -> Option<&'static [&'static str]> {
        Some(&["resolveConfig"])
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        _semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let AstKind::JSXAttribute(attr) = node.kind() else {
            return;
        };

        // shadcn/ui primitives use arbitrary values by design.
        let path_str = crate::rules::path_utils::path_for_matching(ctx.path);
        if path_str.contains("/components/ui/") || path_str.contains("/lib/ui/") {
            return;
        }

        // Must be className or class attribute.
        let JSXAttributeName::Identifier(name) = &attr.name else {
            return;
        };
        let attr_name = name.name.as_str();
        if attr_name != "className" && attr_name != "class" {
            return;
        }

        // Get the string value.
        let Some(JSXAttributeValue::StringLiteral(lit)) = &attr.value else {
            return;
        };
        let text = lit.value.as_str();

        if class_contains_arbitrary(text).is_none() {
            return;
        }

        if file_reads_theme(ctx.source) {
            return;
        }

        let (line, column) =
            byte_offset_to_line_col(ctx.source, lit.span.start as usize);
        diagnostics.push(Diagnostic {
            path: Arc::clone(&ctx.path_arc),
            line,
            column,
            rule_id: super::META.id.into(),
            message: "Arbitrary Tailwind value used without reading the theme. \
                      Import `tailwind.config` / call `resolveConfig(...)` / use `theme(...)`, \
                      or switch to a design-token class."
                .into(),
            severity: Severity::Error,
            span: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_at(path: &str) -> Vec<Diagnostic> {
        crate::rules::test_helpers::run_oxc_check(
            &Check,
            r#"export const A = () => <div className="bg-[#abc]" />;"#,
            std::path::Path::new(path),
            &crate::project::ProjectCtx::empty_with_tailwind(),
            crate::rules::file_ctx::default_static_file_ctx(),
        )
    }

    // `--staged` lists a root `components/ui/` file without `./`.
    #[test]
    fn skips_a_root_ui_library_dir() {
        let d = run_at("components/ui/card.tsx");
        assert!(d.is_empty(), "{d:?}");
    }

    // A partial `components` match stays application code.
    #[test]
    fn still_flags_an_arbitrary_value_near_a_ui_dir_substring() {
        let d = run_at("mycomponents/ui/card.tsx");
        assert_eq!(d.len(), 1, "{d:?}");
    }
}
