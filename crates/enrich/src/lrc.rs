//! Parser for time-synced lyrics in LRC format: `[01:23.45] line of text`.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    pub at: Duration,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct Lyrics {
    /// Sorted by time; empty when only plain lyrics exist.
    pub synced: Vec<LyricLine>,
    pub plain: Option<String>,
}

impl Lyrics {
    /// Index of the line being sung at `position`, if any has started yet.
    pub fn line_at(&self, position: Duration) -> Option<usize> {
        self.synced
            .partition_point(|l| l.at <= position)
            .checked_sub(1)
    }
}

pub fn parse(lrc: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for raw in lrc.lines() {
        // A line may carry several timestamps when a chorus repeats: [00:10.00][01:20.00] text
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(tag_end) = rest.strip_prefix('[').and_then(|r| r.find(']')) {
            // Metadata tags like [ar:Artist] don't parse as timestamps and are dropped.
            stamps.extend(parse_timestamp(&rest[1..=tag_end]));
            rest = &rest[tag_end + 2..];
        }
        let text = rest.trim();
        lines.extend(stamps.into_iter().map(|at| LyricLine {
            at,
            text: text.to_owned(),
        }));
    }
    lines.sort_by_key(|l| l.at);
    lines
}

/// Rough timing for lyrics that come without it, better than none: the lines
/// are spread over the song by their length, after an intro and before an
/// outro, with a pause (an empty line) at each blank line between verses.
/// Section headers like `[Refrén]` are left out.
pub fn estimate(plain: &str, song: Duration) -> Vec<LyricLine> {
    /// Weight of a line on top of its characters: short lines still take a moment.
    const LINE_BASE: f64 = 6.0;
    /// Weight of the pause between verses, about one average line.
    const VERSE_BREAK: f64 = 30.0;

    enum Item<'a> {
        Line(&'a str),
        Break,
    }
    let mut items = Vec::new();
    for line in plain.lines().map(str::trim) {
        let header = line.starts_with('[') && line.ends_with(']');
        if line.is_empty() {
            if matches!(items.last(), Some(Item::Line(_))) {
                items.push(Item::Break);
            }
        } else if !header {
            items.push(Item::Line(line));
        }
    }
    while matches!(items.last(), Some(Item::Break)) {
        items.pop();
    }
    let weight = |item: &Item| match item {
        Item::Line(text) => text.chars().count() as f64 + LINE_BASE,
        Item::Break => VERSE_BREAK,
    };
    let total: f64 = items.iter().map(weight).sum();
    if total == 0.0 {
        return Vec::new();
    }

    let song = song.as_secs_f64();
    let intro = (song * 0.07).clamp(4.0, 15.0);
    let outro = (song * 0.06).clamp(3.0, 12.0);
    let per_weight = (song - intro - outro).max(song * 0.5) / total;
    let mut at = intro;
    let mut lines = Vec::with_capacity(items.len());
    for item in &items {
        let text = match item {
            Item::Line(text) => (*text).to_owned(),
            Item::Break => String::new(),
        };
        lines.push(LyricLine {
            at: Duration::from_secs_f64(at),
            text,
        });
        at += weight(item) * per_weight;
    }
    lines
}

fn parse_timestamp(tag: &str) -> Option<Duration> {
    let (minutes, seconds) = tag.split_once(':')?;
    let minutes: u64 = minutes.parse().ok()?;
    let seconds: f64 = seconds.parse().ok()?;
    (seconds >= 0.0).then(|| Duration::from_secs(minutes * 60) + Duration::from_secs_f64(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_timestamps_metadata_and_repeats() {
        let lines =
            parse("[ar:Someone]\n[00:24.95] Prvni sloka\n[00:10.00][01:05.50] Refrén\n[00:30.00]");
        let at: Vec<_> = lines
            .iter()
            .map(|l| (l.at.as_millis(), l.text.as_str()))
            .collect();
        assert_eq!(
            at,
            [
                (10_000, "Refrén"),
                (24_950, "Prvni sloka"),
                (30_000, ""),
                (65_500, "Refrén")
            ]
        );
    }

    #[test]
    fn estimates_timing_for_plain_lyrics() {
        let plain = "[Sloka 1]\nKratky\nTenhle radek je o dost delsi nez ten prvni\n\n\n[Refrén]\nRefrén\n\n";
        let lines = estimate(plain, Duration::from_secs(200));
        let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Kratky",
                "Tenhle radek je o dost delsi nez ten prvni",
                "",
                "Refrén"
            ]
        );
        // After the intro, in order, and the longer line gets more time.
        assert_eq!(lines[0].at, Duration::from_secs(14));
        assert!(lines.windows(2).all(|w| w[0].at < w[1].at));
        assert!(lines[2].at - lines[1].at > lines[1].at - lines[0].at);
        // The last line starts well before the song ends.
        assert!(lines[3].at < Duration::from_secs(200 - 12));
        assert!(estimate("\n[Intro]\n\n", Duration::from_secs(200)).is_empty());
    }

    #[test]
    fn finds_current_line() {
        let lyrics = Lyrics {
            synced: parse("[00:10.00]a\n[00:20.00]b"),
            plain: None,
        };
        assert_eq!(lyrics.line_at(Duration::from_secs(5)), None);
        assert_eq!(lyrics.line_at(Duration::from_secs(10)), Some(0));
        assert_eq!(lyrics.line_at(Duration::from_secs(25)), Some(1));
    }
}
