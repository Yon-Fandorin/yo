use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
};

use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::repository::{GitRepository, failure_json, success_json};

const ACTIVE_IDS: &[&str] = &["tui.context.base", "tui.context.large", "tui.context.small"];

pub(super) fn active_repository() -> GitRepository {
    let repository = GitRepository::from_fixture("context-active");
    repository.approve_units(ACTIVE_IDS);
    repository.git(&["add", "methexis"]);
    repository.git(&["commit", "-m", "approve context fixture"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);
    repository.integrate_active_checkpoint_roots(&["tui.context.large", "tui.context.small"]);
    repository
}

pub(super) fn direct_request(
    repository: &GitRepository,
    kind: &str,
    value: &str,
    max_tokens: usize,
) -> PathBuf {
    repository.request(
        "context.json",
        &json!({
            "schema": "methexis.context-request/v1alpha1",
            "anchors": [{"kind": kind, "value": value}],
            "tokenizer_profile": "o200k_base/v1",
            "max_tokens": max_tokens
        }),
    )
}

pub(super) fn candidate_request(
    repository: &GitRepository,
    candidates: &[(&str, u64)],
    max_tokens: usize,
    pretty: bool,
) -> PathBuf {
    let candidate_file = write_candidates(repository, candidates, pretty);
    let bytes = fs::read(&candidate_file).unwrap();
    let relative = candidate_file
        .strip_prefix(&repository.path)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    repository.request(
        if pretty {
            "candidate-context-pretty.json"
        } else {
            "candidate-context.json"
        },
        &json!({
            "schema": "methexis.context-request/v1alpha1",
            "candidates": {
                "path": relative,
                "hash": digest(&bytes)
            },
            "tokenizer_profile": "o200k_base/v1",
            "max_tokens": max_tokens
        }),
    )
}

pub(super) fn resolve(repository: &GitRepository, request: &Path) -> Value {
    success_json(repository.run(&["resolve-context", request.to_str().unwrap()]))
}

pub(super) fn resolve_failure(repository: &GitRepository, request: &Path) -> Value {
    failure_json(repository.run(&["resolve-context", request.to_str().unwrap()]))
}

pub(super) fn raw_resolve(repository: &GitRepository, request: &Path) -> Output {
    repository.run(&["resolve-context", request.to_str().unwrap()])
}

fn write_candidates(
    repository: &GitRepository,
    candidates: &[(&str, u64)],
    pretty: bool,
) -> PathBuf {
    let candidates = candidates
        .iter()
        .map(|(id, score)| Candidate {
            id: (*id).to_owned(),
            path: format!("methexis/knowledge/{}.md", id.replace('.', "-")),
            score: *score,
            reasons: vec![Reason::QueryPhrase {
                field: "body",
                score: *score,
            }],
        })
        .collect::<Vec<_>>();
    let request_hash = tagged([0x11; 32]);
    let catalog_hash = tagged([0x22; 32]);
    let compiler = "librarian/fixture";
    let candidate_bytes = serde_json::to_vec(&candidates).unwrap();
    let mut identity = StableHasher::new(b"librarian.candidate-set/v1alpha1");
    identity.part(b"request_hash", request_hash.as_bytes());
    identity.part(b"catalog_hash", catalog_hash.as_bytes());
    identity.part(b"compiler", compiler.as_bytes());
    identity.part(b"candidates", &candidate_bytes);
    let set = CandidateSet {
        schema: "librarian.candidate-set/v1alpha1",
        ok: true,
        candidate_set_id: identity.finish(),
        request_hash,
        catalog_hash,
        compiler,
        candidates,
        unresolved_anchors: Vec::new(),
        truncated: 0,
    };
    let directory = repository.path.join(".local-exclude/methexis/candidates");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(if pretty {
        "candidates-pretty.json"
    } else {
        "candidates.json"
    });
    let mut bytes = if pretty {
        serde_json::to_vec_pretty(&set).unwrap()
    } else {
        serde_json::to_vec(&set).unwrap()
    };
    bytes.push(b'\n');
    fs::write(&path, bytes).unwrap();
    path
}

#[derive(Serialize)]
struct CandidateSet<'a> {
    schema: &'static str,
    ok: bool,
    candidate_set_id: String,
    request_hash: String,
    catalog_hash: String,
    compiler: &'a str,
    candidates: Vec<Candidate>,
    unresolved_anchors: Vec<Anchor>,
    truncated: usize,
}

#[derive(Serialize)]
struct Candidate {
    id: String,
    path: String,
    score: u64,
    reasons: Vec<Reason>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Reason {
    QueryPhrase { field: &'static str, score: u64 },
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Anchor {}

struct StableHasher(Sha256);

impl StableHasher {
    fn new(domain: &[u8]) -> Self {
        let mut hasher = Self(Sha256::new());
        hasher.part(b"domain", domain);
        hasher
    }

    fn part(&mut self, label: &[u8], value: &[u8]) {
        self.0.update((label.len() as u64).to_be_bytes());
        self.0.update(label);
        self.0.update((value.len() as u64).to_be_bytes());
        self.0.update(value);
    }

    fn finish(self) -> String {
        tagged(self.0.finalize())
    }
}

fn digest(bytes: &[u8]) -> String {
    tagged(Sha256::digest(bytes))
}

fn tagged(bytes: impl AsRef<[u8]>) -> String {
    let mut output = String::from("sha256:");
    for byte in bytes.as_ref() {
        use std::fmt::Write as _;
        write!(output, "{byte:02x}").unwrap();
    }
    output
}
