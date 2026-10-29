//! Icons beside the lyric words they picture ("srdce" ♥, "noc" ☾),
//! from Phosphor's filled icons. The bundled font holds only the glyphs in
//! `icons_table.rs`, so a new icon needs the font subset again
//! (`app/tools/gen_icons.py` writes both).
//!
//! Words ending in `*` in the table are stems, so Czech word forms need no
//! listing: "lásk*" also finds "láska", "lásky" and "láskou".

use std::collections::HashMap;
use std::sync::LazyLock;

include!("icons_table.rs");

/// Font family name of the icons.
pub const FONT: &str = "icons";
pub const FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/PhosphorLyrics.ttf");

/// More would turn a line into a rebus.
const MAX_PER_LINE: usize = 3;
/// A stem matches words this many letters longer at most, and is at least
/// [`STEM_MIN`] letters itself, so "aut" can't find "autobus".
const STEM_ENDING: usize = 4;
const STEM_MIN: usize = 4;

/// Whole words, and stems (without their `*`), each folded.
static TABLE: LazyLock<(HashMap<String, char>, HashMap<String, char>)> = LazyLock::new(|| {
    let (mut words, mut stems) = (HashMap::new(), HashMap::new());
    for &(icon, listed) in ICONS {
        for word in listed {
            match word.strip_suffix('*') {
                Some(stem) => stems.insert(fold(stem), icon),
                None => words.insert(fold(word), icon),
            };
        }
    }
    (words, stems)
});

/// The icon of a folded word: listed as it is, or else by its longest stem.
fn lookup(word: &str) -> Option<char> {
    let (words, stems) = &*TABLE;
    if let Some(&icon) = words.get(word) {
        return Some(icon);
    }
    let chars: Vec<(usize, char)> = word.char_indices().collect();
    (STEM_MIN..=chars.len())
        .rev()
        .take_while(|&k| chars.len() - k <= STEM_ENDING)
        .find_map(|k| {
            let end = chars.get(k).map_or(word.len(), |&(b, _)| b);
            stems.get(&word[..end]).copied()
        })
}

/// The icons to show in a line: after which character (the end of its word)
/// each goes, in order. Each icon shows once per line.
pub fn for_line(text: &str) -> Vec<(usize, char)> {
    let mut found: Vec<(usize, char)> = Vec::new();
    let mut word = String::new();
    let mut end = 0;
    for (i, c) in text.chars().chain([' ']).enumerate() {
        if c.is_alphanumeric() || (c == '\'' && !word.is_empty()) {
            word.push(c);
            end = i + 1;
            continue;
        }
        if let Some(icon) = lookup(&fold(&word))
            && !found.iter().any(|&(_, seen)| seen == icon)
        {
            found.push((end, icon));
            if found.len() == MAX_PER_LINE {
                break;
            }
        }
        word.clear();
    }
    found
}

/// The icon for one word, punctuation around it aside.
pub fn for_word(word: &str) -> Option<char> {
    let word = word.trim_matches(|c: char| !c.is_alphanumeric());
    lookup(&fold(word))
}

/// Lowercase without diacritics, so "LÁSKA", "laska" and "láska" match.
pub(crate) fn fold(word: &str) -> String {
    word.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'ä' => 'a',
            'č' => 'c',
            'ď' => 'd',
            'é' | 'ě' => 'e',
            'í' => 'i',
            'ň' => 'n',
            'ó' | 'ö' => 'o',
            'ř' => 'r',
            'š' => 's',
            'ť' => 't',
            'ú' | 'ů' | 'ü' => 'u',
            'ý' => 'y',
            'ž' => 'z',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_icons_after_words() {
        let heart = for_line("srdce")[0].1;
        // After the word, whatever its case, accents or punctuation.
        assert_eq!(
            for_line("Moje SRDCE, tvoje noc"),
            [(10, heart), (21, for_line("noc")[0].1)]
        );
        assert_eq!(for_line("ty jsi moje laska")[0], (17, heart));
        // Once per line, at most three, and not inside other words.
        assert_eq!(for_line("love love love").len(), 1);
        assert_eq!(for_line("srdce noc auto peníze").len(), MAX_PER_LINE);
        assert!(for_line("lovely autor nocturne").is_empty());
    }

    #[test]
    fn stems_find_word_forms() {
        let heart = for_word("srdce");
        assert!(heart.is_some());
        for form in ["láskou", "Lásky,", "srdcem", "miluješ", "zamilovaná"] {
            assert_eq!(for_word(form), heart, "{form}");
        }
        // The longer stem wins: "motork*" (motorcycle) over "motor*" (engine).
        assert_ne!(for_word("motorkou"), for_word("motoru"));
        // Not when the word goes on too long past the stem.
        assert_eq!(for_word("láskyplnými"), None);
    }

    #[test]
    fn words_have_one_icon() {
        let (words, stems) = &*TABLE;
        for &(icon, listed) in ICONS {
            for word in listed {
                let table = if word.ends_with('*') { stems } else { words };
                let key = fold(word.trim_end_matches('*'));
                assert_eq!(table[&key], icon, "{word} is listed for two icons");
            }
        }
    }
}
