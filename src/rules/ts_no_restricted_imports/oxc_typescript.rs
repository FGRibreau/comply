//! ts-no-restricted-imports OXC backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::oxc_helpers::byte_offset_to_line_col;
use crate::rules::backend::{AstKind, AstType, CheckCtx, OxcCheck};
use crate::rules::path_utils::matches_any_glob;
use oxc_ast::ast::{Expression, ImportDeclarationSpecifier};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

pub struct Check;

/// Exact match, or prefix match on a trailing `*`.
fn specifier_matches(specifier: &str, pattern: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        specifier.starts_with(prefix)
    } else {
        specifier == pattern
    }
}

/// One `[[rules.ts-no-restricted-imports.entries]]` table.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    pattern: String,
    message: Option<String>,
    #[serde(default)]
    allow_type_only: bool,
    #[serde(default)]
    import_names: Vec<String>,
}

/// What a module-reference node takes from its module.
enum Bindings<'a> {
    /// `import * as`, `export * from`, `import()`: every export.
    Module { type_only: bool },
    /// Named bindings with their type-only flag; empty for `import "x"`.
    Names(Vec<(&'a str, bool)>),
}

fn import_bindings<'a>(import: &'a oxc_ast::ast::ImportDeclaration<'a>) -> Bindings<'a> {
    let statement_type = import.import_kind.is_type();
    let mut names = Vec::new();
    for specifier in import.specifiers.iter().flatten() {
        match specifier {
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {
                return Bindings::Module {
                    type_only: statement_type,
                };
            }
            ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => {
                names.push(("default", statement_type));
            }
            ImportDeclarationSpecifier::ImportSpecifier(named) => {
                let is_type = statement_type || named.import_kind.is_type();
                names.push((named.imported.name().as_str(), is_type));
            }
        }
    }
    Bindings::Names(names)
}

fn module_reference<'a>(kind: &AstKind<'a>) -> Option<(&'a str, u32, Bindings<'a>)> {
    match kind {
        AstKind::ImportDeclaration(import) => Some((
            import.source.value.as_str(),
            import.span.start,
            import_bindings(import),
        )),
        AstKind::ExportNamedDeclaration(export) => {
            let source = export.source.as_ref()?;
            let statement_type = export.export_kind.is_type();
            let names = export
                .specifiers
                .iter()
                .map(|s| {
                    (
                        s.local.name().as_str(),
                        statement_type || s.export_kind.is_type(),
                    )
                })
                .collect();
            Some((
                source.value.as_str(),
                export.span.start,
                Bindings::Names(names),
            ))
        }
        AstKind::ExportAllDeclaration(export) => Some((
            export.source.value.as_str(),
            export.span.start,
            Bindings::Module {
                type_only: export.export_kind.is_type(),
            },
        )),
        AstKind::ImportExpression(call) => {
            let source = match &call.source {
                Expression::StringLiteral(literal) => literal.value.as_str(),
                Expression::TemplateLiteral(template) => template.single_quasi()?.as_str(),
                _ => return None,
            };
            Some((
                source,
                call.span.start,
                Bindings::Module { type_only: false },
            ))
        }
        _ => None,
    }
}

/// Project-relative resolved path, extension stripped.
fn resolved_project_path(ctx: &CheckCtx, specifier: &str) -> Option<PathBuf> {
    let resolved = ctx
        .project
        .import_index()
        .get_imports(ctx.path)
        .iter()
        .find(|symbol| symbol.specifier == specifier)?
        .source_path
        .as_ref()?;
    // Indexed paths are canonical; the project root may not be.
    // Canonicalized only here, so package imports skip the syscall.
    let root = std::fs::canonicalize(ctx.project.project_root.as_ref()?).ok()?;
    Some(resolved.strip_prefix(&root).ok()?.with_extension(""))
}

fn entry_restricts(entry: &Entry, bindings: &Bindings) -> bool {
    match bindings {
        Bindings::Module { type_only } => !entry.allow_type_only || !*type_only,
        Bindings::Names(names) => {
            let mut value_names = names
                .iter()
                .filter(|(_, is_type)| !entry.allow_type_only || !*is_type)
                .map(|(name, _)| *name)
                .peekable();
            // A statement whose every binding is an allowed type passes.
            if !names.is_empty() && value_names.peek().is_none() {
                return false;
            }
            entry.import_names.is_empty()
                || value_names.any(|name| entry.import_names.iter().any(|n| n == name))
        }
    }
}

