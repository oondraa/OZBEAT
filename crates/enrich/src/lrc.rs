//! Parser for time-synced lyrics in LRC format: `[01:23.45] line of text`.

use std::ops::Range;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    pub at: Duration,
    pub text: String,
    /// Word times from enhanced LRC (`<01:23.45>word`): the character index
    /// each word starts at, and when. Empty when only the line is timed.
    pub words: Vec<(usize, Duration)>,
    /// When the last word ends, if the line says so with a closing tag.
    pub end: Option<Duration>,
}

impl LyricLine {
    fn new(at: Duration, text: String) -> Self {
        Self {
            at,
            text,
            words: Vec::new(),
            end: None,
        }
    }

    /// How many characters of the line are sung at `position`, fractional
    /// inside the word being sung. `next` is when the following line starts
    /// and `pace` the song's usual seconds per syllable, if known. Without
    /// word times, the words are spread over a guess of how long the line is
    /// sung for, each by its syllables.
    pub fn sung_chars(&self, position: Duration, next: Option<Duration>, pace: Option<f64>) -> f32 {
        let len = self.text.chars().count();
        if position <= self.at || len == 0 {
            return 0.0;
        }
        let now = position.as_secs_f64();

        if !self.words.is_empty() {
            let end = self.end.or(next).map_or(f64::INFINITY, |d| d.as_secs_f64());
            for (k, &(from, at)) in self.words.iter().enumerate() {
                let at = at.as_secs_f64();
                let (to, until) = match self.words.get(k + 1) {
                    Some(&(to, until)) => (to, until.as_secs_f64()),
                    None => (len, end.min(at + longest(len - from))),
                };
                if now < until {
                    if now < at {
                        return from as f32;
                    }
                    let t = (now - at) / (until - at).max(1e-3);
                    return from as f32 + (to - from) as f32 * t as f32;
                }
            }
            return len as f32;
        }

        let words = self.weighed_words();
        let per_weight = self.estimated_pace(&words, next, pace);
        let mut left = (now - self.at.as_secs_f64()) / per_weight.max(1e-6);
        for (word, w) in &words {
            if left < w.sung {
                return word.start as f32 + word.len() as f32 * (left / w.sung) as f32;
            }
            // Holds at the word's end through a breath after it.
            if left < w.total() {
                return word.end as f32;
            }
            left -= w.total();
        }
        len as f32
    }

    /// The line's words (split at whitespace, as character ranges) and when
    /// each starts being sung, by the same timing as [`Self::sung_chars`].
    pub fn word_starts(&self, next: Option<Duration>, pace: Option<f64>) -> Vec<(Range<usize>, Duration)> {
        let words = self.weighed_words();
        if !self.words.is_empty() {
            // The timed piece the word's first letter is in.
            let start = |first: usize| {
                self.words
                    .iter()
                    .rev()
                    .find(|&&(from, _)| from <= first)
                    .map_or(self.at, |&(_, at)| at)
            };
            return words
                .into_iter()
                .map(|(w, _)| {
                    let at = start(w.start);
                    (w, at)
                })
                .collect();
        }
        let per_weight = self.estimated_pace(&words, next, pace);
        let mut done = 0.0;
        words
            .into_iter()
            .map(|(w, weight)| {
                let at = self.at + Duration::from_secs_f64(done * per_weight);
                done += weight.total();
                (w, at)
            })
            .collect()
    }

    /// Words as character ranges, split at whitespace, with their weights.
    fn weighed_words(&self) -> Vec<(Range<usize>, Weight)> {
        let chars: Vec<char> = self.text.chars().collect();
        let mut words = Vec::new();
        let mut from = None;
        for (i, c) in chars.iter().enumerate() {
            match (c.is_whitespace(), from) {
                (false, None) => from = Some(i),
                (true, Some(f)) => {
                    words.push(f..i);
                    from = None;
                }
                _ => {}
            }
        }
        if let Some(f) = from {
            words.push(f..chars.len());
        }
        words
            .into_iter()
            .map(|w| {
                let weight = Weight::of(&chars[w.clone()]);
                (w, weight)
            })
            .collect()
    }

