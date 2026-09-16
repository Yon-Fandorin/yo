use std::collections::HashSet;

use serde_json::{Value, json};

use super::{
    super::{valid_schema, valid_value},
    budget::MAX_REPLAY_CONTRACT_BYTES,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelReplayTool {
    name: String,
    description: String,
    schema_version: String,
    parameters: Value,
}

impl ModelReplayTool {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        schema_version: impl Into<String>,
        parameters: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            schema_version: schema_version.into(),
            parameters,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    pub const fn parameters(&self) -> &Value {
        &self.parameters
    }

    pub(crate) fn is_valid(&self) -> bool {
        valid_schema(&self.name)
            && valid_value(&self.description)
            && valid_schema(&self.schema_version)
            && self.parameters.is_object()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelReplayContract {
    system_prompt: String,
    tools: Vec<ModelReplayTool>,
}

impl ModelReplayContract {
    pub fn new(system_prompt: impl Into<String>, tools: Vec<ModelReplayTool>) -> Self {
        Self {
            system_prompt: system_prompt.into(),
            tools,
        }
    }

    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    pub fn tools(&self) -> &[ModelReplayTool] {
        &self.tools
    }

    #[doc(hidden)]
    pub fn is_valid(&self) -> bool {
        let mut names = HashSet::new();
        !self.system_prompt.is_empty()
            && self.tools.len() <= 1_024
            && self.tools.iter().all(ModelReplayTool::is_valid)
            && self.tools.iter().all(|tool| names.insert(tool.name()))
            && encoded_contract_len(self) <= MAX_REPLAY_CONTRACT_BYTES
    }
}

pub(super) fn encoded_contract_len(contract: &ModelReplayContract) -> usize {
    serde_json::to_vec(&contract_value(contract))
        .expect("a replay contract is always JSON serializable")
        .len()
}

fn contract_value(contract: &ModelReplayContract) -> Value {
    json!({
        "system_prompt": contract.system_prompt,
        "tools": contract.tools.iter().map(|tool| json!({
            "name": tool.name,
            "description": tool.description,
            "schema_version": tool.schema_version,
            "parameters": tool.parameters,
        })).collect::<Vec<_>>(),
    })
}
