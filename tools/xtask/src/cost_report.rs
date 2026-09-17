use std::path::Path;

mod capture;
mod input;
mod measurements;
mod model;
mod output;

#[cfg(test)]
mod tests;

pub(crate) fn run(repository: &Path, request_path: &Path, output: &Path) -> Result<(), String> {
    let loaded = input::load(repository, request_path, output)?;
    let report_bytes = output::render(
        &loaded.request,
        &loaded.request_path,
        &loaded.request_bytes,
        loaded.sources,
    )?;

    capture::revalidate_sources(&loaded.request, &loaded.workspace)?;
    input::require_request_unchanged(&loaded.request_path, &loaded.request_bytes)?;
    let created = output::publish(&loaded.output, &report_bytes)?;
    println!(
        "{}",
        output::publication(&loaded.output, &report_bytes, created)?
    );
    Ok(())
}