    /// The line's length in [`Weight`] units, about its syllables.
    fn weight(&self) -> f64 {
        self.weighed_words().iter().map(|(_, w)| w.total()).sum()
    }

    /// Seconds per [`Weight`] unit when the words' times are guessed: the
    /// line is sung over the time until the next one, or less when that
    /// leaves a pause, judged by the song's usual `pace`.
    fn estimated_pace(
        &self,
        words: &[(Range<usize>, Weight)],
        next: Option<Duration>,
        pace: Option<f64>,
    ) -> f64 {
        /// Leaves a breath before the next line.
        const GAP_SHARE: f64 = 0.9;
        /// How much slower than the song's usual pace a line may be sung.
        const SLACK: f64 = 1.35;
        let total = words.iter().map(|(_, w)| w.total()).sum::<f64>().max(1.0);
        let slowest = pace.map_or(longest(self.text.chars().count()), |pace| {
            0.25 + total * pace * SLACK
        });
        let sung_for = next.map_or(slowest, |n| {
            ((n.as_secs_f64() - self.at.as_secs_f64()) * GAP_SHARE).min(slowest)
        });
        sung_for / total
    }
}

/// The longest a line of `chars` characters takes, so a guessed sweep
/// doesn't crawl through an instrumental gap before the next line.
fn longest(chars: usize) -> f64 {
    /// Slowest pace a line is sung at.
    const MAX_PER_CHAR: f64 = 0.22;
    0.5 + MAX_PER_CHAR * chars as f64
}

/// A word's share of a line's time, in about syllables: the sung part, then
/// a breath when punctuation ends it.
#[derive(Debug, Clone, Copy)]
struct Weight {
    sung: f64,
    pause: f64,
}

impl Weight {
    fn of(word: &[char]) -> Self {
        /// Even a one-syllable word takes a moment to get out.
        const BASE: f64 = 0.35;
        let pause = match word.last() {
            Some('.' | '!' | '?' | '…') => 0.9,
            Some(',' | ';' | ':' | '-' | '–') => 0.5,
            _ => 0.0,
        };
        Self {
            sung: BASE + syllables(word).max(0.5),
            pause,
        }
    }

    fn total(&self) -> f64 {
        self.sung + self.pause
    }
}

/// Syllables in a Czech or English word: each run of vowels is one, and so
/// is an `r` or `l` between consonants, as in „vlk“ or „srdce“. Digits count
/// one each, as they're read out.
fn syllables(word: &[char]) -> f64 {
    let vowel = |c: char| "aeiouyáéěíóúůýäöüàèìòùâêîôû".contains(c);
    let consonant = |c: char| c.is_alphabetic() && !vowel(c);
    let lower: Vec<char> = word.iter().flat_map(|c| c.to_lowercase()).collect();
    let mut count = 0;
    for (i, &c) in lower.iter().enumerate() {
        let prev = i.checked_sub(1).map(|p| lower[p]);
        let next = lower.get(i + 1).copied();
        let nucleus = if vowel(c) {
            !prev.is_some_and(vowel)
        } else if c == 'r' || c == 'l' {
            prev.is_some_and(|p| consonant(p) && p != 'r' && p != 'l')
                && !next.is_some_and(vowel)
        } else {
            c.is_ascii_digit()
        };
        count += usize::from(nucleus);
    }
    count as f64
}

#[derive(Debug, Clone, Default)]
pub struct Lyrics {
    /// Sorted by time; empty when only plain lyrics exist.
    pub synced: Vec<LyricLine>,
    pub plain: Option<String>,
    /// The timing is [`estimate`]d from plain lyrics, too rough to sweep words by.
    pub estimated: bool,
}

