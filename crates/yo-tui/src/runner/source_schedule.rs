//! Deterministic cyclic selection across the live frontend's ordinary sources.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OrdinarySource {
    Terminal,
    Agent,
    Workspace,
    Skill,
}

impl OrdinarySource {
    const ALL: [Self; 4] = [Self::Terminal, Self::Agent, Self::Workspace, Self::Skill];

    const fn index(self) -> usize {
        match self {
            Self::Terminal => 0,
            Self::Agent => 1,
            Self::Workspace => 2,
            Self::Skill => 3,
        }
    }

    const fn successor(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Default)]
pub(super) struct SourceSchedule {
    cursor: Option<OrdinarySource>,
}

impl SourceSchedule {
    pub(super) fn order(&self) -> [OrdinarySource; 4] {
        let first = self.cursor.unwrap_or(OrdinarySource::Terminal);
        let mut order = OrdinarySource::ALL;
        order.rotate_left(first.index());
        order
    }

    pub(super) fn handled(&mut self, source: OrdinarySource) {
        self.cursor = Some(source.successor());
    }
}

#[cfg(test)]
mod tests {
    use super::{OrdinarySource, SourceSchedule};

    // 초기 selection과 각 handled source 뒤 selection은 항상 고정된 cyclic order를
    // 사용해 readiness timing과 무관하게 같은 tie-break를 재현한다.
    #[test]
    fn handled_observation_rotates_to_its_successor() {
        let mut schedule = SourceSchedule::default();
        assert_eq!(
            schedule.order(),
            [
                OrdinarySource::Terminal,
                OrdinarySource::Agent,
                OrdinarySource::Workspace,
                OrdinarySource::Skill,
            ]
        );

        schedule.handled(OrdinarySource::Workspace);
        assert_eq!(
            schedule.order(),
            [
                OrdinarySource::Skill,
                OrdinarySource::Terminal,
                OrdinarySource::Agent,
                OrdinarySource::Workspace,
            ]
        );
    }
}
