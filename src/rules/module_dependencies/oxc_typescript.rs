//! module-dependencies OxcCheck backend, over the project import index.

use crate::diagnostic::Diagnostic;
use crate::rules::backend::{AstType, CheckCtx, OxcCheck};
use std::path::Path;
use std::sync::Arc;

pub struct Check;

const RULE: &str = super::META.id;

impl OxcCheck for Check {
    fn interested_kinds(&self) -> &'static [AstType] {
        &[]
    }

    fn run_on_semantic<'a>(
        &self,
        _semantic: &'a oxc_semantic::Semantic<'a>,
        ctx: &CheckCtx,
    ) -> Vec<Diagnostic> {
        let root = ctx.config.string(RULE, "root", ctx.lang);
        let index = ctx.project.import_index();
        let Some(config_dir) = ctx.config.dir() else {
            return Vec::new();
        };
        if root.is_empty() || index.is_empty() {
            return Vec::new();
        }
        let root_dir = std::fs::canonicalize(config_dir.join(root))
            .ok()
            .filter(|dir| dir.is_dir())
            .unwrap_or_else(|| panic!("[rules.\"{RULE}\"] root `{root}` is not a directory"));
        let family = ctx
            .config
            .string(RULE, "wildcard", ctx.lang)
            .trim_end_matches("/*");
        let Some(importer) = module_of(&index.canonical(ctx.path), &root_dir, family) else {
            return Vec::new();
        };
        let matrix = ctx.config.table(RULE, "modules", ctx.lang);
        let allowed: Option<Vec<String>> = matrix.get(&importer.0).map(|line| {
            line.clone()
                .try_into()
                .unwrap_or_else(|err| panic!("[rules.\"{RULE}\"] modules.{}: {err}", importer.0))
        });

        let imports = index
            .get_imports(ctx.path)
            .iter()
            .filter_map(|import| Some((import.line, import.source_path.as_deref()?)));
        let reexports = index
            .reexport_origins(ctx.path)
            .iter()
            .map(|origin| (1, origin.as_path()));
        let mut judged = Vec::new();
        let mut diagnostics = Vec::new();
        let targets = imports
            .chain(reexports)
            .filter_map(|(line, path)| Some((line, module_of(path, &root_dir, family)?)));
        for (line, target) in targets {
            if target.1 == importer.1 || judged.contains(&(line, target.1.clone())) {
                continue;
            }
            judged.push((line, target.1.clone()));
            if let Some(message) = refusal(&importer, &target, allowed.as_deref(), matrix) {
                diagnostics.push(Diagnostic {
                    path: Arc::clone(&ctx.path_arc),
                    line,
                    column: 1,
                    rule_id: RULE.into(),
                    message,
                    severity: super::META.severity,
                    span: None,
                });
            }
        }
        diagnostics
    }
}

/// `(matrix line, module id)` of `path`, `None` outside any module.
fn module_of(path: &Path, root: &Path, family: &str) -> Option<(String, String)> {
    let dir = path.strip_prefix(root).ok()?.parent()?;
    if !family.is_empty()
        && let Ok(rest) = dir.strip_prefix(family)
        && let Some(feature) = rest.components().next()
    {
        let feature = feature.as_os_str().to_string_lossy();
        return Some((format!("{family}/*"), format!("{family}/{feature}")));
    }
    let first = dir
        .components()
        .next()?
        .as_os_str()
        .to_string_lossy()
        .into_owned();
    Some((first.clone(), first))
}

