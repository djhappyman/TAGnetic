//! Immutable saved DTOs, with scoped views that never copy metadata strings.
use crate::{ServiceError, ServiceResult};
use std::{ops::Index, sync::Arc};
use tagrex_commands::TrackDto;
use tagrex_core::resources::{Budget, Charge, Lease};

pub(crate) struct SnapshotRows {
    source: Source,
}
enum Source {
    All(Vec<TrackDto>),
    Selected {
        base: Arc<SnapshotRows>,
        positions: Vec<usize>,
        // Declared after the owned vector: release its memory before its charge.
        _lease: Lease,
    },
}
impl SnapshotRows {
    pub(crate) fn all(rows: Vec<TrackDto>) -> Self {
        Self {
            source: Source::All(rows),
        }
    }
    pub(crate) fn selected(
        base: Arc<Self>,
        mut include: impl FnMut(&TrackDto) -> bool,
    ) -> ServiceResult<Self> {
        // Reserve the complete possible vector before evaluating paths or
        // allocating its capacity. The base is already immutable and shared.
        let bytes = base
            .len()
            .checked_mul(std::mem::size_of::<usize>())
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Self>()))
            .ok_or_else(|| ServiceError::invalid("Snapshot selection is too large"))?;
        let lease = Budget::process()
            .try_acquire(Charge {
                retained: bytes,
                ..Default::default()
            })
            .map_err(tagrex_commands::AppError::Resource)?;
        let mut positions = Vec::with_capacity(base.len());
        for (index, row) in base.iter().enumerate() {
            if include(row) {
                positions.push(index);
            }
        }
        Ok(Self {
            source: Source::Selected {
                base,
                positions,
                _lease: lease,
            },
        })
    }
    pub(crate) fn positions(&self) -> Option<&[usize]> {
        match &self.source {
            Source::All(_) => None,
            Source::Selected { positions, .. } => Some(positions),
        }
    }
    pub(crate) fn len(&self) -> usize {
        match &self.source {
            Source::All(rows) => rows.len(),
            Source::Selected { positions, .. } => positions.len(),
        }
    }
    pub(crate) fn get(&self, index: usize) -> Option<&TrackDto> {
        match &self.source {
            Source::All(rows) => rows.get(index),
            Source::Selected {
                base, positions, ..
            } => base.get(*positions.get(index)?),
        }
    }
    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = &TrackDto> + DoubleEndedIterator {
        (0..self.len()).map(|index| &self[index])
    }
}
impl Index<usize> for SnapshotRows {
    type Output = TrackDto;
    fn index(&self, index: usize) -> &Self::Output {
        self.get(index)
            .expect("snapshot row index within saved view")
    }
}
