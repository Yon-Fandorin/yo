use crate::{git, test_support::TestRepository};

pub(super) fn commit(repository: &TestRepository) -> String {
    repository.git(["add", "."]);
    repository.git(["commit", "--quiet", "-m", "base"]);
    git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned()
}

pub(super) fn contract(slice: &str, base: &str, path: &str, contract: &str) -> String {
    contract_for_ref(slice, base, "refs/heads/develop", path, contract)
}

pub(super) fn contract_for_ref(
    slice: &str,
    base: &str,
    base_ref: &str,
    path: &str,
    contract: &str,
) -> String {
    format!(
        r#"{{
  "schema": "yo.slice-contract/v1",
  "slice": "{slice}",
  "base": "{base}",
  "base_ref": "{base_ref}",
  "owned_contracts": ["{contract}"],
  "dependencies": [],
  "allowed_write_set": ["{path}"],
  "focused_checks": ["cargo test -p owner"],
  "slice_close_checks": ["hk check"]
}}"#
    )
}
