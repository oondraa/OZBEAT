//! Lyrics typed out letter by letter, the way the lyric-typing edits do it:
//! plain black type on a white page, each line typed as it is sung under the
//! lines before it, a few lines to a page, and nothing moving. Or flipping:
//! whole words at once, page and type swapping black and white on each one.
//! Or as a block: three short centered rows, whole words at once.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{
    Color32, FontFamily, FontId, Galley, Painter, Pos2, Rect, Vec2,
    text::{LayoutJob, TextFormat, TextWrapping},
};

use super::{REGULAR, lerp_color};

/// The most lines on a page before it turns.
const PAGE_LINES: usize = 4;
/// The most rows in a block, and the most characters in a row.
const BLOCK_LINES: usize = 3;
const BLOCK_ROW: usize = 12;
/// The block's side, in the lines' sizes, and at most against the window.
const BLOCK_SIDE: f32 = 9.0;
const BLOCK_MEASURE: f32 = 0.86;
/// The least room between words in a block, in type sizes.
const BLOCK_SPACE: f32 = 0.25;
/// A block's type size, against its side.
const BLOCK_TYPE: f32 = 0.15;
/// A block's line height, in its type size: tight.
const BLOCK_LEADING: f32 = 1.05;
/// Lines starting this far apart have a break between them, which turns the page.
const PAGE_BREAK: Duration = Duration::from_secs(7);
/// Seconds to type one letter at most; quicker when the next word comes sooner.
const KEY_TIME: f64 = 0.045;
/// Line height, in type sizes.
const LEADING: f32 = 1.25;
/// The type size against the lines' size.
const TYPING_SCALE: f32 = 0.8;
/// How wide the lines may run, against the window.
const MEASURE: f32 = 0.8;

/// The pages the song's lines fall into, as ranges of lines. A blank line or
/// a long break ends a page, and so does one `full` of lines; blank lines
/// are on none.
fn pages(lyrics: &mv_enrich::Lyrics, full: usize) -> Vec<Range<usize>> {
    let lines = &lyrics.synced;
    let mut pages = Vec::new();
    let mut start: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let blank = line.text.trim().is_empty();
        let gap = i > 0 && line.at.saturating_sub(lines[i - 1].at) > PAGE_BREAK;
        if let Some(s) = start
            && (blank || gap || i - s == full)
        {
            pages.push(s..i);
            start = None;
        }
        if !blank && start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        pages.push(s..lines.len());
    }
    pages
}

/// How many characters of line `i` are typed at `position`, and whether a
/// word is still being typed. Each word is typed the moment it is sung, a
/// letter at a time, with the space after it, or `whole` all at once.
fn typed(lyrics: &mv_enrich::Lyrics, i: usize, position: Duration, whole: bool) -> (usize, bool) {
    let len = lyrics.synced[i].text.chars().count();
    let starts = lyrics.word_starts(i);
    let (mut n, mut typing) = (0, false);
    for (k, (range, at)) in starts.iter().enumerate() {
        if position < *at {
            break;
        }
        if whole {
            n = range.end;
            continue;
        }
        let keys = range.len() + 1;
        let key = starts.get(k + 1).map_or(KEY_TIME, |(_, next)| {
            ((*next - *at).as_secs_f64() / keys as f64).min(KEY_TIME)
        });
        let done = (((position - *at).as_secs_f64() / key.max(1e-3)) as usize + 1).min(keys);
        n = (range.start + done).min(len);
        typing = done < keys;
    }
    (n, typing)
}

/// Whether the page is black at `position`: it swaps on every word sung.
pub(super) fn flipped(lyrics: &mv_enrich::Lyrics, position: Duration) -> bool {
    let Some(i) = lyrics.line_at(position) else {
        return false;
    };
    let before: usize = lyrics.synced[..i].iter().map(|l| l.text.split_whitespace().count()).sum();
    let now = lyrics.word_starts(i).iter().filter(|(_, at)| *at <= position).count();
    (before + now) % 2 == 1
}

