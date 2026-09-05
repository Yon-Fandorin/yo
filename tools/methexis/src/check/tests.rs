use super::{
    DiagnosticPhase, cycles::canonical_cycle, knowledge_revision, local_diagnostic, parse_yaml,
    sort_diagnostics, validate_metadata,
};

// YAML 키 순서와 줄바꿈(CRLF·LF)이 달라도 같은 내용의 revision은 동일하다.
#[test]
fn semantic_revision_ignores_yaml_order_and_line_endings() {
    let first = "\
---\r\nschema: methexis.knowledge/v1alpha1\r\nid: tui.example\r\nkind: rule\r\nowner: tui-architecture\r\nsources:\r\n  - id: tui.arc-001\r\n    revision: sha256:0000000000000000000000000000000000000000000000000000000000000000\r\nrelations:\r\n  depends_on: []\r\n---\r\n## Statement\r\n\r\nAn example rule.\r\n";
    let second = "\
---\nowner: tui-architecture\nkind: rule\nid: tui.example\nschema: methexis.knowledge/v1alpha1\nrelations:\n  depends_on: []\nsources:\n  - revision: sha256:0000000000000000000000000000000000000000000000000000000000000000\n    id: tui.arc-001\n---\n## Statement\n\nAn example rule.\n";

    let first = parse_for_test(first);
    let second = parse_for_test(second);

    assert_eq!(first, second);
}

// 메타데이터가 같아도 본문이 다르면 revision이 달라진다.
#[test]
fn body_change_changes_revision() {
    let metadata = crate::model::KnowledgeMetadata {
        schema: "methexis.knowledge/v1alpha1".to_owned(),
        id: "tui.example".to_owned(),
        kind: crate::model::KnowledgeKind::Rule,
        owner: "tui-architecture".to_owned(),
        sources: vec![source_ref("tui.arc-001")],
        relations: crate::model::Relations::default(),
    };

    assert_ne!(
        knowledge_revision(&metadata, "## Statement\n\nFirst.\n"),
        knowledge_revision(&metadata, "## Statement\n\nSecond.\n"),
    );
}

// 고정된 입력의 revision이 golden digest와 정확히 일치하는지 검증한다.
#[test]
fn semantic_revision_has_a_golden_digest() {
    assert_eq!(
        knowledge_revision(&metadata_for_test(), "## Statement\n\nStable.\n"),
        "sha256:925c20b6fba7467a7d637d7a5ac59cbd183410eb4cc0ade5c20156158f655317",
    );
}

// 단독 CR과 CRLF 줄바꿈이 모두 LF로 정규화되는지 확인한다.
#[test]
fn bare_carriage_returns_normalize_to_lf() {
    assert_eq!(
        super::normalize_line_endings("first\rsecond\r\nthird\n".to_owned()),
        "first\nsecond\nthird\n",
    );
}

// Source와 관계 목록은 작성 순서가 아니라 의미로 revision을 계산해야 한다.
// 같은 항목을 다른 순서로 적어도 정렬 후 동일한 semantic revision이 나오는지 확인한다.
#[test]
fn semantic_revision_sorts_sources_and_typed_relations() {
    let mut first = metadata_for_test();
    first.sources = vec![source_ref("tui.source-b"), source_ref("tui.source-a")];
    first.relations.depends_on = vec!["tui.unit-b".to_owned(), "tui.unit-a".to_owned()];
    let mut second = metadata_for_test();
    second.sources = vec![source_ref("tui.source-a"), source_ref("tui.source-b")];
    second.relations.depends_on = vec!["tui.unit-a".to_owned(), "tui.unit-b".to_owned()];

    assert_eq!(
        knowledge_revision(&first, "## Statement\n\nStable.\n"),
        knowledge_revision(&second, "## Statement\n\nStable.\n"),
    );
}

// 메시지 알파벳 순서와 충돌하더라도 파일에서 더 앞선 line의 진단을 먼저 정렬한다.
#[test]
fn diagnostic_order_uses_location_before_message() {
    let mut diagnostics = vec![
        local_diagnostic(
            "unit.md".to_owned(),
            "same_code",
            "alphabetically first".to_owned(),
            Some(2),
            Some(1),
            Vec::new(),
        ),
        local_diagnostic(
            "unit.md".to_owned(),
            "same_code",
            "alphabetically last".to_owned(),
            Some(1),
            Some(1),
            Vec::new(),
        ),
    ];

    sort_diagnostics(&mut diagnostics);

    assert_eq!(diagnostics[0].phase, DiagnosticPhase::Local);
    assert_eq!(diagnostics[0].line, Some(1));
}

