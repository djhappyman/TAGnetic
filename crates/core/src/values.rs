//! Ordered text values, independent of how a container stores them.
//!
//! This type keeps a literal separator inside a value intact. The legacy string
//! adapter is deliberately fallible: it cannot encode every possible list.
use crate::mask::MaskError;
use std::collections::HashSet;

const MAX_VALUES: usize = 1024;
const MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldValues(Vec<String>);

fn invalid(message: &str) -> MaskError {
    MaskError::BadArgument(message.into())
}

impl FieldValues {
    /// Preserve order, duplicates, whitespace and literal punctuation.
    pub fn new(values: Vec<String>) -> Result<Self, MaskError> {
        if values.len() > MAX_VALUES
            || values.iter().map(String::len).sum::<usize>() > MAX_BYTES
            || values.iter().any(|value| value.contains('\0'))
        {
            return Err(invalid(
                "A field is limited to 1024 values and 1 MiB without NUL.",
            ));
        }
        Ok(Self(values))
    }

    pub fn as_slice(&self) -> &[String] {
        &self.0
    }

    /// Transfer the validated ordered values without changing their meaning.
    pub fn into_vec(self) -> Vec<String> {
        self.0
    }

    /// Append already validated members without copying existing values. A
    /// limit failure leaves the receiver unchanged; no normalization occurs.
    pub fn append(&mut self, other: Self) -> Result<(), MaskError> {
        let bytes = self
            .0
            .iter()
            .chain(&other.0)
            .try_fold(0usize, |sum, value| sum.checked_add(value.len()))
            .unwrap_or(usize::MAX);
        if self.0.len().saturating_add(other.0.len()) > MAX_VALUES || bytes > MAX_BYTES {
            return Err(invalid(
                "The combined field exceeds its ordered-value limits.",
            ));
        }
        self.0.extend(other.0);
        Ok(())
    }

    /// Read the existing joined representation for fields explicitly declared
    /// multiple-valued. Never use this to interpret an ordinary scalar field.
    pub fn from_legacy_list(value: &str, separator: &str) -> Result<Self, MaskError> {
        if separator.is_empty()
            || separator.len() > 1024
            || separator.contains('\0')
            || value.len() > MAX_BYTES
        {
            return Err(invalid(
                "A list needs a nonempty separator and at most 1 MiB.",
            ));
        }
        let values = value
            .split(separator)
            .take(MAX_VALUES + 1)
            .map(str::to_string)
            .collect();
        Self::new(values)
    }

    /// Refuse a conversion that would split one literal value, discard an empty
    /// value or trim meaningful whitespace when the existing writer reads it.
    pub fn to_legacy_list(&self, separator: &str) -> Result<String, MaskError> {
        if separator.is_empty()
            || separator.len() > 1024
            || separator.contains('\0')
            || self
                .0
                .iter()
                .any(|value| value.is_empty() || value.trim() != value || value.contains(separator))
        {
            return Err(invalid(
                "These values cannot be represented losslessly by the current tag adapter.",
            ));
        }
        let size = self.0.iter().map(String::len).sum::<usize>()
            + separator.len() * self.0.len().saturating_sub(1);
        if size > MAX_BYTES {
            return Err(invalid("The combined field exceeds 1 MiB."));
        }
        Ok(self.0.join(separator))
    }

    /// Explicitly split each value, trimming separator spacing and omitting
    /// empty pieces. Fields not selected by the caller are never inspected.
    pub fn split(&self, separator: &str) -> Result<Self, MaskError> {
        if separator.is_empty() || separator.len() > 1024 {
            return Err(invalid("A split separator must contain 1 to 1024 bytes."));
        }
        let values = self
            .0
            .iter()
            .flat_map(|value| value.split(separator))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .take(MAX_VALUES + 1)
            .map(str::to_string)
            .collect();
        Self::new(values)
    }

    pub fn merge(&self, separator: &str) -> Result<Self, MaskError> {
        if separator.len() > 1024 || separator.contains('\0') {
            return Err(invalid(
                "A merge separator is limited to 1024 bytes without NUL.",
            ));
        }
        if self.0.is_empty() {
            return Self::new(Vec::new());
        }
        let size = self.0.iter().map(String::len).sum::<usize>()
            + separator.len() * self.0.len().saturating_sub(1);
        if size > MAX_BYTES {
            return Err(invalid("The merged field exceeds 1 MiB."));
        }
        Self::new(vec![self.0.join(separator)])
    }

    /// Exact, case-sensitive comparison; retain the first spelling and order.
    pub fn deduplicated(&self) -> Self {
        let mut seen = HashSet::new();
        Self(
            self.0
                .iter()
                .filter(|value| seen.insert(value.as_str()))
                .cloned()
                .collect(),
        )
    }
}

impl TryFrom<Vec<String>> for FieldValues {
    type Error = MaskError;

    fn try_from(values: Vec<String>) -> Result<Self, Self::Error> {
        Self::new(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn values(items: &[&str]) -> FieldValues {
        FieldValues::new(items.iter().map(|s| (*s).into()).collect()).unwrap()
    }
    #[test]
    fn values_keep_literal_punctuation_order_and_whitespace() {
        let v = values(&["AC/DC", "Hello; Goodbye", " Björk ", "AC/DC"]);
        assert_eq!(v.as_slice()[1], "Hello; Goodbye");
        assert_eq!(
            v.deduplicated(),
            values(&["AC/DC", "Hello; Goodbye", " Björk "])
        );
        assert!(v.to_legacy_list("; ").is_err());
    }
    #[test]
    fn validated_append_preserves_exact_members_and_limit_failure_is_atomic() {
        let mut first = values(&["A; B", "", " 日本語 "]);
        first.append(values(&["A; B", ""])).unwrap();
        assert_eq!(first, values(&["A; B", "", " 日本語 ", "A; B", ""]));
        let mut full = FieldValues::try_from(vec![String::new(); MAX_VALUES]).unwrap();
        let original = full.clone();
        assert!(full.append(values(&[""])).is_err());
        assert_eq!(full, original);
        let mut full = values(&[&"X".repeat(MAX_BYTES)]);
        assert!(full.append(values(&["Y"])).is_err());
        assert_eq!(full.as_slice()[0].len(), MAX_BYTES);
    }
    #[test]
    fn split_and_merge_are_explicit_and_bounded() {
        let v = values(&[" Björk / 日本語 / ", "AC/DC"]);
        assert_eq!(
            v.split(" / ").unwrap(),
            values(&["Björk", "日本語", "AC/DC"])
        );
        assert_eq!(
            values(&["A", "B"]).merge(" & ").unwrap(),
            values(&["A & B"])
        );
        assert!(v.split("").is_err());
        assert!(values(&[&"x/".repeat(1025)]).split("/").is_err());
        assert!(FieldValues::new(vec!["x".repeat(MAX_BYTES + 1)]).is_err());
    }
    #[test]
    fn legacy_adapter_refuses_loss_instead_of_changing_meaning() {
        assert!(values(&["Hello; Goodbye"]).to_legacy_list("; ").is_err());
        assert!(values(&[""]).to_legacy_list("; ").is_err());
        let v = FieldValues::from_legacy_list("A; B; A; b", "; ").unwrap();
        assert_eq!(v.deduplicated().to_legacy_list("; ").unwrap(), "A; B; b");
        for value in [" A; B", "A;  B", "A; ; B", ""] {
            let values = FieldValues::from_legacy_list(value, "; ").unwrap();
            assert!(
                values.deduplicated().to_legacy_list("; ").is_err(),
                "{value:?}"
            );
        }
    }
}