fn refusal(
    (importer_line, importer_id): &(String, String),
    (target_line, target_id): &(String, String),
    allowed: Option<&[String]>,
    matrix: &toml::Table,
) -> Option<String> {
    let edge = format!("`{importer_id}` may not import `{target_id}`");
    let undeclared = format!("is not declared in [rules.{RULE}] modules");
    let Some(allowed) = allowed else {
        return Some(format!("{edge}: `{importer_line}` {undeclared}"));
    };
    let listed = if allowed.is_empty() {
        "none".to_string()
    } else {
        allowed.join(", ")
    };
    if !matrix.contains_key(target_line) {
        return Some(format!(
            "{edge}: `{target_line}` {undeclared} (allowed: {listed})"
        ));
    }
    if allowed.contains(target_line) {
        return None;
    }
    Some(format!("{edge} (allowed: {listed})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::files::{Language, SourceFile};
    use crate::project::ProjectCtx;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use std::fs;
    use tempfile::TempDir;

    /// Writes the project under `<tmp>/src/proj/`, so `src` is also an ancestor.
    fn run_in_project(comply_toml: &str, files: &[(&str, &str)], target: &str) -> Vec<Diagnostic> {
        let ancestor = TempDir::new().unwrap();
        let project_dir = ancestor.path().join("src/proj");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("comply.toml"), comply_toml).unwrap();
        let mut sources = Vec::new();
        for &(rel, source) in files {
            let path = project_dir.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, source).unwrap();
            if let Some(language) = Language::from_path(&path) {
                sources.push(SourceFile { path, language });
            }
        }
        let refs: Vec<&SourceFile> = sources.iter().collect();
        let project = ProjectCtx::for_test_with_files(&refs);
        let config = Config::load_from(&project_dir).unwrap();
        let path = project_dir.join(target);
        let source = fs::read_to_string(&path).unwrap();
        let allocator = Allocator::default();
        let parsed = Parser::new(
            &allocator,
            &source,
            crate::oxc_helpers::source_type_for_path(&path),
        )
        .parse();
        let semantic = SemanticBuilder::new().build(&parsed.program).semantic;
        let ctx = CheckCtx {
            path: &path,
            path_arc: Arc::from(path.as_path()),
            source: &source,
            config: &config,
            project: &project,
            file: crate::rules::file_ctx::default_static_file_ctx(),
            lang: Language::TypeScript,
        };
        Check.run_on_semantic(&semantic, &ctx)
    }

    const MATRIX_HEADER: &str =
        "[rules.module-dependencies]\nroot = \"src\"\n\n[rules.module-dependencies.modules]\n";

    #[test]
    fn allowed_unlisted_and_own_module_imports() {
        let toml = format!(
            "{MATRIX_HEADER}failure = [\"api\", \"ui\"]\napi = []\nui = []\nlanguage = []\n"
        );
        let diagnostics = run_in_project(
            &toml,
            &[
                ("src/ui/b.ts", "export const b = 1;\n"),
                ("src/failure/d.ts", "export const d = 1;\n"),
                ("src/language/c.ts", "export const c = 1;\n"),
                (
                    "src/failure/a.ts",
                    "import { b } from '../ui/b';\nimport { d } from './d';\nimport { c } from '../language/c';\n",
                ),
            ],
            "src/failure/a.ts",
        );
        let messages: Vec<_> = diagnostics
            .iter()
            .map(|d| (d.line, d.message.as_str()))
            .collect();
        assert_eq!(
            messages,
            vec![(3, "`failure` may not import `language` (allowed: api, ui)")]
        );
    }

    const TSCONFIG: &str = r##"{"compilerOptions":{"baseUrl":".","paths":{"#/*":["./src/*"]}}}"##;

    #[test]
    fn alias_and_relative_imports_share_a_verdict() {
        let toml = format!("{MATRIX_HEADER}failure = []\nlanguage = []\n");
        let diagnostics = run_in_project(
            &toml,
            &[
                ("tsconfig.json", TSCONFIG),
                ("src/language/c.ts", "export const c = 1;\n"),
                (
                    "src/failure/a.ts",
                    "import { c } from '#/language/c';\nimport { c as c2 } from '../language/c';\n",
                ),
            ],
            "src/failure/a.ts",
        );
        let messages: Vec<_> = diagnostics
            .iter()
            .map(|d| (d.line, d.message.as_str()))
            .collect();
        let message = "`failure` may not import `language` (allowed: none)";
        assert_eq!(messages, vec![(1, message), (2, message)]);
    }

    #[test]
    fn wildcard_feature_is_judged_on_family_line() {
        let toml = "[rules.module-dependencies]\nroot = \"src\"\nwildcard = \"features/*\"\n\n[rules.module-dependencies.modules]\n\"features/*\" = [\"ui\"]\nui = []\n";
        let diagnostics = run_in_project(
            toml,
            &[
                ("src/features/b/y.ts", "export const y = 1;\n"),
                ("src/features/a/z.ts", "export const z = 1;\n"),
                ("src/ui/u.ts", "export const u = 1;\n"),
                (
                    "src/features/a/x.ts",
                    "import { y } from '../b/y';\nimport { z } from './z';\nimport { u } from '../../ui/u';\n",
                ),
            ],
            "src/features/a/x.ts",
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "`features/a` may not import `features/b` (allowed: ui)"
        );
    }

    #[test]
    fn undeclared_module_is_refused_both_ways() {
        let toml = format!("{MATRIX_HEADER}ui = []\nfailure = [\"api\"]\napi = []\n");
        let files = [
            ("src/ui/u.ts", "export const u = 1;\n"),
            ("src/newfolder/y.ts", "export const y = 1;\n"),
            (
                "src/newfolder/x.ts",
                "import { u } from '../ui/u';\nimport { y } from './y';\n",
            ),
            ("src/failure/a.ts", "import { x } from '../newfolder/x';\n"),
        ];
        for target in ["src/newfolder/x.ts", "src/failure/a.ts"] {
            let diagnostics = run_in_project(&toml, &files, target);
            assert_eq!(diagnostics.len(), 1);
            assert!(diagnostics[0].message.contains("`newfolder`"));
            assert!(diagnostics[0].message.contains("not declared"));
        }
    }

    #[test]
    fn reexports_and_dynamic_import_are_judged() {
        let toml = format!(
            "{MATRIX_HEADER}failure = [\"shared\"]\nshared = []\nlanguage = []\nui = []\napi = []\n"
        );
        let diagnostics = run_in_project(
            &toml,
            &[
                ("src/language/c.ts", "export const c = 1;\n"),
                ("src/ui/u.ts", "export const k = 1;\n"),
                ("src/api/p.ts", "export const p = 1;\n"),
                (
                    "src/failure/a.ts",
                    "export * from '../language/c';\nexport { k } from '../ui/u';\nimport('../api/p');\n",
                ),
            ],
            "src/failure/a.ts",
        );
        assert_eq!(diagnostics.len(), 3);
        for target in ["language", "ui", "api"] {
            let named = diagnostics
                .iter()
                .filter(|d| d.message.contains(&format!("`{target}`")))
                .count();
            assert_eq!(named, 1);
        }
        assert!(
            diagnostics
                .iter()
                .all(|d| d.message.contains("(allowed: shared)"))
        );
    }

    #[test]
    fn imports_outside_any_module_are_ignored() {
        let toml = format!("{MATRIX_HEADER}failure = []\nlanguage = []\n");
        let files = [
            ("lib/x.ts", "export const x = 1;\n"),
            ("src/main.ts", "import { c } from './language/c';\n"),
            ("src/language/c.ts", "export const c = 1;\n"),
            (
                "src/failure/a.ts",
                "import { x } from '../../lib/x';\nimport React from 'react';\nimport { m } from '../main';\n",
            ),
        ];
        for target in ["src/failure/a.ts", "src/main.ts"] {
            assert!(run_in_project(&toml, &files, target).is_empty());
        }
    }

    #[test]
    fn no_finding_without_config() {
        let diagnostics = run_in_project(
            "",
            &[
                ("src/language/c.ts", "export const c = 1;\n"),
                ("src/failure/a.ts", "import { c } from '../language/c';\n"),
            ],
            "src/failure/a.ts",
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    #[should_panic(expected = "modules.failure")]
    fn non_list_module_line_panics() {
        let toml = format!("{MATRIX_HEADER}failure = \"api\"\n");
        run_in_project(
            &toml,
            &[
                ("src/language/c.ts", "export const c = 1;\n"),
                ("src/failure/a.ts", "import { c } from '../language/c';\n"),
            ],
            "src/failure/a.ts",
        );
    }

    #[test]
    #[should_panic(expected = "root `scr` is not a directory")]
    fn missing_root_panics() {
        let toml = "[rules.module-dependencies]\nroot = \"scr\"\n\n[rules.module-dependencies.modules]\nfailure = []\n";
        run_in_project(
            toml,
            &[
                ("src/language/c.ts", "export const c = 1;\n"),
                ("src/failure/a.ts", "import { c } from '../language/c';\n"),
            ],
            "src/failure/a.ts",
        );
    }
}