impl Lyrics {
    /// Index of the line being sung at `position`, if any has started yet.
    pub fn line_at(&self, position: Duration) -> Option<usize> {
        self.synced
            .partition_point(|l| l.at <= position)
            .checked_sub(1)
    }

    /// Characters of line `i` sung at `position`, see [`LyricLine::sung_chars`].
    pub fn sung_chars(&self, i: usize, position: Duration) -> f32 {
        let next = self.synced.get(i + 1).map(|l| l.at);
        self.synced[i].sung_chars(position, next, self.pace())
    }

    /// When each word of line `i` starts, see [`LyricLine::word_starts`].
    pub fn word_starts(&self, i: usize) -> Vec<(Range<usize>, Duration)> {
        let next = self.synced.get(i + 1).map(|l| l.at);
        self.synced[i].word_starts(next, self.pace())
    }

    /// Whether lyrics belong on screen at `position`: from shortly before
    /// the first line to a little after the last one has been sung, and not
    /// in a long instrumental break. The intro, the outro and such breaks
    /// have nothing to read, so the screen can give the room back.
    pub fn sung_near(&self, position: Duration) -> bool {
        /// Lyrics come in this long before their first word.
        const LEAD: Duration = Duration::from_millis(1500);
        /// And stay this long after the last word.
        const TAIL: Duration = Duration::from_secs(4);
        /// Shorter breaks keep them, so they don't flicker away mid-verse.
        const BREAK: Duration = Duration::from_secs(12);

        let sung = |l: &LyricLine| !l.text.trim().is_empty();
        let Some(i) = self.line_at(position) else {
            return self
                .synced
                .iter()
                .find(|l| sung(l))
                .is_some_and(|l| l.at <= position + LEAD);
        };
        let line = &self.synced[i];
        // A blank line marks where singing stops; otherwise just after the last word.
        let end = if sung(line) {
            line.end.unwrap_or_else(|| {
                self.word_starts(i).last().map_or(line.at, |(_, at)| *at) + Duration::from_millis(800)
            })
        } else {
            line.at
        };
        let next = self.synced[i + 1..].iter().find(|l| sung(l)).map(|l| l.at);
        position <= end + TAIL
            || next.is_some_and(|next| next <= position + LEAD || next.saturating_sub(end) < BREAK)
    }

