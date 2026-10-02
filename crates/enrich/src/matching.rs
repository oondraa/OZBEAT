//! Fuzzy-but-safe matching of search results against what the player reports.
//! Search APIs happily return a different artist with a similar name, so every
//! result is checked before we use it.

const FEATURE_SEPARATORS: &[&str] = &[
    ", ",
    " & ",
    " feat. ",
    " Feat. ",
    " feat ",
    " Feat ",
    " ft. ",
    " Ft. ",
    " featuring ",
];

/// Lowercase, letters and digits only: "P T K" == "PTK", "AC/DC" == "acdc".
pub fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// "A, B feat. C" -> "A". Players list every featured artist; APIs index the main one.
pub fn primary_artist(artist: &str) -> &str {
    let end = FEATURE_SEPARATORS
        .iter()
        .filter_map(|sep| artist.find(sep))
        .min();
    artist[..end.unwrap_or(artist.len())].trim()
}

pub fn same_artist(found: &str, wanted: &str) -> bool {
    let wanted = norm(primary_artist(wanted));
    !wanted.is_empty() && norm(primary_artist(found)) == wanted
}

/// Ignores version suffixes: "Song (Remastered 2011)", "Song - Radio Edit", "Song [Live]".
pub fn same_title(found: &str, wanted: &str) -> bool {
    let found = norm(base_title(found));
    let wanted = norm(base_title(wanted));
    !wanted.is_empty() && found == wanted
}

fn base_title(title: &str) -> &str {
    let end = [" (", " [", " - "]
        .iter()
        .filter_map(|sep| title.find(sep))
        .min();
    let base = title[..end.unwrap_or(title.len())].trim();
    if base.is_empty() { title } else { base }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artists_match_across_spacing_and_features() {
        assert!(same_artist("PTK", "P T K"));
        assert!(same_artist("Calin", "Calin, Viktor Sheen"));
        assert!(same_artist(
            "Daft Punk",
            "Daft Punk feat. Pharrell Williams"
        ));
        assert!(!same_artist("PTK Germany", "P T K"));
        assert!(!same_artist("Anyone", ""));
    }

    #[test]
    fn titles_ignore_version_suffixes() {
        assert!(same_title("NABIJ MI TEN SMOKE", "Nabij mi ten smoke"));
        assert!(same_title(
            "Get Lucky (Radio Edit)",
            "Get Lucky - Radio Edit"
        ));
        assert!(same_title("Song [Live]", "Song"));
        assert!(!same_title("Song Two", "Song"));
    }
}