/// The lyrics typed out (see the module) around `center_y`, or with `flip`
/// whole words at a time. `page` (0 -> 1) is how much of the window is their
/// page, drawn apart by [`draw_page`]: on it the type is black (white on a
/// [`flipped`] one), off it white, and with less room than a whole window
/// only the last two lines show. Over a music video (`video`) white type
/// gets a shadow.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_typing(
    painter: &Painter,
    rect: Rect,
    lyrics: &mv_enrich::Lyrics,
    position: Duration,
    center_y: f32,
    size: f32,
    page: f32,
    video: f32,
    flip: bool,
) {
    let Some(i) = lyrics.line_at(position) else {
        return;
    };
    // A blank line or a break between pages: a clean page.
    let Some(lines) = pages(lyrics, PAGE_LINES).into_iter().find(|p| p.contains(&i)) else {
        return;
    };
    let lines = if page >= 0.5 {
        lines
    } else {
        lines.start.max(i.saturating_sub(1))..i + 1
    };

    // Rasters snapped to 4 px steps, so a resized window doesn't keep adding sizes.
    let px = ((size * TYPING_SCALE / 4.0).round() * 4.0).max(12.0);
    let on_page = if flip && flipped(lyrics, position) { Color32::WHITE } else { Color32::BLACK };
    let ink = lerp_color(Color32::WHITE, on_page, page);
    let width = rect.width() * MEASURE;
    let layout = |text: String| {
        let mut job = LayoutJob::single_section(
            text,
            TextFormat {
                font_id: FontId::new(px, FontFamily::Name(REGULAR.into())),
                color: ink,
                ..Default::default()
            },
        );
        job.wrap = TextWrapping::wrap_at_width(width);
        painter.layout_job(job)
    };

    // Laid out whole first, so the page stands still as it fills: the block
    // centered on `center_y`, its lines flush left.
    let whole: Vec<_> = lines.clone().map(|j| layout(lyrics.synced[j].text.trim_end().to_owned())).collect();
    let leading = px * (LEADING - 1.0);
    let height = whole.iter().map(|g| g.size().y).sum::<f32>() + leading * (whole.len() as f32 - 1.0);
    let block = whole.iter().map(|g| g.size().x).fold(0.0, f32::max);
    let left = rect.center().x - block / 2.0;
    let mut top = center_y - height / 2.0;

    let shadow = Color32::from_black_alpha((140.0 * video * (1.0 - page)) as u8);
    for (j, full) in lines.zip(&whole) {
        if j > i {
            break;
        }
        let pos = Pos2::new(left, top);
        top += full.size().y + leading;
        let (galley, typing) = if j < i {
            (full.clone(), false)
        } else {
            let (n, typing) = typed(lyrics, i, position, flip);
            (layout(lyrics.synced[i].text.chars().take(n).collect()), typing)
        };
        if shadow.a() > 0 {
            let drop = Vec2::splat(px * 0.05);
            painter.galley_with_override_text_color(pos + drop, galley.clone(), shadow);
        }
        if j == i && !flip {
            // The cursor, solid while typing and blinking while it waits.
            let blink = position.as_secs_f32().fract() < 0.5;
            if typing || blink {
                let row = galley.rows.last().map_or(Rect::from_min_size(Pos2::ZERO, Vec2::new(0.0, px)), |r| r.rect());
                let bar = Rect::from_min_size(
                    pos + Vec2::new(row.right() + px * 0.06, row.top() + px * 0.12),
                    Vec2::new((px * 0.06).max(1.5), row.height() - px * 0.24),
                );
                painter.rect_filled(bar, 0.0, ink);
            }
        }
        painter.galley(pos, galley, ink);
    }
}

/// A sung word, for the block.
pub(super) struct Word {
    pub text: String,
    pub at: Duration,
    /// Starts a new block: after a blank line or a long break.
    pub fresh: bool,
}

