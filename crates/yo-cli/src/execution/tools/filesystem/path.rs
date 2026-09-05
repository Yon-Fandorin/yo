use std::{
    ffi::OsString,
    path::{Component, Path},
};

use serde_json::Value;
use yo_core::ToolExecutionError;

const MAX_PATH_BYTES: usize = 1_024;

#[derive(Clone, Debug)]
pub(super) struct AdmittedPath {
    display: String,
    components: Vec<OsString>,
}

impl AdmittedPath {
    pub(super) const fn new(display: String, components: Vec<OsString>) -> Self {
        Self {
            display,
            components,
        }
    }

    pub(super) fn display(&self) -> &str {
        &self.display
    }

    pub(super) fn components(&self) -> &[OsString] {
        &self.components
    }
}

pub(super) fn list_path(
    arguments: &Value,
    name: &str,
) -> Result<Vec<OsString>, ToolExecutionError> {
    admitted_path_components(string_argument(arguments, name)?)
}

pub(super) fn basic_path(value: &str) -> Result<AdmittedPath, ToolExecutionError> {
    let components = admitted_path_components(value)?;
    if components.is_empty() {
        return Err(ToolExecutionError::new(
            "tool file path must not name the workspace root",
        ));
    }
    Ok(AdmittedPath::new(value.to_owned(), components))
}

pub(super) fn path_components(value: &str) -> Result<Vec<OsString>, ToolExecutionError> {
    if value.is_empty()
        || Path::new(value).is_absolute()
        || Path::new(value).components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ToolExecutionError::new(
            "tool path must be a non-empty workspace-relative path without parent traversal",
        ));
    }
    Ok(Path::new(value)
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_owned()),
            Component::CurDir => None,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                unreachable!("invalid components were rejected")
            },
        })
        .collect())
}

pub(super) fn admitted_path_components(value: &str) -> Result<Vec<OsString>, ToolExecutionError> {
    if value.len() > MAX_PATH_BYTES || value.chars().any(char::is_control) {
        return Err(ToolExecutionError::new(
            "tool path exceeds its byte bound or contains a control character",
        ));
    }
    path_components(value)
}

pub(super) fn string_argument<'a>(
    value: &'a Value,
    name: &str,
) -> Result<&'a str, ToolExecutionError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolExecutionError::new("validated local tool argument is unavailable"))
}
