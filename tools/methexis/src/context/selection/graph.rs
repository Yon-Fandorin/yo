//! Required-closure expansion and deterministic dependency-first ordering.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::KnowledgeUnit;

pub(super) fn unit_map(units: &[KnowledgeUnit]) -> BTreeMap<&str, &KnowledgeUnit> {
    units
        .iter()
        .map(|unit| (unit.metadata.id.as_str(), unit))
        .collect()
}

pub(super) fn closure(
    root: &str,
    units: &BTreeMap<&str, &KnowledgeUnit>,
) -> Result<BTreeSet<String>, String> {
    let mut selected = BTreeSet::new();
    let mut pending = vec![root.to_owned()];
    while let Some(id) = pending.pop() {
        if !selected.insert(id.clone()) {
            continue;
        }
        let unit = units.get(id.as_str()).ok_or_else(|| id.clone())?;
        let mut required = unit
            .metadata
            .relations
            .required_targets()
            .cloned()
            .collect::<Vec<_>>();
        required.sort_by(|left, right| right.cmp(left));
        pending.extend(required);
    }
    Ok(selected)
}

pub(super) fn ordered_units<'a>(
    included: &BTreeSet<String>,
    units: &'a [KnowledgeUnit],
) -> Vec<&'a KnowledgeUnit> {
    let map = unit_map(units);
    let mut consumers = BTreeMap::<&str, Vec<&str>>::new();
    let mut indegree = BTreeMap::<&str, usize>::new();
    for id in included {
        indegree.insert(id, 0);
    }
    for id in included {
        let unit = map
            .get(id.as_str())
            .expect("selected KnowledgeId exists in trusted foundation");
        let required = unit
            .metadata
            .relations
            .required_targets()
            .filter(|target| included.contains(*target))
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        *indegree.get_mut(id.as_str()).expect("selected unit") = required.len();
        for target in required {
            consumers.entry(target).or_default().push(id);
        }
    }
    let mut ready = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
        .collect::<BTreeSet<_>>();
    let mut ordered = Vec::with_capacity(included.len());
    while let Some(id) = ready.pop_first() {
        ordered.push(*map.get(id).expect("ready KnowledgeId exists"));
        for consumer in consumers.get(id).into_iter().flatten() {
            let degree = indegree.get_mut(consumer).expect("consumer is selected");
            *degree -= 1;
            if *degree == 0 {
                ready.insert(consumer);
            }
        }
    }
    debug_assert_eq!(ordered.len(), included.len());
    ordered
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::model::{KnowledgeKind, KnowledgeMetadata, Relations};

    // 의존성이 모두 해결돼 다음에 배치할 수 있는 지식이 여러 개면 id 오름차순으로 하나를 고른다.
    // 이 규칙으로 입력 순서와 무관하게 항상 같은 의존성 우선 순서를 만든다.
    #[test]
    fn topological_order_uses_a_global_ascending_ready_tie_break() {
        let units = vec![
            unit("a.consumer", &["z.dependency"]),
            unit("b.independent", &[]),
            unit("z.dependency", &[]),
        ];
        let included = units.iter().map(|unit| unit.metadata.id.clone()).collect();

        let ordered = ordered_units(&included, &units)
            .into_iter()
            .map(|unit| unit.metadata.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ordered, ["b.independent", "z.dependency", "a.consumer"]);
    }

    fn unit(id: &str, depends_on: &[&str]) -> KnowledgeUnit {
        KnowledgeUnit {
            metadata: KnowledgeMetadata {
                schema: "methexis.knowledge/v1alpha1".to_owned(),
                id: id.to_owned(),
                kind: KnowledgeKind::Rule,
                owner: "test".to_owned(),
                sources: Vec::new(),
                relations: Relations {
                    depends_on: depends_on.iter().map(|id| (*id).to_owned()).collect(),
                    ..Relations::default()
                },
            },
            body: String::new(),
            path: PathBuf::new(),
            revision: format!("revision:{id}"),
        }
    }
}