    /// The song's usual seconds per syllable, from how long each line has
    /// until the next. Some of those gaps hold a pause, so it takes a faster
    /// share than the median: a rapper's line then sweeps at rap speed even
    /// right before a break.
    pub fn pace(&self) -> Option<f64> {
        /// The share of lines sung at least this fast.
        const QUANTILE: f64 = 0.35;
        let mut paces: Vec<f64> = self
            .synced
            .windows(2)
            .filter(|w| w[0].words.is_empty() && !w[0].text.trim().is_empty())
            .map(|w| (w[1].at - w[0].at).as_secs_f64() / w[0].weight().max(1.0))
            .collect();
        if self.estimated || paces.len() < 6 {
            return None;
        }
        paces.sort_by(f64::total_cmp);
        let pace = paces[((paces.len() - 1) as f64 * QUANTILE) as usize];
        Some(pace.clamp(0.08, 0.6))
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
        let (text, words, end) = parse_words(rest.trim());
        lines.extend(stamps.into_iter().map(|at| LyricLine {
            at,
            text: text.clone(),
            words: words.clone(),
            end,
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
        lines.push(LyricLine::new(Duration::from_secs_f64(at), text));
        at += weight(item) * per_weight;
    }
    lines
}

/// Splits enhanced LRC's word tags off a line's text:
/// `<00:12.30>Hello <00:12.80>world<00:13.40>`. A tag after the last word is
/// when the line ends. Anything else in angle brackets stays text.
fn parse_words(raw: &str) -> (String, Vec<(usize, Duration)>, Option<Duration>) {
    let mut text = String::new();
    let mut words = Vec::new();
    let mut rest = raw;
    while let Some(open) = rest.find('<') {
        let tag = rest[open..].find('>').and_then(|close| {
            Some((
                parse_timestamp(&rest[open + 1..open + close])?,
                open + close,
            ))
        });
        let Some((at, close)) = tag else {
            text.push_str(&rest[..=open]);
            rest = &rest[open + 1..];
            continue;
        };
        text.push_str(&rest[..open]);
        words.push((text.chars().count(), at));
        rest = &rest[close + 1..];
    }
    text.push_str(rest);

    // Tags often sit before the space: `<00:12.30> Hello`.
    let chars: Vec<char> = text.chars().collect();
    let lead = chars.iter().take_while(|c| c.is_whitespace()).count();
    let len = text.trim_end().chars().count();
    for (from, _) in &mut words {
        while chars.get(*from).is_some_and(|c| c.is_whitespace()) {
            *from += 1;
        }
    }
    let end = words.pop_if(|(from, _)| *from >= len).map(|(_, at)| at);
    for (from, _) in &mut words {
        *from -= lead;
    }
    words.dedup_by_key(|(from, _)| *from);
    (text.trim().to_owned(), words, end)
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
            estimated: false,
        };
        assert_eq!(lyrics.line_at(Duration::from_secs(5)), None);
        assert_eq!(lyrics.line_at(Duration::from_secs(10)), Some(0));
        assert_eq!(lyrics.line_at(Duration::from_secs(25)), Some(1));
    }

    #[test]
    fn parses_enhanced_word_tags() {
        let line = &parse("[00:12.00]<00:12.30> Hello <00:12.80>big world<00:13.40>")[0];
        assert_eq!(line.text, "Hello big world");
        let words: Vec<_> = line
            .words
            .iter()
            .map(|(c, at)| (*c, at.as_millis()))
            .collect();
        assert_eq!(words, [(0, 12_300), (6, 12_800)]);
        assert_eq!(line.end, Some(Duration::from_millis(13_400)));
        // Halfway through "big world"'s time, halfway through its characters.
        let at = |ms| line.sung_chars(Duration::from_millis(ms), None, None);
        assert_eq!(at(12_000), 0.0);
        assert!((at(13_100) - 10.5).abs() < 0.01);
        assert_eq!(at(14_000), 15.0);
        // Angle brackets that aren't times are text.
        let plain = &parse("[00:01.00]a <b> c")[0];
        assert!(plain.words.is_empty());
        assert_eq!(plain.text, "a <b> c");
    }

    #[test]
    fn estimates_words_inside_a_line() {
        let line = LyricLine::new(Duration::from_secs(10), "aa bbbb".to_owned());
        let next = Some(Duration::from_secs(20));
        let at = |s: f64| line.sung_chars(Duration::from_secs_f64(s), next, None);
        assert_eq!(at(9.0), 0.0);
        assert!(at(10.5) > 0.0 && at(10.5) < at(11.0));
        // Done well before an instrumental gap ends.
        assert_eq!(at(12.5), 7.0);
        // A quick next line squeezes the sweep in before it.
        let quick = line.sung_chars(Duration::from_millis(10_900), Some(Duration::from_secs(11)), None);
        assert_eq!(quick, 7.0);
    }

    #[test]
    fn times_each_word() {
        let secs = |s: f64| Duration::from_secs_f64(s);
        let line = LyricLine::new(secs(10.0), "aa  bbbb c".to_owned());
        let starts = line.word_starts(Some(secs(20.0)), None);
        let ranges: Vec<_> = starts.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(ranges, [0..2, 4..8, 9..10]);
        // Each word starts as the sweep reaches it.
        for (range, at) in &starts {
            let sung = line.sung_chars(*at + secs(0.001), Some(secs(20.0)), None);
            assert!((sung - range.start as f32).abs() < 0.1, "{range:?} at {at:?}: {sung}");
        }

        let (text, words, end) = parse_words("<00:01.00>Hel<00:01.50>lo <00:02.00>world");
        let timed = LyricLine { at: secs(1.0), text, words, end };
        let starts: Vec<_> = timed.word_starts(None, None).into_iter().map(|(_, at)| at).collect();
        assert_eq!(starts, [secs(1.0), secs(2.0)]);
    }

    #[test]
    fn counts_syllables() {
        let count = |w: &str| syllables(&w.chars().collect::<Vec<_>>());
        for (word, n) in [
            ("na", 1.0),
            ("rohu", 2.0),
            ("stojím", 2.0),
            ("Peníze", 3.0),
            ("vlk", 1.0),
            ("srdce", 2.0),
            ("smrt,", 1.0),
            ("Petr", 2.0),
            ("world", 1.0),
            ("strong", 1.0),
            ("people", 2.0),
            ("2024", 4.0),
            ("&", 0.0),
        ] {
            assert_eq!(count(word), n, "{word}");
        }
    }

    #[test]
    fn long_words_and_breaths_take_longer() {
        let secs = |s: f64| Duration::from_secs_f64(s);
        let line = LyricLine::new(secs(10.0), "a peníze, hoří".to_owned());
        let at: Vec<f64> = line
            .word_starts(Some(secs(14.0)), None)
            .iter()
            .map(|(_, at)| at.as_secs_f64() - 10.0)
            .collect();
        // „a“ is short; „peníze,“ has three syllables and a breath after it.
        assert!(at[2] - at[1] > 2.5 * (at[1] - at[0]), "{at:?}");
        // Through the breath, the sweep holds at the comma.
        let comma = 9.0;
        let held = line.sung_chars(secs(10.0) + Duration::from_secs_f64(at[2] - 0.05), Some(secs(14.0)), None);
        assert_eq!(held, comma);
    }

    #[test]
    fn learns_the_songs_pace() {
        // Quick rap lines, two seconds each, then one before a long break.
        let mut lrc = String::new();
        for k in 0..8 {
            lrc += &format!("[00:{:02}.00]jedu jedu jedu jedu jedu\n", 10 + 2 * k);
        }
        lrc += "[00:26.00]jedu jedu jedu jedu jedu\n[00:50.00]konec";
        let lyrics = Lyrics {
            synced: parse(&lrc),
            plain: None,
            estimated: false,
        };
        let pace = lyrics.pace().unwrap();
        assert!((0.1..0.25).contains(&pace), "{pace}");
        // The line before the break sweeps about as fast as the others, not
        // over the whole gap.
        let done = |i: usize| {
            let line = &lyrics.synced[i];
            let len = line.text.chars().count() as f32;
            (0..400)
                .map(|ms| line.at + Duration::from_millis(ms * 10))
                .find(|&p| lyrics.sung_chars(i, p) >= len)
                .unwrap()
                - line.at
        };
        assert!(done(8) < done(0) * 2, "{:?} vs {:?}", done(8), done(0));
    }

    #[test]
    fn lyrics_leave_the_intro_outro_and_long_breaks() {
        let lyrics = Lyrics {
            synced: parse(
                "[00:20.00]<00:20.00>prvni <00:20.50>radek<00:21.00>
                 [00:24.00]druhy radek
                 [00:27.00]
                 [01:10.00]po pauze
                 [01:13.00]posledni",
            ),
            plain: None,
            estimated: false,
        };
        let at = |s: f32| lyrics.sung_near(Duration::from_secs_f32(s));
        assert!(!at(5.0), "intro");
        assert!(at(19.0), "just before the first line");
        assert!(at(22.0), "a short gap between lines");
        assert!(at(30.0), "right after the blank line");
        assert!(!at(45.0), "a long instrumental break");
        assert!(at(69.0), "back before the next line");
        assert!(!at(100.0), "the outro");
        assert!(!Lyrics::default().sung_near(Duration::ZERO));
    }
}
