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
