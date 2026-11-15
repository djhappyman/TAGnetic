//! Duration evidence for release alignment. This runs natively over cached
//! metadata; a proposal never changes order or writes files on its own.
use std::collections::{BTreeMap, VecDeque};

const NEAR: i64 = 10;
const AMBIGUOUS: u64 = 2;

fn pair(files: &[Option<u64>], tracks: &[Option<u64>], offset: i64) -> Vec<Option<usize>> {
    let mut available: BTreeMap<i64, VecDeque<usize>> = BTreeMap::new();
    for (i, secs) in files.iter().enumerate() {
        if let Some(secs) = secs.and_then(|n| i64::try_from(n).ok()).filter(|n| *n > 0) {
            if let Some(shifted) = secs.checked_sub(offset) {
                available.entry(shifted).or_default().push_back(i);
            }
        }
    }
    tracks
        .iter()
        .map(|secs| {
            let target = i64::try_from(secs.filter(|n| *n > 0)?).ok()?;
            let below = available.range(..=target).next_back().map(|(&key, _)| key);
            let above = available.range(target..).next().map(|(&key, _)| key);
            let key = [below, above]
                .into_iter()
                .flatten()
                .min_by_key(|key| key.abs_diff(target))?;
            if key.abs_diff(target) > 120 {
                return None;
            }
            let queue = available.get_mut(&key)?;
            let index = queue.pop_front();
            if queue.is_empty() {
                available.remove(&key);
            }
            index
        })
        .collect()
}

/// A small offset shared by at least two pairs indicates a different master or
/// consistently trimmed silence. Disagreement or a large shift remains zero.
pub fn common_offset(files: &[Option<u64>], tracks: &[Option<u64>]) -> i64 {
    let mut deltas: Vec<i64> = pair(files, tracks, 0)
        .iter()
        .enumerate()
        .filter_map(|(k, j)| {
            Some(i64::try_from(files[(*j)?]?).ok()? - i64::try_from(tracks[k]?).ok()?)
        })
        .collect();
    if deltas.len() < 2 {
        return 0;
    }
    deltas.sort_unstable();
    if deltas.last().copied().unwrap_or(0) - deltas[0] > 4 {
        return 0;
    }
    let middle = deltas.len() / 2;
    let median = if deltas.len().is_multiple_of(2) {
        (deltas[middle - 1] + deltas[middle]) / 2
    } else {
        deltas[middle]
    };
    if (3..=30).contains(&median.abs()) {
        median
    } else {
        0
    }
}

/// Return a confirmed permutation only when current order contradicts unique
/// length evidence. Unknown durations and equal-length rivals are not guesses.
pub fn order_proposal(
    files: &[Option<u64>],
    tracks: &[Option<u64>],
    release: &[Option<u64>],
) -> Option<(Vec<usize>, usize, i64)> {
    if tracks.is_empty()
        || files
            .iter()
            .chain(tracks)
            .any(|n| n.is_none_or(|v| v == 0 || v > i64::MAX as u64))
    {
        return None;
    }
    let offset = common_offset(files, tracks);
    let pairs = pair(files, tracks, offset);
    let near = |j: usize, k: usize| {
        (files[j].unwrap_or(0) as i64)
            .checked_sub(offset)
            .and_then(|shifted| shifted.checked_sub(tracks[k].unwrap_or(0) as i64))
            .is_some_and(|delta| delta.unsigned_abs() <= NEAR as u64)
    };
    let mut assigned = BTreeMap::new();
    let mut contradicted = 0;
    for (k, j) in pairs.iter().enumerate() {
        let Some(j) = *j else {
            continue;
        };
        if k >= files.len() || !near(j, k) {
            continue;
        }
        let secs = tracks[k]?;
        if release
            .iter()
            .filter(|other| other.is_some_and(|v| v > 0 && v.abs_diff(secs) <= AMBIGUOUS))
            .count()
            > 1
        {
            continue;
        }
        assigned.insert(k, j);
        if j != k && !near(k, k) {
            contradicted += 1;
        }
    }
    if contradicted == 0 {
        return None;
    }
    let used: std::collections::HashSet<usize> = assigned.values().copied().collect();
    let mut rest = (0..files.len()).filter(|j| !used.contains(j));
    let order = (0..files.len())
        .map(|k| assigned.get(&k).copied().or_else(|| rest.next()))
        .collect::<Option<Vec<_>>>()?;
    Some((order, contradicted, offset))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shuffled_master_offset_proposes_unique_order() {
        let tracks = [Some(100), Some(200), Some(300)];
        let files = [Some(309), Some(109), Some(209)];
        assert_eq!(
            order_proposal(&files, &tracks, &tracks),
            Some((vec![1, 2, 0], 3, 9))
        );
        assert!(order_proposal(&[Some(109), Some(209), Some(309)], &tracks, &tracks).is_none());
    }
    #[test]
    fn unknown_and_ambiguous_lengths_do_not_reorder() {
        assert!(order_proposal(
            &[Some(200), None],
            &[Some(100), Some(200)],
            &[Some(100), Some(200)]
        )
        .is_none());
        assert!(order_proposal(
            &[Some(200), Some(100)],
            &[Some(100), Some(200)],
            &[Some(100), Some(101), Some(200), Some(201)]
        )
        .is_none());
    }
    #[test]
    fn an_extra_file_keeps_its_turn_after_the_paired_tracks() {
        assert_eq!(
            order_proposal(
                &[Some(300), Some(100), Some(200)],
                &[Some(100), Some(200)],
                &[Some(100), Some(200)]
            )
            .map(|p| p.0),
            Some(vec![1, 2, 0])
        );
    }
    #[test]
    fn extreme_metadata_durations_never_overflow() {
        let files = [Some(91), Some(191), Some(i64::MAX as u64)];
        let tracks = [Some(100), Some(200), Some(i64::MAX as u64)];
        let _ = order_proposal(&files, &tracks, &tracks);
        assert!(order_proposal(&[Some(u64::MAX)], &[Some(100)], &[Some(100)]).is_none());
    }

    #[test]
    fn inconsistent_or_large_offsets_are_not_corrected() {
        assert_eq!(
            common_offset(&[Some(130), Some(250)], &[Some(100), Some(200)]),
            0
        );
        assert_eq!(
            common_offset(&[Some(140), Some(240)], &[Some(100), Some(200)]),
            0
        );
    }
}