// 권한 정보를 읽는 도중 일시적인 변경이 감지돼 다시 시도해야 하더라도 진단 정보는 잃지 않는다.
// 마지막으로 확인한 trusted commit과 호출자가 취할 다음 action을 오류에 함께 보존한다.
#[test]
fn retryable_authority_failure_preserves_trusted_commit_and_action() {
    let report = super::failed_authority_report(crate::checkpoint::AuthorityFailure {
        diagnostics: vec![local_diagnostic(
            "methexis/sources".to_owned(),
            "source_changed_during_validation",
            "Source changed".to_owned(),
            None,
            None,
            vec!["tui.example".to_owned()],
        )],
        trusted_commit: Some("0123456789abcdef".to_owned()),
        retryable: true,
    });

    assert!(report.retryable);
    assert_eq!(report.trusted_commit.as_deref(), Some("0123456789abcdef"));
    assert_eq!(
        report.next_actions,
        ["retry `methexis check`; no state was published"]
    );
}

// YAML 1.1 파서는 대문자 `NO`도 false로 오해할 수 있다.
// serde_norway 경계에서는 문자열 필드에 쓴 `NO`를 글자 그대로 보존하는지 확인한다.
#[test]
fn norway_keeps_yaml_no_as_a_string_for_string_fields() {
    let owner: crate::model::OwnerRecord =
        parse_yaml("schema: methexis.owner/v1alpha1\nid: NO\n", "owner.yaml", 0)
            .expect("NO remains a string at the typed boundary");

    assert_eq!(owner.id, "NO");
}

// 같은 YAML key를 두 번 쓰면 어느 값이 진짜인지 조용히 선택해서는 안 된다.
// serde_norway가 중복 key를 발견하는 즉시 역직렬화를 거부하는지 확인한다.
#[test]
fn norway_rejects_duplicate_mapping_keys_at_the_typed_boundary() {
    let result = parse_yaml::<crate::model::OwnerRecord>(
        "schema: methexis.owner/v1alpha1\nid: first\nid: second\n",
        "owner.yaml",
        0,
    );

    assert!(result.is_err());
}

// `<<` merge key는 보이지 않는 필드 상속을 만들어 닫힌 schema 검증을 흐릴 수 있다.
// 입력 의미가 명시적으로 보이도록 serde_norway 경계에서 merge key를 거부한다.
#[test]
fn norway_rejects_yaml_merge_keys_at_the_typed_boundary() {
    let result = parse_yaml::<crate::model::OwnerRecord>(
        "schema: methexis.owner/v1alpha1\nid: direct\n<<: { id: inherited }\n",
        "owner.yaml",
        0,
    );

    assert!(result.is_err());
}

// fenced code 안의 heading은 필수 본문 section을 충족한 것으로 세지 않는다.
#[test]
fn headings_inside_fenced_code_do_not_satisfy_body_sections() {
    let metadata = metadata_for_test();
    let diagnostics = validate_metadata(
        &metadata,
        "# Example\n\n```markdown\n## Statement\n\nNot a real section.\n```\n",
        1,
        "unit.md",
    );

    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "missing_body_section")
    );
}

// HTML comment 자체는 raw_html_forbidden으로 거부하고, 그 안의 heading도 필수 section으로
// 인정하지 않아 missing_body_section을 함께 보고하는지 확인한다.
#[test]
fn headings_inside_html_comments_make_the_body_invalid() {
    let diagnostics = validate_metadata(
        &metadata_for_test(),
        "# Example\n\nprefix <!--\n## Statement\n\nHidden.\n-->\n",
        10,
        "unit.md",
    );

    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "raw_html_forbidden")
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "missing_body_section")
    );
}

