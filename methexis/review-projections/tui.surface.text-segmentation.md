---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.surface.text-segmentation
revision: sha256:a4911404c56747266cada0136602123dc0c06115f330f8bd6e7fbd355bdd46f3
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:ad53c8be3fafc0dfe027f94019d2f6b0612af7dab22ee494111d50abd14c6eab
---
# Korean Review Projection

## Translation

text layout은 Unicode 17.0의 수정되지 않은 extended grapheme cluster boundary algorithm, 즉 UAX #29 conformance clause C1-1로 입력을 분할해야 합니다. locale 또는 CLDR tailoring은 적용하면 안 됩니다.

분할된 cluster는 원래 UTF-8 문자열을 유지해야 합니다. segmentation이 저장 text를 normalize하거나 다시 쓰면 안 되며, canonical equivalent 문자열은 경계가 같아도 cell content의 byte는 서로 다를 수 있습니다.

boundary algorithm과 data version을 고정하면 특정 Rust dependency를 semantic authority로 만들지 않고 emoji ZWJ, regional indicator, combining mark, script-specific cluster를 결정론적으로 처리할 수 있습니다.
