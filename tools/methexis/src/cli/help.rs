use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, Write},
    process::ExitCode,
};

use serde::Serialize;

use super::output::{write_json, write_text};

const HELP: &str = concat!(
    "methexis ",
    env!("CARGO_PKG_VERSION"),
    "
Methexis SOT Pilot

USAGE:
    methexis [--help | --version]
    methexis capabilities
    methexis check [--only <class>[,<class>...]]... [--summary] [--unit <id>]
    methexis check --staged-activation
    methexis author-revision <request.json>
    methexis project-review <request.json>
    methexis build-review <request.json>
    methexis prepare-approval <manifest.json> --reviewer <owner-id> [--replace-current]
    methexis prepare-approval --canonical <knowledge-id> --revision <sha256:revision> --reviewer <owner-id> [--replace-current]
    methexis approve <request.json>
    methexis prepare-checkpoint
    methexis create-checkpoint <request.json>
    methexis prepare-activation <create-output.json>
    methexis propose-activation <request.json>
    methexis refresh-context-manifests <activation-request.json>
    methexis resolve-context <request.json>
    methexis resolve-activation-review-context <activation-request.json> <context-request.json>
    methexis verify-context-build <request.json> <sha256:BuildId>

COMMANDS:
    capabilities      Report complete supported workflow profiles
    check             Validate current SOT integrity or one exact staged activation
    author-revision   Author a derived unit revision as tracked Draft proposals
    project-review    Write a tracked Korean review Projection
    build-review      Build a local human-review packet
    prepare-approval  Emit a Projection or canonical-basis approval request
    approve           Record a human-authorized approval proposal
    prepare-checkpoint Emit a Checkpoint request from the active roots
    create-checkpoint Create an immutable trusted-revision Checkpoint proposal
    prepare-activation Emit an activation request from create-checkpoint output
    propose-activation Propose the active Checkpoint with compare-and-swap
    refresh-context-manifests Refresh registered manifests for an activation proposal
    resolve-context    Build or reuse deterministic token-bounded agent context
    resolve-activation-review-context Build review-only context from one activation proposal
    verify-context-build Independently reproduce and verify one managed ContextBuild

Run commands from the repository root. Mutations remain Draft proposals until
trusted integration. Check derives approval and active/degraded eligibility
from local develop, then uses current Source observations only to demote it.
",
);

#[derive(Serialize)]
struct Capabilities {
    schema: &'static str,
    capabilities: [&'static str; 2],
}

pub(super) fn run_bootstrap(
    args: &[OsString],
    stdout: &mut impl Write,
) -> Option<io::Result<ExitCode>> {
    match args {
        [] => Some(write_text(stdout, HELP, ExitCode::SUCCESS)),
        [arg] if arg == OsStr::new("--help") || arg == OsStr::new("-h") => {
            Some(write_text(stdout, HELP, ExitCode::SUCCESS))
        },
        [arg] if arg == OsStr::new("--version") || arg == OsStr::new("-V") => Some(
            writeln!(stdout, "methexis {}", env!("CARGO_PKG_VERSION")).map(|()| ExitCode::SUCCESS),
        ),
        [command] if command == OsStr::new("capabilities") => Some(write_json(
            stdout,
            &Capabilities {
                schema: "methexis.capabilities/v1",
                capabilities: [
                    "canonical-approval-on-demand-projection/v1",
                    "semantic-first-ko-on-demand/v1",
                ],
            },
            ExitCode::SUCCESS,
        )),
        _ => None,
    }
}
