//! YouTube serves clips as fragmented MP4: an empty sample table up front and
//! the samples listed piece by piece in a `moof` before each `mdat`. Windows'
//! Media Foundation reads that as an empty video, so [`defragment`] rewrites
//! it into a plain MP4 with one sample table, without touching the video
//! itself (no re-encoding, no ffmpeg).

/// A box: its type, where its content starts and where it ends.
struct Mp4Box {
    kind: [u8; 4],
    start: usize,
    body: usize,
    end: usize,
}

/// The boxes in `data[from..to]`.
fn boxes(data: &[u8], from: usize, to: usize) -> Result<Vec<Mp4Box>, String> {
    let mut out = Vec::new();
    let mut at = from;
    while at + 8 <= to {
        let size = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as u64;
        let kind: [u8; 4] = data[at + 4..at + 8].try_into().unwrap();
        let (size, header) = match size {
            0 => ((to - at) as u64, 8),
            1 => {
                let large = data.get(at + 8..at + 16).ok_or("cut off box header")?;
                (u64::from_be_bytes(large.try_into().unwrap()), 16)
            }
            size => (size, 8),
        };
        let end = at
            .checked_add(usize::try_from(size).map_err(|_| "box too big")?)
            .filter(|&end| end <= to && size >= header as u64)
            .ok_or_else(|| format!("broken {} box", String::from_utf8_lossy(&kind)))?;
        out.push(Mp4Box {
            kind,
            start: at,
            body: at + header,
            end,
        });
        at = end;
    }
    Ok(out)
}