// fenced code 안에 raw HTML 철자가 있어도 실제 HTML 노드가 아니므로 허용한다.
#[test]
fn raw_html_spelling_inside_fenced_code_is_allowed() {
    let diagnostics = validate_metadata(
        &metadata_for_test(),
        "## Statement\n\n```html\n<div>Rendered as code</div>\n```\n",
        1,
        "unit.md",
    );

    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "raw_html_forbidden")
    );
}

// 보이는 HTML은 fenced code가 아니므로 raw_html_forbidden으로 거부하고, 진단 위치도
// 본문의 해당 line을 가리켜 실제로 찾아 고칠 수 있는 현재 동작을 고정한다.
#[test]
fn visible_html_outside_fenced_code_is_forbidden() {
    let diagnostics = validate_metadata(
        &metadata_for_test(),
        "## Statement\n\nVisible text.\n<div>Rendered HTML</div>\n",
        4,
        "unit.md",
    );

    let html = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "raw_html_forbidden")
        .expect("visible HTML diagnostic");
    assert_eq!(html.line, Some(7));
    assert_eq!(html.column, Some(1));
}

// tilde fence 안에만 있는 Statement heading은 필수 section을 충족하지 않지만, 같은
// fence 안의 raw HTML 예시는 허용되는 현재 경계를 각각의 진단으로 고정한다.
#[test]
fn tilde_fenced_content_does_not_satisfy_the_required_statement() {
    let diagnostics = validate_metadata(
        &metadata_for_test(),
        "~~~markdown\n## Statement\n<div>Code example</div>\n~~~\n",
        1,
        "unit.md",
    );

    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "missing_body_section")
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "raw_html_forbidden")
    );
}

// cycle 입력의 시작점이 달라도 KnowledgeId가 가장 작은 항목부터 닫힌 경로로
// 표현되어야 하므로, 전역 진단이 사용할 canonical cycle 모양을 직접 고정한다.
#[test]
fn canonical_cycle_starts_at_the_smallest_id_and_closes_the_path() {
    assert_eq!(
        canonical_cycle(vec![
            "tui.cycle-z".to_owned(),
            "tui.cycle-a".to_owned(),
            "tui.cycle-m".to_owned(),
            "tui.cycle-z".to_owned(),
        ]),
        [
            "tui.cycle-a".to_owned(),
            "tui.cycle-m".to_owned(),
            "tui.cycle-z".to_owned(),
            "tui.cycle-a".to_owned(),
        ]
    );
}

// 본문만 따로 검사하더라도 오류 line은 잘라 낸 본문 기준이 되어서는 안 된다.
// 사용자가 바로 찾아갈 수 있도록 frontmatter를 포함한 원본 파일의 line을 보고한다.
#[test]
fn body_diagnostic_lines_are_file_relative() {
    let diagnostics = validate_metadata(&metadata_for_test(), "## Statement\n", 12, "unit.md");
    let empty = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "empty_body_section")
        .expect("empty Statement diagnostic");

    assert_eq!(empty.line, Some(12));
}

// frontmatter 끝의 빈 줄도 세어 본문 시작 line을 정확히 계산한다.
#[test]
fn body_start_line_counts_trailing_blank_frontmatter_lines() {
    let content = "---\nschema: example\n\n---\n## Statement\n";
    let (_, body) = super::split_frontmatter(content).expect("frontmatter");

    assert_eq!(super::body_start_line(content, body), 5);
}

fn metadata_for_test() -> crate::model::KnowledgeMetadata {
    crate::model::KnowledgeMetadata {
        schema: "methexis.knowledge/v1alpha1".to_owned(),
        id: "tui.example".to_owned(),
        kind: crate::model::KnowledgeKind::Rule,
        owner: "tui-architecture".to_owned(),
        sources: vec![source_ref("tui.fixture")],
        relations: crate::model::Relations::default(),
    }
}

fn source_ref(id: &str) -> crate::model::SourceRef {
    crate::model::SourceRef {
        id: id.to_owned(),
        revision: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
            .to_owned(),
    }
}

fn parse_for_test(content: &str) -> String {
    let normalized = content.replace("\r\n", "\n");
    let (frontmatter, body) = super::split_frontmatter(&normalized).expect("frontmatter");
    let metadata = parse_yaml(frontmatter, "test.md", 1).expect("valid test frontmatter");
    knowledge_revision(&metadata, body)
}
