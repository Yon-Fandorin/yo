#[cfg(test)]
use std::cell;
use std::path::Path;

mod evaluate;
mod model;
mod output;
mod prepare;
mod revalidate;

pub(crate) use output::ReadyGate;
// 기존 호출자가 쓰는 타입 경로는 유지하되 내부 사용 강제는 하지 않는다.
#[allow(unused_imports)]
pub(crate) use output::ReadyValidation;
pub(crate) use prepare::run as prepare_request;

#[cfg(test)]
mod tests;

#[cfg(test)]
type FinalRevalidateTestHook = Box<dyn FnOnce() -> Result<(), String>>;

#[cfg(test)]
thread_local! {
    static FINAL_REVALIDATE_TEST_HOOK: cell::RefCell<Option<FinalRevalidateTestHook>> =
        const { cell::RefCell::new(None) };
}

#[cfg(test)]
fn set_final_revalidate_test_hook(hook: impl FnOnce() -> Result<(), String> + 'static) {
    FINAL_REVALIDATE_TEST_HOOK.with(|slot| {
        assert!(slot.replace(Some(Box::new(hook))).is_none());
    });
}

#[cfg(test)]
fn run_final_revalidate_hook() -> Result<(), String> {
    let hook = FINAL_REVALIDATE_TEST_HOOK.with(|slot| slot.borrow_mut().take());
    hook.map_or(Ok(()), |hook| hook())
}

#[cfg(not(test))]
fn run_final_revalidate_hook() -> Result<(), String> {
    Ok(())
}

pub(crate) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let output = evaluate(repository, request_path)?;
    println!(
        "{}",
        serde_json::to_string(&output)
            .map_err(|error| format!("cannot encode Slice gate result: {error}"))?
    );
    Ok(())
}

pub(crate) fn ready(repository: &Path, request_path: &Path) -> Result<ReadyGate, String> {
    output::ready(repository, request_path, &evaluate::evaluate)
}

// 루트 테스트가 계속 파사드 경계만 호출하도록 평가 진입점을 보존한다.
fn evaluate(repository: &Path, request_path: &Path) -> Result<model::ResultDocument, String> {
    evaluate::evaluate(repository, request_path)
}
