use std::collections::{BTreeMap, BTreeSet};

use crate::model::{KnowledgeUnit, Relations};

pub(super) fn find_cycles(
    units: &BTreeMap<String, KnowledgeUnit>,
    edges: impl Fn(&Relations) -> Vec<String>,
) -> Vec<Vec<String>> {
    let graph = units
        .iter()
        .map(|(id, unit)| {
            let mut targets = edges(&unit.metadata.relations);
            targets.retain(|target| units.contains_key(target));
            targets.sort();
            (id.clone(), targets)
        })
        .collect::<BTreeMap<_, _>>();
    let mut states = BTreeMap::<String, VisitState>::new();
    let mut stack = Vec::new();
    let mut cycles = BTreeSet::new();

    for id in graph.keys() {
        visit(id, &graph, &mut states, &mut stack, &mut cycles);
    }

    cycles.into_iter().collect()
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum VisitState {
    Visiting,
    Visited,
}

fn visit(
    id: &str,
    graph: &BTreeMap<String, Vec<String>>,
    states: &mut BTreeMap<String, VisitState>,
    stack: &mut Vec<String>,
    cycles: &mut BTreeSet<Vec<String>>,
) {
    match states.get(id) {
        Some(VisitState::Visited) => return,
        Some(VisitState::Visiting) => {
            if let Some(start) = stack.iter().position(|entry| entry == id) {
                let mut cycle = stack[start..].to_vec();
                cycle.push(id.to_owned());
                cycles.insert(canonical_cycle(cycle));
            }
            return;
        },
        None => {},
    }

    states.insert(id.to_owned(), VisitState::Visiting);
    stack.push(id.to_owned());
    if let Some(targets) = graph.get(id) {
        for target in targets {
            visit(target, graph, states, stack, cycles);
        }
    }
    stack.pop();
    states.insert(id.to_owned(), VisitState::Visited);
}

pub(super) fn canonical_cycle(mut cycle: Vec<String>) -> Vec<String> {
    cycle.pop();
    if cycle.is_empty() {
        return cycle;
    }
    let start = cycle
        .iter()
        .enumerate()
        .min_by_key(|(_, id)| *id)
        .map_or(0, |(index, _)| index);
    cycle.rotate_left(start);
    cycle.push(cycle[0].clone());
    cycle
}
