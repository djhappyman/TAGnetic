use serde::{Deserialize, Serialize};

use super::{deserialize_slots, Biquad, MediaError};

pub const MAX_EQ_BANDS: usize = 8;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EqBandKind {
    Bell,
    Notch,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EqBand {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    pub kind: EqBandKind,
    pub hz: u16,
    pub gain_db: i8,
    pub q: u16,
    #[serde(default)]
    pub bypassed: bool,
}

impl EqBand {
    pub(super) fn active(self) -> bool {
        !self.bypassed && (self.kind == EqBandKind::Notch || self.gain_db != 0)
    }

    pub(super) fn validate(self, sample_rate: u32) -> Result<(), MediaError> {
        let notch = self.kind == EqBandKind::Notch;
        let quality = if notch { 500..=3000 } else { 20..=1200 };
        if !(20..=18_000).contains(&self.hz)
            || !(-6..=6).contains(&self.gain_db)
            || !quality.contains(&self.q)
            || (self.active() && f64::from(self.hz) > f64::from(sample_rate) * 0.45)
        {
            return Err(MediaError::Unsupported(
                "Choose a valid frequency, gain and width for this EQ band",
            ));
        }
        Ok(())
    }

    pub(super) fn filter(self, rate: f64) -> Biquad {
        let q = f64::from(self.q) / 100.0;
        match self.kind {
            EqBandKind::Bell => Biquad::peak(rate, self.hz, self.gain_db, q),
            EqBandKind::Notch => Biquad::notch(rate, self.hz, q),
        }
    }
}

pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<[Option<EqBand>; MAX_EQ_BANDS]>, D::Error> {
    deserialize_slots(deserializer)
}