/// The diagnostic message of the first entry restricting this reference.
fn entries_message(ctx: &CheckCtx, specifier: &str, bindings: &Bindings) -> Option<String> {
    let entries: Vec<Entry> = ctx
        .config
        .table_list("ts-no-restricted-imports", "entries", ctx.lang)
        .into_iter()
        .enumerate()
        .map(|(i, table)| {
            toml::Value::Table(table.clone())
                .try_into()
                .unwrap_or_else(|e| panic!("[rules.ts-no-restricted-imports] entries[{i}]: {e}"))
        })
        .collect();
    if entries.is_empty() {
        return None;
    }
    let resolved = resolved_project_path(ctx, specifier);
    entries.into_iter().find_map(|entry| {
        let patterns = std::slice::from_ref(&entry.pattern);
        let matches = matches_any_glob(specifier, patterns)
            || resolved
                .as_ref()
                .is_some_and(|path| matches_any_glob(path, patterns));
        if !matches || !entry_restricts(&entry, bindings) {
            return None;
        }
        let base = format!(
            "Import from `{specifier}` matches restricted pattern `{}`.",
            entry.pattern
        );
        Some(match entry.message {
            Some(message) => format!("{base} {message}"),
            None => format!("{base} See comply.toml for the restriction list."),
        })
    })
}

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[
            AstType::ImportDeclaration,
            AstType::ExportNamedDeclaration,
            AstType::ExportAllDeclaration,
            AstType::ImportExpression,
        ]
    }

    fn run<'a>(
        &self,
        node: &oxc_semantic::AstNode<'a>,
        ctx: &CheckCtx,
        _semantic: &'a oxc_semantic::Semantic<'a>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let kind = node.kind();
        let Some((module_path, start, bindings)) = module_reference(&kind) else {
            return;
        };
        if module_path.is_empty() {
            return;
        }

        // Bare `patterns` reach `import` statements only.
        let bare = if let AstKind::ImportDeclaration(_) = kind {
            ctx.config
                .string_list("ts-no-restricted-imports", "patterns", ctx.lang)
                .into_iter()
                .find(|p| specifier_matches(module_path, p))
                .map(|matched| format!(
                    "Import from `{module_path}` matches restricted pattern `{matched}`. See comply.toml for the restriction list."
                ))
        } else {
            None
        };
        let Some(message) = bare.or_else(|| entries_message(ctx, module_path, &bindings)) else {
            return;
        };

        let (line, column) = byte_offset_to_line_col(ctx.source, start as usize);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::files::{Language, SourceFile};
    use crate::project::ProjectCtx;
    use crate::rules::file_ctx::FileCtx;

    /// Index `files` under a project root; run on the last.
    fn run_in_project(comply_toml: Option<&str>, files: &[(&str, &str)]) -> Vec<Diagnostic> {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        if let Some(text) = comply_toml {
            std::fs::write(root.join("comply.toml"), text).unwrap();
        }
        let mut indexed = Vec::new();
        for &(rel, source) in files {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, source).unwrap();
            if let Some(language) = Language::from_path(&path) {
                indexed.push(SourceFile { path, language });
            }
        }
        let refs: Vec<&SourceFile> = indexed.iter().collect();
        let mut project = ProjectCtx::for_test_with_files(&refs);
        // Production roots may be relative or symlinked.
        project.project_root = Some(root.join("src/.."));
        let config = Config::load_from(&root).unwrap();

        let (rel, source) = *files.last().unwrap();
        let path = root.join(rel);
        let lang = Language::from_path(&path).unwrap();
        let file = FileCtx::build(&path, source, lang, &project);
        let ctx = CheckCtx {
            path: &path,
            path_arc: Arc::from(path.as_path()),
            source,
            config: &config,
            project: &project,
            file: &file,
            lang,
        };
        let allocator = oxc_allocator::Allocator::default();
        let source_type = crate::oxc_helpers::source_type_for_path(&path);
        let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();
        let semantic = oxc_semantic::SemanticBuilder::new()
            .build(&parsed.program)
            .semantic;
        let mut diagnostics = Vec::new();
        for node in semantic.nodes().iter() {
            if Check.interested_kinds().contains(&node.kind().ty()) {
                Check.run(node, &ctx, &semantic, &mut diagnostics);
            }
        }
        diagnostics
    }

    #[test]
    fn entry_message_flags_named_import() {
        let comply_toml = r#"
[[rules.ts-no-restricted-imports.entries]]
pattern = "lucide-react"
message = "Icon"
"#;
        let diagnostics = run_in_project(
            Some(comply_toml),
            &[("src/a.tsx", "import { X } from \"lucide-react\";\n")],
        );
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].message.contains("Icon"),
            "{}",
            diagnostics[0].message
        );
    }

    const LUCIDE: &str = "[[rules.ts-no-restricted-imports.entries]]\npattern = \"lucide-react\"\n";

    fn count(comply_toml: Option<&str>, source: &str) -> usize {
        run_in_project(comply_toml, &[("src/a.tsx", source)]).len()
    }

    #[test]
    fn allow_type_only_passes_types_not_values() {
        let toml = format!("{LUCIDE}allow_type_only = true\n");
        for source in [
            "import type { X } from \"lucide-react\";\n",
            "import { type Y } from \"lucide-react\";\n",
            "export type { Z } from \"lucide-react\";\n",
        ] {
            assert_eq!(count(Some(&toml), source), 0, "{source}");
        }
        assert_eq!(
            count(Some(&toml), "import { type X, W } from \"lucide-react\";\n"),
            1
        );
    }

    #[test]
    fn import_names_restricts_to_listed_names() {
        let toml = "[[rules.ts-no-restricted-imports.entries]]\npattern = \"motion/react\"\nimport_names = [\"motion\"]\n";
        for source in [
            "import { motion } from \"motion/react\";\n",
            "export { motion } from \"motion/react\";\n",
            "import * as m from \"motion/react\";\n",
        ] {
            assert_eq!(count(Some(toml), source), 1, "{source}");
        }
        for source in [
            "import { useReducedMotion } from \"motion/react\";\n",
            "import \"motion/react\";\n",
        ] {
            assert_eq!(count(Some(toml), source), 0, "{source}");
        }
    }

    #[test]
    fn pattern_matches_resolved_project_path() {
        let toml = "[[rules.ts-no-restricted-imports.entries]]\npattern = \"**/ui/collapsible\"\n";
        let tsconfig = (
            "tsconfig.json",
            r##"{"compilerOptions":{"baseUrl":".","paths":{"#/*":["src/*"],"@ui/*":["src/ui/*"]}}}"##,
        );
        let target = ("src/ui/collapsible.tsx", "export const C = 1;\n");
        for (path, source) in [
            (
                "src/pages/a.tsx",
                "import { C } from \"@ui/collapsible\";\n",
            ),
            (
                "src/pages/a.tsx",
                "import { C } from \"#/ui/collapsible\";\n",
            ),
            (
                "src/pages/a.tsx",
                "import { C } from \"../ui/collapsible\";\n",
            ),
            ("src/ui/menu.tsx", "import { C } from \"./collapsible\";\n"),
        ] {
            let diagnostics = run_in_project(Some(toml), &[tsconfig, target, (path, source)]);
            assert_eq!(diagnostics.len(), 1, "{source}");
        }
    }

    #[test]
    fn reexports_and_literal_dynamic_imports_are_flagged() {
        for source in [
            "export { X } from \"lucide-react\";\n",
            "export * from \"lucide-react\";\n",
            "const m = import(\"lucide-react\");\n",
            "const m = import(`lucide-react`);\n",
        ] {
            assert_eq!(count(Some(LUCIDE), source), 1, "{source}");
        }
        assert_eq!(count(Some(LUCIDE), "const m = import(name);\n"), 0);
    }

    #[test]
    fn bare_patterns_behave_as_before() {
        let toml = "[rules.ts-no-restricted-imports]\npatterns = [\"lodash\", \"@banned/*\"]\n";
        let diagnostics = run_in_project(
            Some(toml),
            &[(
                "src/a.tsx",
                "import _ from \"lodash\";\nimport a from \"@banned/x/y\";\n",
            )],
        );
        let messages: Vec<&str> = diagnostics.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(
            messages,
            [
                "Import from `lodash` matches restricted pattern `lodash`. See comply.toml for the restriction list.",
                "Import from `@banned/x/y` matches restricted pattern `@banned/*`. See comply.toml for the restriction list.",
            ]
        );
        assert_eq!(count(Some(toml), "import b from \"lodash-es\";\n"), 0);
        assert_eq!(count(Some(toml), "export * from \"lodash\";\n"), 0);
    }

    #[test]
    fn default_config_is_inert() {
        assert_eq!(count(None, "import { X } from \"lucide-react\";\n"), 0);
    }

    #[test]
    #[should_panic(expected = "entries[1]")]
    fn invalid_entry_panics_naming_its_index() {
        let toml = format!(
            "{LUCIDE}\n[[rules.ts-no-restricted-imports.entries]]\npattern = \"x\"\nmesssage = \"y\"\n"
        );
        count(Some(&toml), "import { X } from \"lucide-react\";\n");
    }
}