/// The song's words in order, each marked whether it starts a new block;
/// with `by_line`, every line starts one.
pub(super) fn song_words(lyrics: &mv_enrich::Lyrics, by_line: bool) -> Vec<Word> {
    let mut words = Vec::new();
    let mut fresh = true;
    for (i, line) in lyrics.synced.iter().enumerate() {
        if by_line || line.text.trim().is_empty() || (i > 0 && line.at.saturating_sub(lyrics.synced[i - 1].at) > PAGE_BREAK) {
            fresh = true;
        }
        for (range, at) in lyrics.word_starts(i) {
            let text = line.text.chars().skip(range.start).take(range.len()).collect();
            words.push(Word { text, at, fresh });
            fresh = false;
        }
    }
    words
}

/// The words cut into blocks of rows, as ranges of words: a row takes words
/// while they fit in `row_chars` characters (at least one), a block takes
/// `rows` rows, and a fresh word starts a new block.
pub(super) fn blocks(words: &[Word], row_chars: usize, rows_max: usize) -> Vec<Vec<Range<usize>>> {
    let mut blocks: Vec<Vec<Range<usize>>> = Vec::new();
    for (k, word) in words.iter().enumerate() {
        let len = word.text.chars().count();
        let Some(rows) = blocks.last_mut().filter(|_| !word.fresh) else {
            blocks.push(vec![Range { start: k, end: k + 1 }]);
            continue;
        };
        let row = rows.last_mut().expect("a block has a row");
        let used: usize = words[row.clone()].iter().map(|w| w.text.chars().count() + 1).sum();
        if used + len <= row_chars {
            row.end = k + 1;
        } else if rows.len() < rows_max {
            rows.push(k..k + 1);
        } else {
            blocks.push(vec![Range { start: k, end: k + 1 }]);
        }
    }
    blocks
}

/// The lyrics as a block (see the module): the words sung lately set in a
/// few short centered rows, all at one size, stacked tight into a square
/// around `center_y`. Each word
/// shows whole as it is sung; when the next doesn't fit, a new block begins.
/// `page` and `video` as for [`draw_typing`]; with less room than a whole
/// window only the row being sung shows.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_block(
    painter: &Painter,
    rect: Rect,
    lyrics: &mv_enrich::Lyrics,
    position: Duration,
    center_y: f32,
    size: f32,
    page: f32,
    video: f32,
) {
    /// Size the rows are measured at before being fitted.
    const MEASURE_PX: f32 = 64.0;
    let words = song_words(lyrics, false);
    let Some(current) = words.partition_point(|w| w.at <= position).checked_sub(1) else {
        return;
    };
    let Some(rows) = blocks(&words, BLOCK_ROW, BLOCK_LINES).into_iter().find(|rows| rows.iter().any(|r| r.contains(&current))) else {
        return;
    };
    let rows = if page >= 0.5 {
        rows
    } else {
        rows.into_iter().filter(|r| r.contains(&current)).collect()
    };

    let side = (size * BLOCK_SIDE).min(rect.width() * BLOCK_MEASURE);
    // Rasters snapped to 2 px steps, so a resized window doesn't keep adding sizes.
    let snap = |px: f32| ((px / 2.0).floor() * 2.0).max(10.0);
    let ink = lerp_color(Color32::WHITE, Color32::BLACK, page);
    let shadow = Color32::from_black_alpha((140.0 * video * (1.0 - page)) as u8);
    let layout = |text: &str, px: f32| {
        painter.layout_no_wrap(text.to_owned(), FontId::new(px, FontFamily::Name(REGULAR.into())), ink)
    };

    // Every row of the block fitted first, so it stands still as it fills.
    struct Set {
        galleys: Vec<Arc<Galley>>,
        range: Range<usize>,
        px: f32,
        gap: f32,
        width: f32,
    }
    let sets: Vec<Set> = rows
        .into_iter()
        .map(|range| {
            let row = &words[range.clone()];
            let fit = |px: f32| -> (Vec<_>, f32) {
                let galleys: Vec<_> = row.iter().map(|w| layout(&w.text, px)).collect();
                let width = galleys.iter().map(|g| g.size().x).sum();
                (galleys, width)
            };
            let gaps = row.len().saturating_sub(1) as f32;
            // One size for all rows; a word too long for the block is set smaller.
            let (_, width) = fit(MEASURE_PX);
            let need = (width + MEASURE_PX * BLOCK_SPACE * gaps).max(1.0);
            let px = snap((MEASURE_PX * side / need).min(side * BLOCK_TYPE));
            let (galleys, width) = fit(px);
            let gap = px * BLOCK_SPACE;
            Set { galleys, range, px, gap, width: width + gap * gaps }
        })
        .collect();

    let height: f32 = sets.iter().map(|s| s.px * BLOCK_LEADING).sum();
    let mut top = center_y - height / 2.0;
    for set in sets {
        let middle = top + set.px * BLOCK_LEADING / 2.0;
        top += set.px * BLOCK_LEADING;
        let mut x = rect.center().x - set.width / 2.0;
        for (galley, k) in set.galleys.into_iter().zip(set.range) {
            let w = galley.size().x;
            if k <= current {
                let pos = Pos2::new(x, middle - galley.size().y / 2.0);
                if shadow.a() > 0 {
                    let drop = Vec2::splat(set.px * 0.05);
                    painter.galley_with_override_text_color(pos + drop, galley.clone(), shadow);
                }
                painter.galley(pos, galley, ink);
            }
            x += w + set.gap;
        }
    }
}

