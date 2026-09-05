use std::{
    fs::{File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use nix::{dir::Dir, fcntl::OFlag};
use serde_json::Value;
use yo_core::{
    ToolDefinition, ToolExecution, ToolExecutionError, ToolExecutionHost, ToolExecutionRequest,
    ToolId,
};

use super::{
    command::CommandExecution,
    execution::{ThreadExecution, failed},
};

mod descriptor;
mod list;
mod mutation;
mod mutation_plan;
mod output;
mod path;
mod read;

const HOST_IDENTITY: &str = "yo.local-workspace-tools/v1";

pub(crate) fn initialize_process_file_mode() {
    descriptor::initialize_process_file_mode();
}

pub(super) fn validate_arguments(
    definition: &ToolDefinition,
    arguments: &Value,
) -> Result<(), ToolExecutionError> {
    match definition.id().as_str() {
        "list-files" => path::list_path(arguments, "path").map(drop),
        "read-files" => read::parse_requests(arguments, path::basic_path).map(drop),
        "edit-file" => mutation::parse_edit(arguments, path::basic_path).map(drop),
        "write-file" => mutation::parse_write(arguments, path::basic_path).map(drop),
        _ => Ok(()),
    }
}

pub(crate) struct LocalToolHost {
    workspace: PathBuf,
    workspace_directory: File,
    denied_credential: Option<descriptor::FileIdentity>,
    mutation_lock: Arc<Mutex<()>>,
    new_file_mode: u32,
}

impl LocalToolHost {
    pub(crate) fn new(
        workspace: &Path,
        credential_path: &Path,
    ) -> Result<Self, ToolExecutionError> {
        let workspace_directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(workspace)
            .map_err(|_| ToolExecutionError::new("workspace cannot be opened safely"))?;
        let workspace = workspace
            .canonicalize()
            .map_err(|_| ToolExecutionError::new("workspace cannot be canonicalized"))?;
        let denied_credential = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(credential_path)
            .ok()
            .and_then(|file| descriptor::file_identity(&file));
        Ok(Self {
            workspace,
            workspace_directory,
            denied_credential,
            mutation_lock: Arc::new(Mutex::new(())),
            new_file_mode: descriptor::new_file_mode(),
        })
    }

    fn open_directory(&self, value: &str) -> Result<(Dir, PathBuf), ToolExecutionError> {
        let components = path::admitted_path_components(value)?;
        let relative = components.iter().collect();
        let descriptor = descriptor::open_beneath(
            &self.workspace_directory,
            &components,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY,
        )?;
        let directory = Dir::from_fd(descriptor)
            .map_err(|_| ToolExecutionError::new("list_files requires a directory"))?;
        Ok((directory, relative))
    }
}

impl ToolExecutionHost for LocalToolHost {
    fn identity(&self) -> &str {
        HOST_IDENTITY
    }

    fn is_available(&self, tool: &ToolId) -> bool {
        matches!(
            tool.as_str(),
            "read-file" | "list-files" | "read-files" | "edit-file" | "write-file" | "run-command"
        )
    }

    fn start(
        &mut self,
        request: ToolExecutionRequest,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
        let maximum_output_bytes = request.maximum_output_bytes;
        match request.call.definition().id().as_str() {
            "read-file" => {
                let path = path::string_argument(request.call.arguments(), "path")?;
                let components = path::path_components(path)?;
                if components.is_empty() {
                    return Err(ToolExecutionError::new(
                        "read_file path must not name the workspace root",
                    ));
                }
                let workspace = self
                    .workspace_directory
                    .try_clone()
                    .map_err(|_| ToolExecutionError::new("workspace handle is unavailable"))?;
                let denied = self.denied_credential;
                Ok(Box::new(ThreadExecution::spawn(move |cancelled| {
                    let Ok(file) = descriptor::open_regular_file(&workspace, &components, denied)
                    else {
                        return failed("tool execution failed");
                    };
                    read::read_file(file, maximum_output_bytes, &cancelled)
                })))
            },
            "list-files" => {
                let path = path::string_argument(request.call.arguments(), "path")?;
                let (directory, relative) = self.open_directory(path)?;
                Ok(Box::new(ThreadExecution::spawn(move |cancelled| {
                    list::list_files(directory, relative, maximum_output_bytes, &cancelled)
                })))
            },
            "run-command" => {
                let command =
                    path::string_argument(request.call.arguments(), "command")?.to_owned();
                Ok(Box::new(CommandExecution::spawn(
                    self.workspace.clone(),
                    command,
                    maximum_output_bytes,
                    request.absolute_execution_timeout,
                )?))
            },
            "read-files" => {
                let files = read::parse_requests(request.call.arguments(), path::basic_path)?;
                let workspace = self
                    .workspace_directory
                    .try_clone()
                    .map_err(|_| ToolExecutionError::new("workspace handle is unavailable"))?;
                let denied = self.denied_credential;
                Ok(Box::new(ThreadExecution::spawn(move |cancelled| {
                    read::execute(workspace, denied, files, &cancelled)
                })))
            },
            "edit-file" => {
                let edit = mutation::parse_edit(request.call.arguments(), path::basic_path)?;
                let result_path = edit.path().to_owned();
                let workspace = self
                    .workspace_directory
                    .try_clone()
                    .map_err(|_| ToolExecutionError::new("workspace handle is unavailable"))?;
                let denied = self.denied_credential;
                let lock = Arc::clone(&self.mutation_lock);
                Ok(Box::new(ThreadExecution::spawn(move |cancelled| {
                    let cleanup = mutation::UnwindCleanup::default();
                    mutation::catch_failure(&result_path, &cleanup, || {
                        mutation::execute_edit(
                            workspace,
                            denied,
                            lock,
                            edit,
                            &cancelled,
                            cleanup.clone(),
                        )
                    })
                })))
            },
            "write-file" => {
                let write = mutation::parse_write(request.call.arguments(), path::basic_path)?;
                let result_path = write.path().to_owned();
                let workspace = self
                    .workspace_directory
                    .try_clone()
                    .map_err(|_| ToolExecutionError::new("workspace handle is unavailable"))?;
                let denied = self.denied_credential;
                let lock = Arc::clone(&self.mutation_lock);
                let mode = self.new_file_mode;
                Ok(Box::new(ThreadExecution::spawn(move |cancelled| {
                    let cleanup = mutation::UnwindCleanup::default();
                    mutation::catch_failure(&result_path, &cleanup, || {
                        mutation::execute_write(
                            workspace,
                            denied,
                            lock,
                            write,
                            mode,
                            &cancelled,
                            cleanup.clone(),
                        )
                    })
                })))
            },
            _ => Err(ToolExecutionError::new("unknown local tool")),
        }
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
