//! require-readable-spacing OXC backend.

use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::backend::{CheckCtx, OxcCheck};
use oxc_ast::AstKind;
use oxc_ast::ast::{Comment, Declaration, ExportDefaultDeclarationKind, Statement};
use oxc_span::{GetSpan, Span};
use std::sync::Arc;

pub struct Check;

impl OxcCheck for Check {
    fn run_on_semantic<'a>(
        &self,
        semantic: &'a oxc_semantic::Semantic<'a>,
        ctx: &CheckCtx,
    ) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for node in semantic.nodes().iter() {
            let statements = match node.kind() {
                AstKind::Program(program) => &program.body,
                AstKind::FunctionBody(body) => &body.statements,
                AstKind::BlockStatement(block) => &block.body,
                AstKind::StaticBlock(block) => &block.body,
                AstKind::SwitchCase(case) => &case.consequent,
                AstKind::TSModuleBlock(block) => &block.body,
                _ => continue,
            };
            for pair in statements.windows(2) {
                let (previous, next) = (&pair[0], &pair[1]);
                if !needs_blank_line(group(previous, ctx.source), group(next, ctx.source))
                    || has_blank_line(
                        Span::new(previous.span().end, next.span().start),
                        ctx.source,
                        semantic.comments(),
                    )
                {
                    continue;
                }
                let span = next.span();
                diagnostics.push(Diagnostic::at_offset(
                    Arc::clone(&ctx.path_arc),
                    ctx.source,
                    (span.start as usize, span.size() as usize),
                    super::META.id,
                    "Expected a blank line before this statement.".into(),
                    Severity::Error,
                ));
            }
        }
        diagnostics
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Group {
    Declaration,
    Exit,
    MultilineBlock,
    Other,
}

fn needs_blank_line(previous: Group, next: Group) -> bool {
    match (previous, next) {
        (Group::MultilineBlock, _) | (_, Group::MultilineBlock | Group::Exit) => true,
        (Group::Declaration, next) => next != Group::Declaration,
        _ => false,
    }
}

fn group(statement: &Statement, source: &str) -> Group {
    let block = if statement.span().source_text(source).contains('\n') {
        Group::MultilineBlock
    } else {
        Group::Other
    };
    match statement {
        Statement::VariableDeclaration(_) => Group::Declaration,
        Statement::ReturnStatement(_) | Statement::ThrowStatement(_) => Group::Exit,
        Statement::IfStatement(_)
        | Statement::ForStatement(_)
        | Statement::ForInStatement(_)
        | Statement::ForOfStatement(_)
        | Statement::WhileStatement(_)
        | Statement::DoWhileStatement(_)
        | Statement::SwitchStatement(_)
        | Statement::TryStatement(_)
        | Statement::FunctionDeclaration(_)
        | Statement::ClassDeclaration(_) => block,
        Statement::ExportNamedDeclaration(export) => match &export.declaration {
            Some(Declaration::VariableDeclaration(_)) => Group::Declaration,
            Some(Declaration::FunctionDeclaration(_) | Declaration::ClassDeclaration(_)) => block,
            _ => Group::Other,
        },
        Statement::ExportDefaultDeclaration(export) => match &export.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(_)
            | ExportDefaultDeclarationKind::ClassDeclaration(_) => block,
            _ => Group::Other,
        },
        _ => Group::Other,
    }
}

/// True when a whitespace-only line outside comments separates the statements.
fn has_blank_line(gap: Span, source: &str, comments: &[Comment]) -> bool {
    let first = comments.partition_point(|comment| comment.span.start < gap.start);
    let inside = comments[first..]
        .iter()
        .map(|comment| comment.span)
        .take_while(|span| span.end <= gap.end);
    let mut start = gap.start;
    for comment in inside.chain([Span::new(gap.end, gap.end)]) {
        let mut lines = Span::new(start, comment.start)
            .source_text(source)
            .split('\n');
        // First and last pieces touch a statement or a comment.
        lines.next();
        lines.next_back();
        if lines.any(|line| line.trim().is_empty()) {
            return true;
        }
        start = comment.end;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::files::Language;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    const ENABLED: &str = "[rules.require-readable-spacing]\ndisabled = false\n";

    fn run_with_config(src: &str, toml: &str) -> Vec<Diagnostic> {
        let project_dir = TempDir::new().expect("tempdir");
        fs::write(project_dir.path().join("comply.toml"), toml).expect("write cfg");
        let config = Config::load_from(project_dir.path()).expect("load cfg");
        crate::engine::lint_in_memory(Path::new("t.ts"), Language::TypeScript, src, &config, None)
            .into_iter()
            .filter(|d| d.rule_id.as_ref() == super::super::META.id)
            .collect()
    }

    fn lines(src: &str) -> Vec<usize> {
        run_with_config(src, ENABLED)
            .iter()
            .map(|d| d.line)
            .collect()
    }

    #[test]
    fn declaration_then_statement_needs_blank_line() {
        assert_eq!(lines("const a = 1;\ndoIt(a);\n"), [2]);
        assert!(lines("const a = 1;\n\ndoIt(a);\n").is_empty());
        assert!(lines("const a = 1;\r\n\r\ndoIt(a);\r\n").is_empty());
    }

    #[test]
    fn not_enabled_flags_nothing() {
        let src = "const a = 1;\ndoIt(a);\nx();\nif (a) {\n  b();\n}\nfunction f() {\n  x();\n  return y;\n}\n";
        assert!(run_with_config(src, "").is_empty());
    }

    #[test]
    fn return_and_throw_need_blank_line() {
        assert_eq!(lines("function f() {\n  x();\n  return y;\n}\n"), [3]);
        assert_eq!(lines("function f() {\n  x();\n  throw e;\n}\n"), [3]);
        assert!(lines("function f() {\n  return y;\n}\n").is_empty());
        assert_eq!(
            lines("switch (a) {\n  case 1:\n    x();\n    return y;\n}\n"),
            [4]
        );
        assert_eq!(lines("namespace N {\n  x();\n  throw e;\n}\n"), [3]);
    }

    #[test]
    fn multiline_block_is_padded_on_both_sides() {
        assert_eq!(lines("x();\nif (a) {\n  b();\n}\ny();\n"), [2, 5]);
        assert_eq!(lines("x();\nfor (;;) {\n  b();\n}\ny();\n"), [2, 5]);
        assert_eq!(lines("x();\nfunction g() {\n  b();\n}\ny();\n"), [2, 5]);
        assert_eq!(
            lines("x();\ntry {\n  a();\n} catch {\n  b();\n}\ny();\n"),
            [2, 7]
        );
        assert!(lines("x();\nif (a) b();\ny();\n").is_empty());
    }

    #[test]
    fn consecutive_declarations_and_calls_pass() {
        assert!(lines("const a = 1;\nlet b = 2;\nvar c = 3;\n").is_empty());
        assert!(lines("a();\nb();\n").is_empty());
    }

    #[test]
    fn comment_line_is_not_a_blank_line() {
        assert_eq!(lines("const a = 1;\n// note\ndoIt(a);\n"), [3]);
        assert!(lines("const a = 1;\n\n// note\ndoIt(a);\n").is_empty());
        assert!(lines("const a = 1;\n// note\n\ndoIt(a);\n").is_empty());
        assert_eq!(lines("const a = 1;\n/*\n\n*/\ndoIt(a);\n"), [5]);
    }
}
