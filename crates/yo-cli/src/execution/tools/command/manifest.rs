//! Frozen command definitions shared by startup admission and approved execution.

mod artifact;
mod encoding;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use artifact::{Artifact, VerificationPass};
use serde_json::{Value, json};
use yo_core::{
    FrozenToolRegistry, ModelReplayContract, ToolDefinition, ToolExecutionError, ToolId,
    ToolRegistry,
};

use super::super::registry::{LocalToolRegistryRevision, registry};
use crate::state::config::CommandToolConfig;

const REGISTRY_PROFILE: &str = "yo.local-tool-registry/command-tools/v1";
const MANIFEST_PROFILE: &str = "yo.execution-definition-manifest/v1";

#[derive(Clone)]
pub(crate) struct PreparedCommandTools {
    registry: FrozenToolRegistry,
    commands: Arc<[PreparedCommand]>,
    digest: String,
    host_identity: String,
}

impl PreparedCommandTools {
    /// Verifies only the model-visible projection, before any configured artifact is opened.
    pub(crate) fn validate_replay_contract(
        commands: &[CommandToolConfig],
        contract: Option<&ModelReplayContract>,
    ) -> Result<(), ToolExecutionError> {
        if commands.is_empty() || commands.len() > 16 {
            return Err(invalid(
                "saved command tools require explicit configuration",
            ));
        }
        let builtins = registry(LocalToolRegistryRevision::BasicFiles)?.freeze();
        let registry = configured_registry(&builtins, commands)?;
        if contract.is_none_or(|contract| contract.tools() != registry.replay_tools()) {
            return Err(invalid(
                "configured command tool projection does not match the saved Session",
            ));
        }
        Ok(())
    }

    /// Captures configured artifacts once; empty lists perform no filesystem access.
    pub(crate) fn prepare(
        commands: &[CommandToolConfig],
        workspace: &Path,
        credential_path: &Path,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Option<Self>, ToolExecutionError> {
        if commands.is_empty() {
            return Ok(None);
        }
        if commands.len() > 16 {
            return Err(invalid("command tool count exceeds its bound"));
        }
        let mut pass = VerificationPass::startup(cancelled, None);
        let workspace = pass.workspace(workspace)?;
        let denied = pass.credential(credential_path)?;
        let builtins = registry(LocalToolRegistryRevision::BasicFiles)?.freeze();
        let registry = configured_registry(&builtins, commands)?;
        let mut prepared = Vec::with_capacity(commands.len());
        for command in commands {
            let executable = pass.capture(command.executable(), &workspace, true, denied)?;
            let script = command
                .script()
                .map(|script| pass.capture(script, &workspace, false, denied))
                .transpose()?;
            prepared.push(PreparedCommand {
                definition: command.definition().clone(),
                executable,
                script,
                executable_args: command.executable_args().to_vec(),
                argv: command.argv().to_vec(),
                workspace: workspace.clone(),
                credential_path: credential_path.to_owned(),
            });
        }
        pass.check()?;
        let mut tools = builtins
            .definitions()
            .iter()
            .map(|definition| manifest_tool(definition, Value::Null))
            .collect::<Vec<_>>();
        tools.extend(
            prepared
                .iter()
                .map(|command| manifest_tool(&command.definition, command.launch_manifest())),
        );
        let digest = encoding::digest(&json!({
            "profile": MANIFEST_PROFILE,
            "registry": REGISTRY_PROFILE,
            "tools": tools,
            "protocols": {
                "stdin": "yo.command-json-stdin/v1",
                "output": "yo.command-text-output/v1",
                "environment": "yo.command-safe-environment/v1",
                "runner": "yo.command-execution/v1",
            },
        }))?;
        pass.check()?;
        Ok(Some(Self {
            registry,
            commands: prepared.into(),
            host_identity: format!("yo.local-workspace-tools/command-tools/v1/{digest}"),
            digest,
        }))
    }

    pub(crate) fn registry(&self) -> &FrozenToolRegistry {
        &self.registry
    }

    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }

    pub(crate) fn host_identity(&self) -> &str {
        &self.host_identity
    }

    /// Frozen lookup only; availability never resolves or hashes an artifact.
    pub(crate) fn command(&self, id: &ToolId) -> Option<PreparedCommand> {
        self.commands
            .iter()
            .find(|command| command.definition.id() == id)
            .cloned()
    }
}

#[derive(Clone)]
pub(crate) struct PreparedCommand {
    definition: ToolDefinition,
    executable: Artifact,
    script: Option<Artifact>,
    executable_args: Vec<String>,
    argv: Vec<String>,
    workspace: PathBuf,
    credential_path: PathBuf,
}

impl PreparedCommand {
    pub(crate) fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    /// One approved worker's final verification; expected hashes are never refreshed.
    pub(crate) fn verify_for_launch(
        &self,
        cancelled: &mut dyn FnMut() -> bool,
        absolute_deadline: Option<Instant>,
    ) -> Result<VerifiedCommandLaunch, ToolExecutionError> {
        let mut pass = VerificationPass::call(cancelled, absolute_deadline);
        let denied = pass.credential(&self.credential_path)?;
        let executable =
            pass.capture(&self.executable.configured, &self.workspace, true, denied)?;
        let script = self
            .script
            .as_ref()
            .map(|script| pass.capture(&script.configured, &self.workspace, false, denied))
            .transpose()?;
        if executable != self.executable || script != self.script {
            return Err(invalid("command tool artifact changed"));
        }
        let mut args = self.executable_args.clone();
        if let Some(script) = &script {
            args.push(script.resolved.clone());
        }
        args.extend(self.argv.iter().cloned());
        pass.check()?;
        Ok(VerifiedCommandLaunch {
            executable: self.executable.configured.clone(),
            args,
        })
    }

    fn launch_manifest(&self) -> Value {
        json!({
            "executable": self.executable.manifest(),
            "script": self.script.as_ref().map(Artifact::manifest),
            "executable_args": self.executable_args,
            "argv": self.argv,
        })
    }
}

/// Literal configured executable and ordered fixed argv, admitted for one immediate spawn.
pub(crate) struct VerifiedCommandLaunch {
    pub(crate) executable: String,
    pub(crate) args: Vec<String>,
}

fn manifest_tool(definition: &ToolDefinition, launch: Value) -> Value {
    use yo_core::{ToolApprovalRequirement, ToolEffect};
    json!({
        "id": definition.id().as_str(),
        "name": definition.wire_name(),
        "description": definition.description(),
        "schema_version": definition.schema_version(),
        "parameters": definition.input_schema(),
        "effect": match definition.effect() {
            ToolEffect::ReadOnly => "ReadOnly",
            ToolEffect::WorkspaceWrite => "WorkspaceWrite",
            ToolEffect::Process => "Process",
        },
        "approval": match definition.approval() {
            ToolApprovalRequirement::Automatic => "Automatic",
            ToolApprovalRequirement::Required => "Required",
        },
        "launch": launch,
    })
}

fn configured_registry(
    builtins: &FrozenToolRegistry,
    commands: &[CommandToolConfig],
) -> Result<FrozenToolRegistry, ToolExecutionError> {
    let mut definitions = builtins.definitions().to_vec();
    definitions.extend(commands.iter().map(|command| command.definition().clone()));
    ToolRegistry::new(definitions)
        .map(|registry| registry.freeze())
        .map_err(|_| invalid("command tool registry is invalid"))
}

fn invalid(message: &'static str) -> ToolExecutionError {
    ToolExecutionError::new(message)
}

#[cfg(test)]
mod tests;
