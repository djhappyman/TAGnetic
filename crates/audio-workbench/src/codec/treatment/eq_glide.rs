use super::{Biquad, ChannelEq, SpectralEq, TreatmentProcessor};

pub(super) struct EqGlide {
    stereo: Vec<Vec<Biquad>>,
    source: [Vec<Biquad>; 2],
    matrix: [Vec<Biquad>; 2],
    eq_input_gain: f64,
    level_gain: f64,
    frames_left: usize,
}

impl TreatmentProcessor {
    /// Retune a compatible EQ with its filter delays and FFT history intact.
    pub fn glide_eq(&mut self, target: &mut Self, frames: usize) -> bool {
        let matching_stages = |current: &[ChannelEq; 2], target: &[ChannelEq; 2]| {
            current.iter().zip(target).all(|(current, target)| {
                current.filters.len() == target.filters.len()
                    && current.spectral.is_some() == target.spectral.is_some()
            })
        };
        let unchanged_dynamics = self
            .prepared
            .settings
            .ordinary_eq_dynamics_match(target.prepared.settings);
        if frames == 0
            || !self
                .prepared
                .can_retune_eq(target.prepared, self.output.sample_rate)
            || self.prepared.gain != target.prepared.gain
            || self.prepared.monitor_gain != target.prepared.monitor_gain
            || self.prepared.preview_ceiling != target.prepared.preview_ceiling
            || !unchanged_dynamics
            || self.spectral_eq.is_some() != target.spectral_eq.is_some()
            || self.filters.len() != target.filters.len()
            || self
                .filters
                .iter()
                .zip(&target.filters)
                .any(|(a, b)| a.len() != b.len())
            || !matching_stages(&self.source_eq, &target.source_eq)
            || !matching_stages(&self.matrix_eq, &target.matrix_eq)
        {
            return false;
        }
        retune_spectral(&mut self.spectral_eq, &mut target.spectral_eq);
        for (current, target) in self
            .source_eq
            .iter_mut()
            .zip(&mut target.source_eq)
            .chain(self.matrix_eq.iter_mut().zip(&mut target.matrix_eq))
        {
            retune_spectral(&mut current.spectral, &mut target.spectral);
        }
        self.eq_glide = Some(EqGlide {
            stereo: std::mem::take(&mut target.filters),
            source: target
                .source_eq
                .each_mut()
                .map(|eq| std::mem::take(&mut eq.filters)),
            matrix: target
                .matrix_eq
                .each_mut()
                .map(|eq| std::mem::take(&mut eq.filters)),
            eq_input_gain: target.eq_input_gain,
            level_gain: target.level_gain,
            frames_left: frames,
        });
        self.prepared = target.prepared;
        true
    }

    pub(super) fn advance_eq_glide(&mut self) {
        let Some(glide) = &mut self.eq_glide else {
            return;
        };
        let weight = (glide.frames_left as f64).recip();
        let advance = |filters: &mut [Biquad], targets: &[Biquad]| {
            for (filter, target) in filters.iter_mut().zip(targets) {
                filter.glide_toward(target, weight);
            }
        };
        for (filters, targets) in self.filters.iter_mut().zip(&glide.stereo) {
            advance(filters, targets);
        }
        for (eq, targets) in self.source_eq.iter_mut().zip(&glide.source) {
            advance(&mut eq.filters, targets);
        }
        for (eq, targets) in self.matrix_eq.iter_mut().zip(&glide.matrix) {
            advance(&mut eq.filters, targets);
        }
        self.eq_input_gain += (glide.eq_input_gain - self.eq_input_gain) * weight;
        self.level_gain += (glide.level_gain - self.level_gain) * weight;
        glide.frames_left -= 1;
        if glide.frames_left == 0 {
            self.eq_input_gain = glide.eq_input_gain;
            self.level_gain = glide.level_gain;
            self.eq_glide = None;
        }
    }
}

fn retune_spectral(current: &mut Option<SpectralEq>, target: &mut Option<SpectralEq>) {
    if let (Some(current), Some(target)) = (current, target) {
        current.retune_from(target);
    }
}