/// The page under typed lyrics, white or `dark`, `amount` (0 -> 1) of the way in.
pub(super) fn draw_page(painter: &Painter, rect: Rect, amount: f32, dark: bool) {
    if amount > 0.0 {
        let a = (255.0 * amount) as u8;
        let color = if dark { Color32::from_black_alpha(a) } else { Color32::from_white_alpha(a) };
        painter.rect_filled(rect, 0.0, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mv_enrich::{LyricLine, Lyrics};

    fn lyrics(lines: &[(f64, &str)]) -> Lyrics {
        Lyrics {
            synced: lines
                .iter()
                .map(|&(at, text)| LyricLine {
                    at: Duration::from_secs_f64(at),
                    text: text.to_owned(),
                    words: Vec::new(),
                    end: None,
                })
                .collect(),
            plain: None,
            estimated: false,
        }
    }

    #[test]
    fn pages_turn_at_blanks_breaks_and_when_full() {
        let l = lyrics(&[
            (0.0, "a"),
            (2.0, "b"),
            (4.0, ""),
            (5.0, "c"),
            (7.0, "d"),
            (9.0, "e"),
            (11.0, "f"),
            (13.0, "g"),
            (30.0, "h"),
        ]);
        assert_eq!(pages(&l, 4), vec![0..2, 3..7, 7..8, 8..9]);
    }

    #[test]
    fn words_are_typed_as_they_are_sung() {
        let l = lyrics(&[(10.0, "ahoj svete"), (20.0, "x")]);
        let at = |s: f64| typed(&l, 0, Duration::from_secs_f64(s), false);
        assert_eq!(at(10.0), (1, true));
        // The first word and its space, then a wait for the next word.
        let (n, _) = at(10.5);
        assert!((5..10).contains(&n), "{n}");
        assert_eq!(at(19.9), (10, false));
        // Whole, the first word shows at once.
        assert_eq!(typed(&l, 0, Duration::from_secs_f64(10.0), true), (4, false));
    }

    #[test]
    fn blocks_fill_short_rows_and_start_fresh_after_a_break() {
        let l = lyrics(&[
            (0.0, "celou noc jedu městem sám"),
            (3.0, "a pak"),
            (4.0, ""),
            (5.0, "nic"),
        ]);
        let words = song_words(&l, false);
        let rows = blocks(&words, BLOCK_ROW, BLOCK_LINES);
        // "celou noc" | "jedu městem" | "sám a pak", then "nic" after the blank line.
        assert_eq!(rows, vec![vec![0..2, 2..4, 4..7], vec![7..8]]);
    }

    #[test]
    fn the_page_swaps_on_every_word() {
        let l = lyrics(&[(10.0, "jedna dva"), (20.0, "tri")]);
        let at = |s: f64| flipped(&l, Duration::from_secs_f64(s));
        assert!(!at(9.0));
        assert!(at(10.0));
        assert!(!at(19.0));
        assert!(at(20.0));
    }
}
