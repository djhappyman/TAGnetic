//! Deterministic, read-only interpretation of list-like tag values.
//!
//! Artist punctuation is inherently ambiguous. Preserve the complete credit,
//! support explicit collaboration separators, and retain mixed comma/ampersand
//! monikers as a unit. Whole-credit `equals` remains available to the caller.

use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

static STRONG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)[;\x00\r\n]+|\s+(?:featuring|feat|ft|vs)\.?\s+")
        .expect("static artist separators")
});
static SPACED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s+(?:&|/)\s+").expect("static artist punctuation"));

pub fn is_member_field(field: &str) -> bool {
    matches!(field, "artist" | "albumartist" | "album_artist" | "genre")
}
pub fn normalize_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
pub fn known_artists<'a>(credits: impl Iterator<Item = &'a str>) -> HashSet<String> {
    known_artists_with_cancel(credits, &|| false).unwrap_or_default()
}

pub fn known_artists_with_cancel<'a>(
    credits: impl Iterator<Item = &'a str>,
    cancelled: &impl Fn() -> bool,
) -> Option<HashSet<String>> {
    let empty = HashSet::new();
    let mut unique = HashSet::new();
    for (i, credit) in credits.enumerate() {
        if i % 256 == 0 && cancelled() {
            return None;
        }
        if !credit.trim().is_empty() {
            unique.insert(credit);
        }
    }
    let mut known = HashSet::new();
    for (i, credit) in unique.into_iter().enumerate() {
        if i % 128 == 0 && cancelled() {
            return None;
        }
        for part in artist_parts(credit, &empty) {
            known.insert(part.to_lowercase());
        }
    }
    Some(known)
}

/// Include the full identity plus normalized members, at most once per row.
pub fn members(credit: &str, field: &str, known: &HashSet<String>) -> Vec<String> {
    let whole = normalize_space(credit);
    if whole.is_empty() {
        return Vec::new();
    }
    if credit.len() > 65_536 {
        return vec![whole];
    }
    let parts = if field == "genre" {
        credit
            .split([';', '\0', '\r', '\n'])
            .take(129)
            .map(normalize_space)
            .collect()
    } else {
        artist_parts(credit, known)
    };
    if parts.len() > 128 {
        return vec![whole];
    }
    let mut found = HashSet::new();
    let mut values = Vec::new();
    for value in std::iter::once(whole).chain(parts) {
        if !value.is_empty() && found.insert(value.to_lowercase()) {
            values.push(value);
        }
    }
    values
}

fn artist_parts(credit: &str, known: &HashSet<String>) -> Vec<String> {
    if credit.len() > 65_536 {
        return vec![normalize_space(credit)];
    }
    let mut values = Vec::new();
    for strong in STRONG.split(credit) {
        if values.len() > 128 {
            return vec![normalize_space(credit)];
        }
        let strong = normalize_space(strong);
        if strong.is_empty() {
            continue;
        }
        // A credit such as Earth, Wind & Fire may name one act. A featuring or
        // explicit list separator can still separate that act from another.
        if strong.contains(',') && strong.contains('&') {
            values.push(strong);
            continue;
        }
        if strong.contains(',') {
            values.extend(
                strong
                    .split(',')
                    .map(normalize_space)
                    .filter(|part| !part.is_empty()),
            );
            continue;
        }
        let parts = SPACED
            .split(&strong)
            .map(normalize_space)
            .collect::<Vec<_>>();
        for part in parts {
            // AC/DC and every other unspaced slash remain intact. An unspaced
            // ampersand is interpreted only with existing snapshot evidence.
            let ampersand = part.split('&').map(normalize_space).collect::<Vec<_>>();
            if ampersand.len() > 1
                && ampersand
                    .iter()
                    .all(|name| !name.is_empty() && known.contains(&name.to_lowercase()))
            {
                values.extend(ampersand);
            } else {
                values.push(part);
            }
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn common_credits_preserve_identity_and_exact_members() {
        let known =
            known_artists(["Hixxy & Sharkey", "Hixxy / Dougal", "Hixxy, Seduction"].into_iter());
        for credit in [
            "Hixxy & Sharkey",
            "Hixxy / Dougal",
            "Hixxy, Seduction",
            "Hixxy&Sharkey",
            "Hixxy FEAT. Sharkey",
            "Hixxy ft Dougal",
            "Hixxy vs. Seduction",
        ] {
            assert!(
                members(credit, "artist", &known)
                    .iter()
                    .any(|name| name == "Hixxy"),
                "{credit}"
            );
        }
        assert_eq!(
            members("Hixxy; hixxy", "artist", &known)
                .iter()
                .filter(|name| name.eq_ignore_ascii_case("Hixxy"))
                .count(),
            1
        );
        assert!(!members("Hixxy2 & Sharkey", "artist", &known)
            .iter()
            .any(|name| name == "Hixxy"));
    }
    #[test]
    fn ambiguous_monikers_and_genres_remain_intact() {
        let known = HashSet::new();
        assert_eq!(members("AC/DC", "artist", &known), vec!["AC/DC"]);
        assert_eq!(
            members("Earth, Wind & Fire", "artist", &known),
            vec!["Earth, Wind & Fire"]
        );
        assert_eq!(members("Drum & Bass", "genre", &known), vec!["Drum & Bass"]);
        assert_eq!(
            members("Drum & Bass; Jungle", "genre", &known),
            vec!["Drum & Bass; Jungle", "Drum & Bass", "Jungle"]
        );
        assert_eq!(
            members("Unknown&Moniker", "artist", &known),
            vec!["Unknown&Moniker"]
        );
    }
}