/// Reads big-endian numbers from a box's content.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn u32(&mut self) -> Result<u32, String> {
        let bytes = self.data.get(self.at..self.at + 4).ok_or("cut off box")?;
        self.at += 4;
        Ok(u32::from_be_bytes(bytes.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, String> {
        Ok(((self.u32()? as u64) << 32) | self.u32()? as u64)
    }
}

#[derive(Clone, Copy, Default)]
struct Defaults {
    duration: u32,
    size: u32,
    flags: u32,
}

struct Sample {
    offset: usize,
    size: u32,
    duration: u32,
    flags: u32,
    composition: i32,
}

/// Set on every sample but the keyframes.
const NON_SYNC: u32 = 0x1_0000;

/// A plain MP4 with the same video as the fragmented one in `data`. Only
/// single-track files are handled, which is what a video-only clip is.
pub fn defragment(data: &[u8]) -> Result<Vec<u8>, String> {
    let top = boxes(data, 0, data.len())?;
    if !top.iter().any(|b| &b.kind == b"moof") {
        return Ok(data.to_vec()); // plain already
    }
    let find = |kind: &[u8; 4]| top.iter().find(|b| &b.kind == kind);
    let ftyp = find(b"ftyp").ok_or("no ftyp")?;
    let moov = find(b"moov").ok_or("no moov")?;
    let moov_children = boxes(data, moov.body, moov.end)?;
    if moov_children.iter().filter(|b| &b.kind == b"trak").count() != 1 {
        return Err("not a single-track file".into());
    }
    let mut trex = Defaults::default();
    for mvex in moov_children.iter().filter(|b| &b.kind == b"mvex") {
        for b in boxes(data, mvex.body, mvex.end)?.iter().filter(|b| &b.kind == b"trex") {
            let mut r = Reader { data, at: b.body + 12 }; // version/flags, track, description
            trex = Defaults {
                duration: r.u32()?,
                size: r.u32()?,
                flags: r.u32()?,
            };
        }
    }

    // Each fragment's samples, kept together as one chunk.
    let mut chunks: Vec<Vec<Sample>> = Vec::new();
    for moof in top.iter().filter(|b| &b.kind == b"moof") {
        for traf in boxes(data, moof.body, moof.end)?.iter().filter(|b| &b.kind == b"traf") {
            let children = boxes(data, traf.body, traf.end)?;
            let tfhd = children.iter().find(|b| &b.kind == b"tfhd").ok_or("no tfhd")?;
            let mut r = Reader { data, at: tfhd.body };
            let flags = r.u32()? & 0xff_ffff;
            r.u32()?; // track
            let mut base = moof.start;
            if flags & 0x1 != 0 {
                base = usize::try_from(r.u64()?).map_err(|_| "bad base offset")?;
            }
            if flags & 0x2 != 0 {
                r.u32()?; // sample description
            }
            let mut defaults = trex;
            if flags & 0x8 != 0 {
                defaults.duration = r.u32()?;
            }
            if flags & 0x10 != 0 {
                defaults.size = r.u32()?;
            }
            if flags & 0x20 != 0 {
                defaults.flags = r.u32()?;
            }

            for trun in children.iter().filter(|b| &b.kind == b"trun") {
                let mut r = Reader { data, at: trun.body };
                let head = r.u32()?;
                let (version, flags) = (head >> 24, head & 0xff_ffff);
                let count = r.u32()?;
                let mut offset = base;
                if flags & 0x1 != 0 {
                    let delta = r.u32()? as i32 as isize;
                    offset = base.checked_add_signed(delta).ok_or("bad data offset")?;
                }
                let first_flags = if flags & 0x4 != 0 { Some(r.u32()?) } else { None };
                let mut chunk = Vec::with_capacity(count as usize);
                for i in 0..count {
                    let duration = if flags & 0x100 != 0 { r.u32()? } else { defaults.duration };
                    let size = if flags & 0x200 != 0 { r.u32()? } else { defaults.size };
                    let mut sample_flags = if flags & 0x400 != 0 { r.u32()? } else { defaults.flags };
                    if i == 0 && let Some(first) = first_flags {
                        sample_flags = first;
                    }
                    let composition = match (flags & 0x800 != 0, version) {
                        (false, _) => 0,
                        (true, 0) => i32::try_from(r.u32()?).map_err(|_| "bad time offset")?,
                        (true, _) => r.u32()? as i32,
                    };
                    if offset + size as usize > data.len() {
                        return Err("sample past the end of the file".into());
                    }
                    chunk.push(Sample {
                        offset,
                        size,
                        duration,
                        flags: sample_flags,
                        composition,
                    });
                    offset += size as usize;
                }
                chunks.push(chunk);
            }
        }
    }
    chunks.retain(|c| !c.is_empty());
    if chunks.is_empty() {
        return Err("no samples".into());
    }

    let payload: usize = chunks.iter().flatten().map(|s| s.size as usize).sum();
    let mdat_header = if payload + 8 <= u32::MAX as usize { 8 } else { 16 };
    // The moov's size doesn't depend on the offsets in it (co64 entries are
    // fixed width), so lay it out once to learn where the samples will be.
    let probe = rebuild_moov(data, moov, &chunks, 0)?;
    let first = ftyp.end - ftyp.start + probe.len() + mdat_header;
    let moov = rebuild_moov(data, moov, &chunks, first as u64)?;

    let mut out = Vec::with_capacity(first + payload);
    out.extend_from_slice(&data[ftyp.start..ftyp.end]);
    out.extend_from_slice(&moov);
    if mdat_header == 8 {
        out.extend_from_slice(&((payload + 8) as u32).to_be_bytes());
        out.extend_from_slice(b"mdat");
    } else {
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(b"mdat");
        out.extend_from_slice(&((payload + 16) as u64).to_be_bytes());
    }
    for sample in chunks.iter().flatten() {
        out.extend_from_slice(&data[sample.offset..sample.offset + sample.size as usize]);
    }
    Ok(out)
}

/// The moov without `mvex`, and with its sample table describing `chunks`
/// laid out back to back from `first` in the new file.
fn rebuild_moov(
    data: &[u8],
    moov: &Mp4Box,
    chunks: &[Vec<Sample>],
    first: u64,
) -> Result<Vec<u8>, String> {
    fn rebuild(
        data: &[u8],
        b: &Mp4Box,
        table: &dyn Fn() -> Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        const CONTAINERS: [&[u8; 4]; 5] = [b"moov", b"trak", b"mdia", b"minf", b"stbl"];
        const REPLACED: [&[u8; 4]; 8] =
            [b"stts", b"ctts", b"stss", b"stsc", b"stsz", b"stz2", b"stco", b"co64"];
        if !CONTAINERS.contains(&&b.kind) {
            return Ok(data[b.start..b.end].to_vec());
        }
        let mut body = Vec::new();
        for child in boxes(data, b.body, b.end)? {
            if &child.kind == b"mvex" || (&b.kind == b"stbl" && REPLACED.contains(&&child.kind)) {
                continue;
            }
            body.extend(rebuild(data, &child, table)?);
        }
        if &b.kind == b"stbl" {
            body.extend(table());
        }
        Ok(make_box(&b.kind, &body))
    }
    rebuild(data, moov, &|| sample_table(chunks, first))
}

/// stts, ctts, stss, stsz, stsc and co64 for `chunks` stored back to back from `first`.
fn sample_table(chunks: &[Vec<Sample>], first: u64) -> Vec<u8> {
    let samples: Vec<&Sample> = chunks.iter().flatten().collect();
    let mut out = Vec::new();

    let durations = runs(samples.iter().map(|s| s.duration));
    out.extend(full_box(b"stts", 0, &counted(&durations, |&(n, d)| [n, d])));

    if samples.iter().any(|s| s.composition != 0) {
        let offsets = runs(samples.iter().map(|s| s.composition));
        let version = u8::from(samples.iter().any(|s| s.composition < 0));
        out.extend(full_box(b"ctts", version, &counted(&offsets, |&(n, o)| [n, o as u32])));
    }

    if samples.iter().any(|s| s.flags & NON_SYNC != 0) {
        let sync: Vec<u32> = (1..)
            .zip(&samples)
            .filter(|(_, s)| s.flags & NON_SYNC == 0)
            .map(|(n, _)| n)
            .collect();
        out.extend(full_box(b"stss", 0, &counted(&sync, |&n| [n])));
    }

    let mut stsz = 0u32.to_be_bytes().to_vec(); // sizes vary
    stsz.extend(counted(&samples, |s| [s.size]));
    out.extend(full_box(b"stsz", 0, &stsz));

    // Chunk runs of the same length: (first chunk, samples per chunk, description 1).
    let mut stsc: Vec<[u32; 3]> = Vec::new();
    for (i, chunk) in (1..).zip(chunks) {
        let n = chunk.len() as u32;
        if stsc.last().is_none_or(|last| last[1] != n) {
            stsc.push([i, n, 1]);
        }
    }
    out.extend(full_box(b"stsc", 0, &counted(&stsc, |e| *e)));

    let mut co64 = (chunks.len() as u32).to_be_bytes().to_vec();
    let mut at = first;
    for chunk in chunks {
        co64.extend(at.to_be_bytes());
        at += chunk.iter().map(|s| s.size as u64).sum::<u64>();
    }
    out.extend(full_box(b"co64", 0, &co64));
    out
}

/// (repeat count, value) for each run of equal values.
fn runs<T: PartialEq + Copy>(values: impl Iterator<Item = T>) -> Vec<(u32, T)> {
    let mut out: Vec<(u32, T)> = Vec::new();
    for v in values {
        match out.last_mut() {
            Some((n, last)) if *last == v => *n += 1,
            _ => out.push((1, v)),
        }
    }
    out
}

/// An entry count followed by each entry's numbers.
fn counted<T, const N: usize>(entries: &[T], numbers: impl Fn(&T) -> [u32; N]) -> Vec<u8> {
    let mut out = (entries.len() as u32).to_be_bytes().to_vec();
    for e in entries {
        out.extend(numbers(e).iter().flat_map(|n| n.to_be_bytes()));
    }
    out
}

fn full_box(kind: &[u8; 4], version: u8, body: &[u8]) -> Vec<u8> {
    let mut content = vec![version, 0, 0, 0];
    content.extend_from_slice(body);
    make_box(kind, &content)
}

fn make_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moof(sizes: &[u32], first_key_only: bool, data_offset: i32) -> Vec<u8> {
        // tfhd: default-base-is-moof, default duration 512.
        let mut tfhd = vec![0, 2, 0, 8];
        tfhd.extend(1u32.to_be_bytes());
        tfhd.extend(512u32.to_be_bytes());
        // trun: data offset, sizes and flags per sample.
        let mut trun = vec![0, 0, 0x06, 0x01];
        trun.extend((sizes.len() as u32).to_be_bytes());
        trun.extend(data_offset.to_be_bytes());
        for (i, size) in sizes.iter().enumerate() {
            trun.extend(size.to_be_bytes());
            let flags = if i == 0 || !first_key_only { 0 } else { NON_SYNC };
            trun.extend(flags.to_be_bytes());
        }
        let traf = [make_box(b"tfhd", &tfhd), make_box(b"trun", &trun)].concat();
        make_box(b"moof", &make_box(b"traf", &traf))
    }

    /// A fragmented file with two pieces: 3 samples and 2 samples, each
    /// sample's bytes all equal to its number.
    fn fragmented() -> Vec<u8> {
        let stbl = [
            full_box(b"stsd", 0, &[0, 0, 0, 0]),
            full_box(b"stts", 0, &[0, 0, 0, 0]),
            full_box(b"stco", 0, &[0, 0, 0, 0]),
        ]
        .concat();
        let trak = make_box(b"trak", &make_box(b"mdia", &make_box(b"minf", &make_box(b"stbl", &stbl))));
        let mvex = make_box(b"mvex", &full_box(b"trex", 0, &[0; 20]));
        let mut file = make_box(b"ftyp", b"dash");
        file.extend(make_box(b"moov", &[trak, mvex].concat()));
        let mut n = 1u8;
        for sizes in [&[3u32, 1, 2][..], &[4, 2]] {
            // The moof is followed by the mdat header, then the samples.
            let len = moof(sizes, true, 0).len() as i32;
            file.extend(moof(sizes, true, len + 8));
            let payload: Vec<u8> = sizes
                .iter()
                .flat_map(|&s| {
                    n += 1;
                    vec![n - 1; s as usize]
                })
                .collect();
            file.extend(make_box(b"mdat", &payload));
        }
        file
    }

    fn table(file: &[u8], kind: &[u8; 4]) -> Vec<u32> {
        let at = file.windows(4).position(|w| w == kind).expect("box") + 8;
        file[at..]
            .chunks_exact(4)
            .take(12)
            .map(|c| u32::from_be_bytes(c.try_into().unwrap()))
            .collect()
    }

    #[test]
    fn defragments_into_one_sample_table() {
        let plain = defragment(&fragmented()).unwrap();
        assert!(!plain.windows(4).any(|w| w == b"moof" || w == b"mvex" || w == b"stco"));
        assert_eq!(table(&plain, b"stsz")[..7], [0, 5, 3, 1, 2, 4, 2]);
        assert_eq!(table(&plain, b"stts")[..3], [1, 5, 512]);
        // Keyframes: the first sample of each piece.
        assert_eq!(table(&plain, b"stss")[..3], [2, 1, 4]);
        assert_eq!(table(&plain, b"stsc")[..7], [2, 1, 3, 1, 2, 2, 1]);
        // Both chunks point at their samples, in order.
        let co64 = table(&plain, b"co64");
        let (a, b) = (co64[2] as usize, co64[4] as usize);
        assert_eq!(&plain[a..a + 6], &[1, 1, 1, 2, 3, 3]);
        assert_eq!(&plain[b..b + 6], &[4, 4, 4, 4, 5, 5]);
        assert_eq!(plain.len(), b + 6);
    }

    /// `MV_TEST_DASH=in.mp4 MV_TEST_PLAIN=out.mp4 cargo test -- --ignored`
    #[test]
    #[ignore]
    fn defragments_a_real_clip() {
        let input = std::fs::read(std::env::var_os("MV_TEST_DASH").unwrap()).unwrap();
        let plain = defragment(&input).unwrap();
        std::fs::write(std::env::var_os("MV_TEST_PLAIN").unwrap(), plain).unwrap();
    }

    #[test]
    fn rejects_broken_files() {
        assert!(defragment(b"not an mp4 at all").is_err());
        let mut cut = fragmented();
        cut.truncate(cut.len() - 3);
        assert!(defragment(&cut).is_err());
    }
}
