use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{model, parallel, repository_root};
use crate::{bounded_file, git, review_protocol};

const BOUNDARY_SCHEMA: &str = "yo.wave-assembly-boundary/v1alpha1";
const RESULT_SCHEMA: &str = "yo.wave-assembly-check/v1alpha1";
const INPUT_LIMIT: usize = 64 * 1024;
const MAX_COMPONENTS: usize = 16;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Boundary {
    schema: String,
    wave: String,
    base: String,
    base_ref: String,
    assembly_slice: String,
    owned_contracts: Vec<String>,
    allowed_write_set: Vec<String>,
}

#[derive(Serialize)]
struct ResultDocument {
    schema: &'static str,
    ok: bool,
    wave: String,
    base: String,
    base_ref: String,
    boundary: Artifact,
    assembly: AssemblyOwner,
    components: Vec<Component>,
    next_action: &'static str,
}

#[derive(Serialize)]
struct Artifact {
    path: String,
    hash: String,
}

#[derive(Serialize)]
struct AssemblyOwner {
    slice: String,
    owned_contracts: Vec<String>,
    allowed_write_set: Vec<String>,
}

#[derive(Serialize)]
struct Component {
    slice: String,
    contract: Artifact,
}

struct ComponentInput {
    path: PathBuf,
    bytes: Vec<u8>,
    contract: model::SliceContract,
}

pub(crate) fn check_wave_assembly(
    repository: &Path,
    boundary_path: &Path,
    component_paths: &[PathBuf],
) -> Result<(), String> {
    let result = evaluate(repository, boundary_path, component_paths)?;
    println!(
        "{}",
        serde_json::to_string(&result)
            .map_err(|error| format!("cannot encode Wave assembly check result: {error}"))?
    );
    Ok(())
}

fn evaluate(
    repository: &Path,
    boundary_path: &Path,
    component_paths: &[PathBuf],
) -> Result<ResultDocument, String> {
    if component_paths.is_empty() || component_paths.len() > MAX_COMPONENTS {
        return Err(format!(
            "Wave assembly check requires 1 to {MAX_COMPONENTS} component contracts"
        ));
    }
    let repository = repository_root(repository)?;
    let boundary_bytes =
        bounded_file::read_regular(boundary_path, INPUT_LIMIT, "Wave assembly boundary")?;
    let boundary: Boundary = serde_json::from_slice(&boundary_bytes).map_err(|error| {
        format!(
            "invalid Wave assembly boundary {}: {error}",
            boundary_path.display()
        )
    })?;
    validate_boundary(&repository, &boundary)?;

    let mut components = component_paths
        .iter()
        .map(|path| read_component(path))
        .collect::<Result<Vec<_>, _>>()?;
    components.sort_by(|left, right| left.contract.slice.cmp(&right.contract.slice));
    validate_components(&repository, &boundary, &components)?;

    let integration = integration_head(&repository, &boundary.base_ref)?;
    if integration != boundary.base {
        return Err(format!(
            "Wave assembly base {} is stale; current {} is {integration}",
            boundary.base, boundary.base_ref
        ));
    }

    require_unchanged(boundary_path, &boundary_bytes, "Wave assembly boundary")?;
    for component in &components {
        require_unchanged(
            &component.path,
            &component.bytes,
            "Wave component Slice contract",
        )?;
    }
    if integration_head(&repository, &boundary.base_ref)? != integration {
        return Err("Wave integration ref changed during assembly preflight".to_owned());
    }

    Ok(ResultDocument {
        schema: RESULT_SCHEMA,
        ok: true,
        wave: boundary.wave,
        base: boundary.base,
        base_ref: boundary.base_ref,
        boundary: Artifact {
            path: boundary_path.display().to_string(),
            hash: review_protocol::digest(&boundary_bytes),
        },
        assembly: AssemblyOwner {
            slice: boundary.assembly_slice,
            owned_contracts: boundary.owned_contracts,
            allowed_write_set: boundary.allowed_write_set,
        },
        components: components
            .into_iter()
            .map(|component| Component {
                slice: component.contract.slice,
                contract: Artifact {
                    path: component.path.display().to_string(),
                    hash: review_protocol::digest(&component.bytes),
                },
            })
            .collect(),
        next_action: "dispatch_components",
    })
}

