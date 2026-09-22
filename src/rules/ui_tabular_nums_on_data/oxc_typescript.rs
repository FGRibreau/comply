//! ui-tabular-nums-on-data OXC backend — flag JSX elements whose className
//! contains a numeric-data hint word but lacks `tabular-nums`.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use oxc_ast::ast::JSXAttributeItem;
use std::sync::Arc;

pub struct Check;

fn extract_class_value<'a>(
    attrs: &'a oxc_allocator::Vec<'a, JSXAttributeItem<'a>>,
) -> Option<&'a str> {
    for item in attrs {
        let JSXAttributeItem::Attribute(attr) = item else {
            continue;
        };
        let oxc_ast::ast::JSXAttributeName::Identifier(name) = &attr.name else {
            continue;
        };
        if name.name.as_str() != "className" {
            continue;
        }
        if let Some(oxc_ast::ast::JSXAttributeValue::StringLiteral(lit)) = &attr.value {
            return Some(lit.value.as_str());
        }
    }
    None
}

fn check_class(
    cls: &str,
    span_start: u32,
    ctx: &CheckCtx,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !super::has_data_hint(cls) {
        return;
    }
    let lower = cls.to_ascii_lowercase();
    if lower.contains("tabular-nums")
        || lower.contains("tabular-numbers")
        || lower.contains("font-variant-numeric")
    {
        return;
    }
    let (line, column) = byte_offset_to_line_col(ctx.source, span_start as usize);
    diagnostics.push(Diagnostic {
        path: Arc::clone(&ctx.path_arc),
        line,
        column,
        rule_id: super::META.id.into(),
        message: format!(
            "className `{cls}` suggests numeric data but is missing `tabular-nums` — digits will jitter between updates."
        ),
        severity: Severity::Error,
        span: None,
    });
}

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[AstType::JSXOpeningElement]
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        _semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let AstKind::JSXOpeningElement(opening) = node.kind() else {
            return;
        };
        if let Some(cls) = extract_class_value(&opening.attributes) {
            check_class(cls, opening.span.start, ctx, diagnostics);
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

    fn run(source: &str) -> Vec<Diagnostic> {
        crate::rules::test_helpers::run_rule(&Check, source, "t.tsx")
    }

    #[test]
    fn flags_stat_class_without_tabular_nums() {
        assert_eq!(run(r#"const x = <span className="stat-value text-2xl" />;"#).len(), 1);
    }

    #[test]
    fn allows_stat_class_with_tabular_nums() {
        assert!(run(r#"const x = <span className="stat-value tabular-nums" />;"#).is_empty());
    }

    #[test]
    fn ignores_state_data_attribute_variant() {
        // natalia v3 `team-switcher.tsx`: `group-data-[state=collapsed]:*`
        // contains `stat` only inside the word `state`.
        assert!(
            run(r#"const x = <div className="grid min-w-0 flex-1 text-left leading-tight group-data-[state=collapsed]:hidden" />;"#)
                .is_empty()
        );
    }
}
