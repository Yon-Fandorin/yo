use super::{
    super::super::{
        ContextCheckpoint, ForkExactReplay, ForkGroup, ForkItemCoordinate, ForkItemOrigin,
        ForkSeed, JournalCodecError,
    },
    CorrelationRecovery,
};
use crate::JournalSequence;

impl CorrelationRecovery {
    pub(super) const fn open_epoch(&self) -> Option<u64> {
        self.open_epoch
    }

    pub(super) const fn latest_anchor(&self) -> Option<JournalSequence> {
        self.latest_anchor
    }

    pub(super) const fn latest_checkpoint(&self) -> Option<JournalSequence> {
        if self.request_after_checkpoint || self.latest_anchor.is_some() {
            None
        } else {
            self.latest_checkpoint
        }
    }

    pub(super) fn initial_fork_seed(&self) -> Option<JournalSequence> {
        self.fork_seed
            .as_ref()
            .filter(|fork| {
                !fork.has_accepted_request
                    && self.latest_checkpoint.is_none()
                    && fork.owner_epoch.is_some()
                    && fork.owner_epoch == self.open_epoch
                    && !matches!(fork.seed.seed(), ForkSeed::Empty)
            })
            .map(|fork| fork.sequence)
    }

    pub(super) fn fork_replay(&self) -> Result<ForkExactReplay, JournalCodecError> {
        if !self.active_turn_starts.is_empty() {
            return Err(JournalCodecError::new(
                "fork replay requires an idle Session",
            ));
        }
        if self.model_replay_origins.len() != self.model_replay.items().len()
            || !self
                .replay_groups
                .iter()
                .flat_map(|group| &group.items)
                .eq(self.model_replay.items())
        {
            return Err(JournalCodecError::new(
                "fork replay lacks complete correlated item provenance",
            ));
        }
        let contract = self
            .model_replay
            .contract()
            .cloned()
            .ok_or_else(|| JournalCodecError::new("fork replay has no exact contract"))?;
        let mut start = 0;
        let groups = self
            .replay_groups
            .iter()
            .map(|group| {
                let end = start + group.items.len();
                let result = ForkGroup::new(start, end);
                start = end;
                result
            })
            .collect::<Result<Vec<_>, _>>()?;
        ForkExactReplay::new(
            contract,
            self.model_replay.items().to_vec(),
            self.model_replay_origins.clone(),
            groups,
        )
    }

    pub(super) fn fork_boundary_is_idle(&self) -> bool {
        self.active_turn_starts.is_empty() && self.model_replay.contract().is_some()
    }

    pub(super) const fn open_binding_has_accepted_request(&self) -> bool {
        self.open_epoch_has_accepted_request
    }

    pub(super) fn local_item_origin(
        &self,
        epoch: u64,
        context_epoch: u64,
        sequence: JournalSequence,
        index: usize,
    ) -> Result<ForkItemOrigin, JournalCodecError> {
        let session = self
            .session_id
            .ok_or_else(|| JournalCodecError::new("replay origin has no Session"))?;
        let binding = self
            .open_binding
            .clone()
            .ok_or_else(|| JournalCodecError::new("replay origin has no exact binding"))?;
        let coordinate = ForkItemCoordinate::new(session, epoch, context_epoch, sequence, index)?;
        ForkItemOrigin::new(coordinate, coordinate, binding)
    }

    pub(super) fn checkpoint_origins(
        &self,
        sequence: JournalSequence,
        checkpoint: &ContextCheckpoint,
    ) -> Result<Vec<ForkItemOrigin>, JournalCodecError> {
        if self.model_replay_origins.len() != self.model_replay.items().len()
            || !self
                .replay_groups
                .iter()
                .flat_map(|group| &group.items)
                .eq(self.model_replay.items())
        {
            return Err(JournalCodecError::new(
                "checkpoint source lacks complete replay provenance",
            ));
        }
        let mut origins = vec![self.local_item_origin(
            checkpoint.epoch(),
            checkpoint.successor_context_epoch(),
            sequence,
            0,
        )?];
        let mut used = BTreeSet::new();
        for retained in checkpoint.retained_groups() {
            let mut offset = 0;
            let mut selected = None;
            for (index, group) in self.replay_groups.iter().enumerate() {
                let end = offset + group.items.len();
                if !used.contains(&index)
                    && group.first_sequence == retained.first_sequence()
                    && group.last_sequence == retained.last_sequence()
                    && group.fork_group_index == retained.fork_import().map(|(_, index)| index)
                    && group.items == retained.items()
                {
                    selected = Some((index, offset, end));
                    break;
                }
                offset = end;
            }
            if let Some((index, start, end)) = selected {
                used.insert(index);
                let session = self.session_id.ok_or_else(|| {
                    JournalCodecError::new("checkpoint provenance has no Session")
                })?;
                for origin in &self.model_replay_origins[start..end] {
                    let imported_from = ForkItemCoordinate::new(
                        session,
                        checkpoint.epoch(),
                        checkpoint.successor_context_epoch(),
                        sequence,
                        origins.len(),
                    )?;
                    origins.push(ForkItemOrigin::new(
                        *origin.original(),
                        imported_from,
                        origin.source_binding().clone(),
                    )?);
                }
            } else if retained.fork_import().is_none()
                && retained.first_sequence() > checkpoint.source_anchor_sequence()
                && retained.last_sequence() <= checkpoint.source_journal_boundary()
                && self.active_input_group_matches(retained)
            {
                let first = origins.len();
                for index in first..first + retained.items().len() {
                    origins.push(self.local_item_origin(
                        checkpoint.epoch(),
                        checkpoint.successor_context_epoch(),
                        sequence,
                        index,
                    )?);
                }
            } else {
                return Err(JournalCodecError::new(
                    "checkpoint retained provenance has no exact source group",
                ));
            }
        }
        Ok(origins)
    }
}