fn validate_boundary(repository: &Path, boundary: &Boundary) -> Result<(), String> {
    if boundary.schema != BOUNDARY_SCHEMA {
        return Err(format!(
            "unsupported Wave assembly boundary schema `{}`; expected `{BOUNDARY_SCHEMA}`",
            boundary.schema
        ));
    }
    validate_segment(repository, "Wave name", &boundary.wave, true)?;
    validate_segment(
        repository,
        "assembly Slice name",
        &boundary.assembly_slice,
        false,
    )?;
    let expected_ref = format!("refs/heads/wave/{}", boundary.wave);
    if boundary.base_ref != expected_ref {
        return Err(format!(
            "Wave assembly boundary for `{}` must use base_ref `{expected_ref}`",
            boundary.wave
        ));
    }
    if boundary.owned_contracts.is_empty() || boundary.allowed_write_set.is_empty() {
        return Err("Wave assembly owner must declare contract and write ownership".to_owned());
    }

    let synthetic = model::SliceContract {
        schema: model::SCHEMA.to_owned(),
        slice: boundary.assembly_slice.clone(),
        base: boundary.base.clone(),
        base_ref: boundary.base_ref.clone(),
        owned_contracts: boundary.owned_contracts.clone(),
        dependencies: Vec::new(),
        allowed_write_set: boundary.allowed_write_set.clone(),
        focused_checks: vec!["deferred Wave assembly preflight".to_owned()],
        slice_close_checks: vec!["deferred Wave assembly close".to_owned()],
    };
    model::validate(repository, &synthetic)
}

fn validate_components(
    repository: &Path,
    boundary: &Boundary,
    components: &[ComponentInput],
) -> Result<(), String> {
    let assembly_rules = model::parse_rules(&boundary.allowed_write_set)?;
    let assembly_contracts = boundary
        .owned_contracts
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut slices = BTreeSet::new();

    for component in components {
        model::validate(repository, &component.contract)?;
        validate_segment(
            repository,
            "component Slice name",
            &component.contract.slice,
            false,
        )?;
        if component.contract.base != boundary.base
            || component.contract.base_ref != boundary.base_ref
        {
            return Err(format!(
                "component Slice `{}` does not share Wave assembly base {} at {}",
                component.contract.slice, boundary.base, boundary.base_ref
            ));
        }
        if component.contract.slice == boundary.assembly_slice {
            return Err(format!(
                "assembly owner `{}` cannot also be a component Slice",
                boundary.assembly_slice
            ));
        }
        if !slices.insert(component.contract.slice.as_str()) {
            return Err(format!(
                "duplicate Wave component Slice `{}`",
                component.contract.slice
            ));
        }

        let component_rules = model::parse_rules(&component.contract.allowed_write_set)?;
        let path_overlaps = parallel::overlaps(&assembly_rules, &component_rules);
        if !path_overlaps.is_empty() {
            return Err(format!(
                "component Slice `{}` claims write paths reserved for assembly owner `{}`:\n{}",
                component.contract.slice,
                boundary.assembly_slice,
                path_overlaps
                    .iter()
                    .map(|(assembly, component)| format!(
                        "- {} <> {}",
                        assembly.display(),
                        component.display()
                    ))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        let contract_overlaps = component
            .contract
            .owned_contracts
            .iter()
            .filter(|owned| assembly_contracts.contains(owned.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !contract_overlaps.is_empty() {
            return Err(format!(
                "component Slice `{}` claims contracts reserved for assembly owner `{}`: {}",
                component.contract.slice,
                boundary.assembly_slice,
                contract_overlaps.join(", ")
            ));
        }
    }

    for (index, left) in components.iter().enumerate() {
        for right in &components[index + 1..] {
            parallel::ensure_lease_compatible(&left.contract, &right.contract)?;
        }
    }
    Ok(())
}

fn read_component(path: &Path) -> Result<ComponentInput, String> {
    let bytes = bounded_file::read_regular(path, INPUT_LIMIT, "Wave component Slice contract")?;
    let contract = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "invalid Wave component Slice contract {}: {error}",
            path.display()
        )
    })?;
    Ok(ComponentInput {
        path: path.to_owned(),
        bytes,
        contract,
    })
}

fn integration_head(repository: &Path, base_ref: &str) -> Result<String, String> {
    git::output_in(
        repository,
        &["rev-parse", "--verify", &format!("{base_ref}^{{commit}}")],
        false,
    )
    .map(|value| value.trim().to_owned())
}

fn require_unchanged(path: &Path, expected: &[u8], label: &str) -> Result<(), String> {
    let current = bounded_file::read_regular(path, INPUT_LIMIT, label)?;
    if current == expected {
        Ok(())
    } else {
        Err(format!("{label} changed during assembly preflight"))
    }
}

fn validate_segment(
    repository: &Path,
    label: &str,
    value: &str,
    direct_is_reserved: bool,
) -> Result<(), String> {
    let reference = format!("refs/heads/wave-segment-check/{value}");
    if value.is_empty()
        || value != value.trim()
        || value.contains('/')
        || matches!(value, "." | "..")
        || (direct_is_reserved && value == "direct")
        || !git::succeeds_in(repository, &["check-ref-format", &reference], false)?
    {
        Err(format!("{label} must be one non-reserved branch segment"))
    } else {
        Ok(())
    }
}
