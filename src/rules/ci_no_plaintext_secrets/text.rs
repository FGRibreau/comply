//! Flag secret-like keys with literal values in CI workflow files.
//! `${{ secrets.* }}` references are allowed.
//! Other YAML is out of scope.
//! A Kubernetes quota `secrets: "120"` is a count, not a credential.

use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::yaml_k8s_helpers::{pair_key_text, pair_scalar_value};
use std::path::Path;

/// Key fragments that indicate a secret-bearing value. Matched case-insensitively
/// as a substring so `DB_PASSWORD`, `API_TOKEN`, `GITHUB_TOKEN`, `SECRET_KEY`
/// and friends all trigger.
const SECRET_KEY_FRAGMENTS: &[&str] = &[
    "password",
    "passwd",
    "token",
    "secret",
    "api_key",
    "apikey",
    "private_key",
];

fn key_looks_like_secret(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_KEY_FRAGMENTS.iter().any(|frag| lower.contains(frag))
}

/// GitHub Actions workflows: `.github/workflows/*.yml`.
/// GitLab CI: `.gitlab-ci.yml` and its local includes `*.gitlab-ci.yml`.
fn is_ci_workflow(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false; };
    let name = name.to_ascii_lowercase();
    if !name.ends_with(".yml") && !name.ends_with(".yaml") { return false; }
    if name.ends_with(".gitlab-ci.yml") || name.ends_with(".gitlab-ci.yaml") { return true; }
    let mut dirs = path.parent().into_iter().flat_map(Path::components).rev();
    dirs.next().is_some_and(|d| d.as_os_str() == "workflows")
        && dirs.next().is_some_and(|d| d.as_os_str() == ".github")
}

fn is_secret_reference(value: &str) -> bool {
    // Allow `${{ secrets.X }}`, `${{ vars.X }}`, or any `${{ ... }}` expression —
    // these are resolved at runtime and don't commit a literal.
    value.contains("${{")
}

crate::ast_check! { on ["block_mapping_pair"] => |node, source, ctx, diagnostics|
    if !is_ci_workflow(ctx.path) { return; }
    let Some(key) = pair_key_text(node, source) else { return; };
    if !key_looks_like_secret(&key) { return; }
    let Some(value) = pair_scalar_value(node, source) else { return; };
    let value = value.split('#').next().unwrap_or("").trim();
    // A blank value means the actual scalar lives on the next line
    // (block scalar). We conservatively skip — avoids multi-line parsing.
    if value.is_empty() { return; }
    if is_secret_reference(value) { return; }

    let key_node = node.named_child(0).unwrap_or(node);
    let pos = key_node.start_position();
    diagnostics.push(Diagnostic {
        path: std::sync::Arc::clone(&ctx.path_arc),
        line: pos.row + 1,
        column: pos.column + 1,
        rule_id: "ci-no-plaintext-secrets".into(),
        message: format!(
            "`{key}` has a literal value — reference `${{{{ secrets.{} }}}}` instead.",
            key.to_ascii_uppercase()
        ),
        severity: Severity::Error,
        span: Some((key_node.byte_range().start, key_node.byte_range().len())),
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
        crate::rules::test_helpers::run_ast_check(self, src, path, project, file)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Diagnostic;
    
    fn run(source: &str) -> Vec<Diagnostic> {
        crate::rules::test_helpers::run_rule(&Check, source, ".github/workflows/ci.yml")
    }

    #[test]
    fn flags_literal_token() {
        let yaml = "\
on: push
jobs:
  build:
    env:
      GITHUB_TOKEN: ghp_abcdef1234567890
";
        assert_eq!(run(yaml).len(), 1);
    }

    #[test]
    fn flags_literal_api_key_in_with() {
        let yaml = "\
on: push
jobs:
  build:
    steps:
      - uses: some/action@v1
        with:
          api_key: sk_live_12345
";
        assert_eq!(run(yaml).len(), 1);
    }

    #[test]
    fn allows_secret_expression() {
        let yaml = "\
on: push
jobs:
  build:
    env:
      GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
";
        assert!(run(yaml).is_empty());
    }

    fn run_at(source: &str, path: &str) -> Vec<Diagnostic> {
        crate::rules::test_helpers::run_rule(&Check, source, path)
    }

    /// A Capsule `Tenant` quota caps the number of Secret objects.
    /// That is a count, not a credential.
    #[test]
    fn ignores_secret_shaped_key_outside_ci_workflow() {
        let yaml = "\
apiVersion: capsule.clastix.io/v1beta2
kind: Tenant
metadata:
  name: acme
spec:
  resourceQuotas:
    items:
      - hard:
          secrets: \"120\"
          configmaps: \"100\"
";
        assert!(run_at(yaml, "rbac/acme/00-tenant.yaml").is_empty());
        assert!(run_at(yaml, ".github/dependabot.yml").is_empty());
    }

    #[test]
    fn flags_literal_token_in_github_workflow_yaml_extension() {
        let yaml = "on: push\njobs:\n  build:\n    env:\n      API_TOKEN: abc123\n";
        assert_eq!(run_at(yaml, ".github/workflows/release.yaml").len(), 1);
    }

    #[test]
    fn flags_literal_token_in_gitlab_ci() {
        let yaml = "\
deploy:
  variables:
    DEPLOY_TOKEN: glpat-abcdef123456
  script:
    - ./deploy.sh
";
        assert_eq!(run_at(yaml, ".gitlab-ci.yml").len(), 1);
        assert_eq!(run_at(yaml, "ci/deploy.gitlab-ci.yml").len(), 1);
    }

    #[test]
    fn ignores_non_secret_keys() {
        let yaml = "on: push\njobs:\n  build:\n    env:\n      NODE_VERSION: 20";
        assert!(run(yaml).is_empty());
    }
}
