//! tailwind-no-transition-all-layout oxc backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{CheckCtx, OxcCheck};
use crate::rules::tailwind_class_strings::class_strings;
use std::sync::Arc;

pub struct Check;

/// CSS properties that move layout, matched by prefix.
const LAYOUT_PROPERTIES: &[&str] = &[
    "all", "width", "height", "min-", "max-", "top", "left", "right", "bottom", "inset", "margin",
    "padding",
];

fn transitions_layout(utility: &str) -> bool {
    if utility == "transition-all" {
        return true;
    }
    let Some(properties) = utility
        .strip_prefix("transition-[")
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return false;
    };
    properties.split(',').any(|property| {
        LAYOUT_PROPERTIES
            .iter()
            .any(|p| property.trim().starts_with(p))
    })
}

impl OxcCheck for Check {
    fn prefilter(&self) -> Option<&'static [&'static str]> {
        Some(&["transition-"])
    }

    fn run_on_semantic<'a>(
        &self,
        semantic: &'a oxc_semantic::Semantic<'a>,
        ctx: &CheckCtx,
    ) -> Vec<Diagnostic> {
        let hint = ctx.config.string(super::META.id, "hint", ctx.lang);
        let mut diagnostics = Vec::new();
        for string in class_strings(semantic) {
            for class in string
                .classes
                .iter()
                .filter(|c| transitions_layout(c.utility))
            {
                let mut message = format!(
                    "`{}` transitions layout on every frame. Use `transition-transform` \
                     or `transition-opacity`.",
                    class.raw
                );
                if !hint.is_empty() {
                    message.push(' ');
                    message.push_str(hint);
                }
                let (line, column) =
                    byte_offset_to_line_col(ctx.source, string.span.start as usize);
                diagnostics.push(Diagnostic {
                    path: Arc::clone(&ctx.path_arc),
                    line,
                    column,
                    rule_id: super::META.id.into(),
                    message,
                    severity: Severity::Error,
                    span: None,
                });
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
            format!("[rules.tailwind-no-transition-all-layout]\n{toml}\n"),
        )
        .expect("write cfg");
        let config = Config::load_from(project_dir.path()).expect("load cfg");
        crate::engine::lint_in_memory(Path::new("t.tsx"), Language::Tsx, src, &config, None)
            .into_iter()
            .filter(|d| d.rule_id.as_ref() == super::super::META.id)
            .collect()
    }

    #[test]
    fn flags_transition_all_and_layout_transition() {
        let src = r#"cn("transition-all");
            const a = <i className="hover:transition-[height]" />;
            cn("transition-[opacity,max-height]");
            cn("transition-[all]");"#;
        assert_eq!(run(src).len(), 4);
        let clean = r#"cn("transition-opacity transition-[opacity,transform]");
            const a = <i className="transition w-4" />;"#;
        assert!(run(clean).is_empty());
    }

    #[test]
    fn hint_appended_to_message() {
        let src = r#"cn("transition-all");"#;
        let hinted = run_with_config(src, r#"hint = "Use AnimatedSize.""#);
        assert!(hinted[0].message.ends_with(" Use AnimatedSize."));
        let plain = run_with_config(src, "");
        assert!(!plain[0].message.ends_with(char::is_whitespace));
    }
}
