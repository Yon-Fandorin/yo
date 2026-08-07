//! Agent-facing contract fixtures for the prepare commands.

use std::{fs, path::Path};

use serde_json::Value;

use super::support::*;

// prepare-approval 예제 manifest로 실제 CLI를 실행하고 wall-clock인 reviewed_at만
// placeholder로 정규화한 뒤 golden과 비교한다. 예제 manifest 자체도 build-review 출력과
// 일치하는지 함께 고정해 예제가 실제 파이프라인에서 벗어나지 않게 한다.
#[test]
fn prepare_approval_contract_fixtures_are_complete_and_current() {
    let repository = GitRepository::foundation();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/prepare-approval-contract");

    let revision = repository.revision_for(KNOWLEDGE_ID);
    let projection = repository.project(KNOWLEDGE_ID, &revision);
    let manifest_path = repository.build_manifest(
        KNOWLEDGE_ID,
        &revision,
        projection["hash"].as_str().unwrap(),
    );
    let produced: Value =
        serde_json::from_slice(&fs::read(repository.path.join(manifest_path)).unwrap()).unwrap();
    let checked_in: Value =
        serde_json::from_slice(&fs::read(examples.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(produced, checked_in);

    let actual = success_json(repository.run(&[
        "prepare-approval",
        examples.join("manifest.json").to_str().unwrap(),
        "--reviewer",
        OWNER_ID,
    ]));
    let actual = normalize_reviewed_at(actual);
    let expected: Value =
        serde_json::from_slice(&fs::read(examples.join("approval-request.json")).unwrap()).unwrap();
    assert_eq!(actual, expected);

    let failure = failure_json(repository.run(&[
        "prepare-approval",
        examples.join("manifest.json").to_str().unwrap(),
        "--reviewer",
        "nobody",
    ]));
    let expected: Value =
        serde_json::from_slice(&fs::read(examples.join("unknown-reviewer.json")).unwrap()).unwrap();
    assert_eq!(failure, expected);
}

// 활성 Checkpoint를 통합한 저장소에서 prepare-checkpoint 출력을 golden과 비교하고,
// 활성 Checkpoint가 없는 저장소에서는 no_active_checkpoint 실패 golden과 비교한다.
#[test]
fn prepare_checkpoint_contract_fixtures_are_complete_and_current() {
    let examples =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/prepare-checkpoint-contract");

    let repository = GitRepository::approved();
    repository.integrate_active_checkpoint();
    let actual = success_json(repository.run(&["prepare-checkpoint"]));
    let expected: Value =
        serde_json::from_slice(&fs::read(examples.join("checkpoint-request.json")).unwrap())
            .unwrap();
    assert_eq!(actual, expected);

    let pristine = GitRepository::approved();
    let failure = failure_json(pristine.run(&["prepare-checkpoint"]));
    let expected: Value =
        serde_json::from_slice(&fs::read(examples.join("no-active-checkpoint.json")).unwrap())
            .unwrap();
    assert_eq!(failure, expected);
}

// 예제 create 출력이 현재 create-checkpoint 결과와 같은지 확인한 뒤, 그 입력으로
// prepare-activation을 실행해 golden activation 요청과 비교하고, 방출된 요청이
// 실제 propose-activation에 그대로 수용되는지까지 증명한다.
#[test]
fn prepare_activation_contract_fixtures_are_complete_and_current() {
    let repository = GitRepository::approved();
    let examples =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/prepare-activation-contract");
    let checkpoint_examples =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/checkpoint-contract");

    let created = success_json(
        repository.run(&[
            "create-checkpoint",
            checkpoint_examples
                .join("checkpoint-request.json")
                .to_str()
                .unwrap(),
        ]),
    );
    let checked_in: Value =
        serde_json::from_slice(&fs::read(examples.join("create-output.json")).unwrap()).unwrap();
    assert_eq!(created, checked_in);

    let actual = success_json(repository.run(&[
        "prepare-activation",
        examples.join("create-output.json").to_str().unwrap(),
    ]));
    assert!(actual.get("replace_active_hash").is_none());
    let expected: Value =
        serde_json::from_slice(&fs::read(examples.join("activation-request.json")).unwrap())
            .unwrap();
    assert_eq!(actual, expected);

    let request = repository.request("activation.json", &actual);
    let activation =
        success_json(repository.run(&["propose-activation", request.to_str().unwrap()]));
    assert_eq!(activation["status"], "written");
}

// reviewed_at이 승인 기록의 UTC 형식(`YYYY-MM-DDTHH:MM:SSZ`)인지 검증한 뒤
// wall-clock 값을 golden의 placeholder로 치환한다.
fn normalize_reviewed_at(mut value: Value) -> Value {
    let reviewed_at = value["reviewed_at"].as_str().expect("reviewed_at string");
    let bytes = reviewed_at.as_bytes();
    assert_eq!(bytes.len(), 20, "reviewed_at length");
    assert!(reviewed_at.ends_with('Z'), "reviewed_at is UTC");
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 4 | 7 | 10 | 13 | 16 | 19) {
            continue;
        }
        assert!(byte.is_ascii_digit(), "reviewed_at digits at {index}");
    }
    value["reviewed_at"] = Value::String("<wall-clock>".to_owned());
    value
}
