#[derive(Clone, Debug)]
pub struct Mp4Sample {
    pub offset: usize,
    pub size: usize,
    pub dts_sec: f64,
    pub pts_sec: f64,
    pub is_keyframe: bool,
}

#[derive(Clone, Debug)]
pub struct Mp4VideoTrack {
    pub track_id: u32,
    pub width: u32,
    pub height: u32,
    pub timescale: u32,
    pub duration_sec: f64,
    pub nal_len_size: usize,
    pub sps_pps_annex_b: Vec<u8>,
    pub samples: Vec<Mp4Sample>,
}

struct TrackBuild {
    track_id: u32,
    handler: [u8; 4],
    timescale: u32,
    duration: u64,
    width: u32,
    height: u32,
    nal_len_size: usize,
    sps_pps_annex_b: Vec<u8>,
    stts: Vec<(u32, u32)>,
    ctts: Vec<(u32, i32)>,
    stsc: Vec<(u32, u32, u32)>,
    sample_sizes: Vec<u32>,
    chunk_offsets: Vec<u64>,
    sync_samples: Vec<u32>,
}

impl Default for TrackBuild {
    fn default() -> Self {
        Self {
            track_id: 0,
            handler: [0; 4],
            timescale: 1000,
            duration: 0,
            width: 0,
            height: 0,
            nal_len_size: 4,
            sps_pps_annex_b: Vec::new(),
            stts: Vec::new(),
            ctts: Vec::new(),
            stsc: Vec::new(),
            sample_sizes: Vec::new(),
            chunk_offsets: Vec::new(),
            sync_samples: Vec::new(),
        }
    }
}

#[derive(Default, Clone, Copy)]
struct TrexDefaults {
    track_id: u32,
    default_sample_duration: u32,
    default_sample_size: u32,
    default_sample_flags: u32,
}

fn read_u16(b: &[u8], off: usize) -> Option<u16> {
    if off + 2 > b.len() {
        return None;
    }
    Some(u16::from_be_bytes([b[off], b[off + 1]]))
}

fn read_u32(b: &[u8], off: usize) -> Option<u32> {
    if off + 4 > b.len() {
        return None;
    }
    Some(u32::from_be_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]]))
}

fn read_i32(b: &[u8], off: usize) -> Option<i32> {
    read_u32(b, off).map(|v| v as i32)
}

fn read_u64(b: &[u8], off: usize) -> Option<u64> {
    if off + 8 > b.len() {
        return None;
    }
    Some(u64::from_be_bytes([
        b[off],
        b[off + 1],
        b[off + 2],
        b[off + 3],
        b[off + 4],
        b[off + 5],
        b[off + 6],
        b[off + 7],
    ]))
}

struct BoxIter<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BoxIter<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
}

struct Mp4Box<'a> {
    kind: [u8; 4],
    start: usize,
    payload: &'a [u8],
}

impl<'a> Iterator for BoxIter<'a> {
    type Item = Mp4Box<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos + 8 > self.data.len() {
            return None;
        }
        let start = self.pos;
        let size32 = read_u32(self.data, self.pos)? as u64;
        let kind = [
            self.data[self.pos + 4],
            self.data[self.pos + 5],
            self.data[self.pos + 6],
            self.data[self.pos + 7],
        ];
        let (hdr_len, box_size) = if size32 == 1 {
            let s64 = read_u64(self.data, self.pos + 8)?;
            (16usize, s64)
        } else if size32 == 0 {
            (8usize, (self.data.len() - self.pos) as u64)
        } else {
            (8usize, size32)
        };
        if box_size < hdr_len as u64 {
            self.pos = self.data.len();
            return None;
        }
        let end = (self.pos as u64).saturating_add(box_size).min(self.data.len() as u64) as usize;
        let payload = &self.data[self.pos + hdr_len..end];
        self.pos = end;
        Some(Mp4Box {
            kind,
            start,
            payload,
        })
    }
}

pub fn parse_mp4_video_track(data: &[u8]) -> Option<Mp4VideoTrack> {
    let mut video_track: Option<TrackBuild> = None;
    let mut movie_timescale = 1000u32;
    let mut movie_duration = 0u64;
    let mut trex_list: Vec<TrexDefaults> = Vec::new();
    let mut moof_ranges: Vec<(usize, &[u8])> = Vec::new();

    for bx in BoxIter::new(data) {
        if &bx.kind == b"moov" {
            for child in BoxIter::new(bx.payload) {
                match &child.kind {
                    b"mvhd" => {
                        let p = child.payload;
                        if !p.is_empty() {
                            if p[0] == 1 && p.len() >= 32 {
                                movie_timescale = read_u32(p, 20).unwrap_or(1000).max(1);
                                movie_duration = read_u64(p, 24).unwrap_or(0);
                            } else if p[0] == 0 && p.len() >= 20 {
                                movie_timescale = read_u32(p, 12).unwrap_or(1000).max(1);
                                movie_duration = read_u32(p, 16).unwrap_or(0) as u64;
                            }
                        }
                    }
                    b"trak" => {
                        let mut tb = TrackBuild::default();
                        parse_trak(child.payload, &mut tb);
                        if &tb.handler == b"vide" && !tb.sps_pps_annex_b.is_empty() {
                            video_track = Some(tb);
                        }
                    }
                    b"mvex" => {
                        for mv in BoxIter::new(child.payload) {
                            if &mv.kind == b"trex" && mv.payload.len() >= 24 {
                                let p = mv.payload;
                                trex_list.push(TrexDefaults {
                                    track_id: read_u32(p, 4).unwrap_or(0),
                                    default_sample_duration: read_u32(p, 12).unwrap_or(0),
                                    default_sample_size: read_u32(p, 16).unwrap_or(0),
                                    default_sample_flags: read_u32(p, 20).unwrap_or(0),
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        } else if &bx.kind == b"moof" {
            moof_ranges.push((bx.start, bx.payload));
        }
    }

    let tb = video_track?;
    let ts = tb.timescale.max(1) as f64;
    let mut samples = build_stbl_samples(&tb, ts);

    if !moof_ranges.is_empty() {
        let trex = trex_list
            .iter()
            .find(|t| t.track_id == tb.track_id)
            .copied()
            .unwrap_or_default();
        let mut running_dts: u64 = 0;
        for (moof_start, moof_payload) in moof_ranges {
            parse_moof(
                moof_start,
                moof_payload,
                tb.track_id,
                &trex,
                ts,
                data.len(),
                &mut running_dts,
                &mut samples,
            );
        }
    }

    if samples.is_empty() {
        return None;
    }

    let track_dur = if tb.duration > 0 {
        tb.duration as f64 / ts
    } else if movie_duration > 0 {
        movie_duration as f64 / (movie_timescale.max(1) as f64)
    } else {
        samples
            .iter()
            .map(|s| s.pts_sec)
            .fold(0.0f64, f64::max)
    };
    let last_pts = samples.iter().map(|s| s.pts_sec).fold(0.0f64, f64::max);
    let duration_sec = track_dur.max(last_pts);

    Some(Mp4VideoTrack {
        track_id: tb.track_id,
        width: tb.width,
        height: tb.height,
        timescale: tb.timescale.max(1),
        duration_sec,
        nal_len_size: tb.nal_len_size.clamp(1, 4),
        sps_pps_annex_b: tb.sps_pps_annex_b,
        samples,
    })
}

fn parse_trak(payload: &[u8], tb: &mut TrackBuild) {
    for bx in BoxIter::new(payload) {
        match &bx.kind {
            b"tkhd" => {
                let p = bx.payload;
                if !p.is_empty() {
                    if p[0] == 1 && p.len() >= 24 {
                        tb.track_id = read_u32(p, 20).unwrap_or(0);
                    } else if p[0] == 0 && p.len() >= 16 {
                        tb.track_id = read_u32(p, 12).unwrap_or(0);
                    }
                }
            }
            b"mdia" => {
                for m in BoxIter::new(bx.payload) {
                    match &m.kind {
                        b"mdhd" => {
                            let p = m.payload;
                            if !p.is_empty() {
                                if p[0] == 1 && p.len() >= 32 {
                                    tb.timescale = read_u32(p, 20).unwrap_or(1000).max(1);
                                    tb.duration = read_u64(p, 24).unwrap_or(0);
                                } else if p[0] == 0 && p.len() >= 20 {
                                    tb.timescale = read_u32(p, 12).unwrap_or(1000).max(1);
                                    tb.duration = read_u32(p, 16).unwrap_or(0) as u64;
                                }
                            }
                        }
                        b"hdlr" => {
                            if m.payload.len() >= 12 {
                                tb.handler = [
                                    m.payload[8],
                                    m.payload[9],
                                    m.payload[10],
                                    m.payload[11],
                                ];
                            }
                        }
                        b"minf" => {
                            for mi in BoxIter::new(m.payload) {
                                if &mi.kind == b"stbl" {
                                    parse_stbl(mi.payload, tb);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

fn parse_stbl(payload: &[u8], tb: &mut TrackBuild) {
    for bx in BoxIter::new(payload) {
        let p = bx.payload;
        match &bx.kind {
            b"stsd" => {
                if p.len() >= 8 {
                    let count = read_u32(p, 4).unwrap_or(0);
                    let mut pos = 8usize;
                    for _ in 0..count {
                        if pos + 8 > p.len() {
                            break;
                        }
                        let sz = read_u32(p, pos).unwrap_or(0) as usize;
                        let kind = [p[pos + 4], p[pos + 5], p[pos + 6], p[pos + 7]];
                        if sz < 8 || pos + sz > p.len() {
                            break;
                        }
                        let entry = &p[pos + 8..pos + sz];
                        if (&kind == b"avc1" || &kind == b"avc3") && entry.len() >= 78 {
                            tb.width = read_u16(entry, 24).unwrap_or(0) as u32;
                            tb.height = read_u16(entry, 26).unwrap_or(0) as u32;
                            for sub in BoxIter::new(&entry[78..]) {
                                if &sub.kind == b"avcC" {
                                    parse_avcc(sub.payload, tb);
                                }
                            }
                        }
                        pos += sz;
                    }
                }
            }
            b"stts" => {
                if p.len() >= 8 {
                    let n = read_u32(p, 4).unwrap_or(0) as usize;
                    for i in 0..n {
                        let off = 8 + i * 8;
                        if let (Some(c), Some(d)) = (read_u32(p, off), read_u32(p, off + 4)) {
                            tb.stts.push((c, d));
                        }
                    }
                }
            }
            b"ctts" => {
                if p.len() >= 8 {
                    let ver = p[0];
                    let n = read_u32(p, 4).unwrap_or(0) as usize;
                    for i in 0..n {
                        let off = 8 + i * 8;
                        if let (Some(c), Some(raw)) = (read_u32(p, off), read_u32(p, off + 4)) {
                            let val = if ver == 0 { raw as i32 } else { raw as i32 };
                            tb.ctts.push((c, val));
                        }
                    }
                }
            }
            b"stsc" => {
                if p.len() >= 8 {
                    let n = read_u32(p, 4).unwrap_or(0) as usize;
                    for i in 0..n {
                        let off = 8 + i * 12;
                        if let (Some(fc), Some(spc), Some(sdi)) = (
                            read_u32(p, off),
                            read_u32(p, off + 4),
                            read_u32(p, off + 8),
                        ) {
                            tb.stsc.push((fc, spc, sdi));
                        }
                    }
                }
            }
            b"stsz" => {
                if p.len() >= 12 {
                    let sample_size = read_u32(p, 4).unwrap_or(0);
                    let count = read_u32(p, 8).unwrap_or(0) as usize;
                    if sample_size > 0 {
                        tb.sample_sizes = vec![sample_size; count.min(2_000_000)];
                    } else {
                        tb.sample_sizes.reserve(count.min(2_000_000));
                        for i in 0..count {
                            if let Some(s) = read_u32(p, 12 + i * 4) {
                                tb.sample_sizes.push(s);
                            } else {
                                break;
                            }
                        }
                    }
                }
            }
            b"stco" => {
                if p.len() >= 8 {
                    let n = read_u32(p, 4).unwrap_or(0) as usize;
                    tb.chunk_offsets.reserve(n.min(2_000_000));
                    for i in 0..n {
                        if let Some(o) = read_u32(p, 8 + i * 4) {
                            tb.chunk_offsets.push(o as u64);
                        } else {
                            break;
                        }
                    }
                }
            }
            b"co64" => {
                if p.len() >= 8 {
                    let n = read_u32(p, 4).unwrap_or(0) as usize;
                    tb.chunk_offsets.reserve(n.min(2_000_000));
                    for i in 0..n {
                        if let Some(o) = read_u64(p, 8 + i * 8) {
                            tb.chunk_offsets.push(o);
                        } else {
                            break;
                        }
                    }
                }
            }
            b"stss" => {
                if p.len() >= 8 {
                    let n = read_u32(p, 4).unwrap_or(0) as usize;
                    tb.sync_samples.reserve(n.min(2_000_000));
                    for i in 0..n {
                        if let Some(s) = read_u32(p, 8 + i * 4) {
                            tb.sync_samples.push(s);
                        } else {
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn parse_avcc(p: &[u8], tb: &mut TrackBuild) {
    if p.len() < 7 {
        return;
    }
    tb.nal_len_size = ((p[4] & 0x03) as usize) + 1;
    let num_sps = (p[5] & 0x1F) as usize;
    let mut pos = 6usize;
    let mut out = Vec::new();
    for _ in 0..num_sps {
        let Some(len) = read_u16(p, pos).map(|v| v as usize) else {
            return;
        };
        pos += 2;
        if pos + len > p.len() {
            return;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&p[pos..pos + len]);
        pos += len;
    }
    if pos >= p.len() {
        tb.sps_pps_annex_b = out;
        return;
    }
    let num_pps = p[pos] as usize;
    pos += 1;
    for _ in 0..num_pps {
        let Some(len) = read_u16(p, pos).map(|v| v as usize) else {
            break;
        };
        pos += 2;
        if pos + len > p.len() {
            break;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&p[pos..pos + len]);
        pos += len;
    }
    tb.sps_pps_annex_b = out;
}

fn build_stbl_samples(tb: &TrackBuild, ts: f64) -> Vec<Mp4Sample> {
    let total = tb.sample_sizes.len();
    if total == 0 || tb.chunk_offsets.is_empty() || tb.stsc.is_empty() {
        return Vec::new();
    }

    let mut dts_list = Vec::with_capacity(total);
    let mut cur_dts = 0u64;
    for &(count, delta) in &tb.stts {
        for _ in 0..count {
            if dts_list.len() >= total {
                break;
            }
            dts_list.push(cur_dts);
            cur_dts = cur_dts.saturating_add(delta as u64);
        }
    }
    while dts_list.len() < total {
        dts_list.push(cur_dts);
    }

    let mut ctts_list = Vec::with_capacity(total);
    for &(count, off) in &tb.ctts {
        for _ in 0..count {
            if ctts_list.len() >= total {
                break;
            }
            ctts_list.push(off);
        }
    }

    let has_sync_table = !tb.sync_samples.is_empty();
    let sync_set: std::collections::HashSet<u32> = tb.sync_samples.iter().copied().collect();

    let mut out = Vec::with_capacity(total);
    let mut sample_idx = 0usize;
    let num_chunks = tb.chunk_offsets.len();
    for (entry_idx, &(first_chunk, samples_per_chunk, _)) in tb.stsc.iter().enumerate() {
        let start_chunk = first_chunk.saturating_sub(1) as usize;
        let end_chunk = if entry_idx + 1 < tb.stsc.len() {
            (tb.stsc[entry_idx + 1].0.saturating_sub(1) as usize).min(num_chunks)
        } else {
            num_chunks
        };
        for chunk in start_chunk..end_chunk {
            let mut offset = tb.chunk_offsets[chunk] as usize;
            for _ in 0..samples_per_chunk {
                if sample_idx >= total {
                    break;
                }
                let size = tb.sample_sizes[sample_idx] as usize;
                let dts = dts_list[sample_idx];
                let cto = ctts_list.get(sample_idx).copied().unwrap_or(0) as i64;
                let pts = (dts as i64).saturating_add(cto).max(0) as u64;
                let is_keyframe = if has_sync_table {
                    sync_set.contains(&((sample_idx + 1) as u32))
                } else {
                    sample_idx == 0
                };
                out.push(Mp4Sample {
                    offset,
                    size,
                    dts_sec: dts as f64 / ts,
                    pts_sec: pts as f64 / ts,
                    is_keyframe,
                });
                offset = offset.saturating_add(size);
                sample_idx += 1;
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn parse_moof(
    moof_start: usize,
    moof_payload: &[u8],
    target_track_id: u32,
    trex: &TrexDefaults,
    ts: f64,
    file_len: usize,
    running_dts: &mut u64,
    out: &mut Vec<Mp4Sample>,
) {
    for child in BoxIter::new(moof_payload) {
        if &child.kind != b"traf" {
            continue;
        }
        let mut track_id = 0u32;
        let mut base_data_offset: Option<u64> = None;
        let mut default_dur = trex.default_sample_duration;
        let mut default_size = trex.default_sample_size;
        let mut default_flags = trex.default_sample_flags;
        let mut base_decode_time: Option<u64> = None;
        let mut truns: Vec<&[u8]> = Vec::new();

        for tb in BoxIter::new(child.payload) {
            let p = tb.payload;
            match &tb.kind {
                b"tfhd" => {
                    if p.len() >= 8 {
                        let flags = ((p[1] as u32) << 16) | ((p[2] as u32) << 8) | (p[3] as u32);
                        track_id = read_u32(p, 4).unwrap_or(0);
                        let mut off = 8usize;
                        if flags & 0x000001 != 0 {
                            base_data_offset = read_u64(p, off);
                            off += 8;
                        }
                        if flags & 0x000002 != 0 {
                            off += 4;
                        }
                        if flags & 0x000008 != 0 {
                            if let Some(d) = read_u32(p, off) {
                                default_dur = d;
                            }
                            off += 4;
                        }
                        if flags & 0x000010 != 0 {
                            if let Some(s) = read_u32(p, off) {
                                default_size = s;
                            }
                            off += 4;
                        }
                        if flags & 0x000020 != 0 {
                            if let Some(f) = read_u32(p, off) {
                                default_flags = f;
                            }
                        }
                    }
                }
                b"tfdt" => {
                    if !p.is_empty() {
                        if p[0] == 1 && p.len() >= 12 {
                            base_decode_time = read_u64(p, 4);
                        } else if p[0] == 0 && p.len() >= 8 {
                            base_decode_time = read_u32(p, 4).map(|v| v as u64);
                        }
                    }
                }
                b"trun" => {
                    truns.push(p);
                }
                _ => {}
            }
        }

        if track_id != target_track_id {
            continue;
        }
        let mut cur_dts = base_decode_time.unwrap_or(*running_dts);
        let base_off = base_data_offset.unwrap_or(moof_start as u64);
        let mut cursor_off = base_off;

        for p in truns {
            if p.len() < 8 {
                continue;
            }
            let ver = p[0];
            let flags = ((p[1] as u32) << 16) | ((p[2] as u32) << 8) | (p[3] as u32);
            let sample_count = read_u32(p, 4).unwrap_or(0) as usize;
            let mut off = 8usize;
            if flags & 0x000001 != 0 {
                if let Some(data_off) = read_i32(p, off) {
                    cursor_off = (base_off as i64).saturating_add(data_off as i64).max(0) as u64;
                }
                off += 4;
            }
            let first_sample_flags = if flags & 0x000004 != 0 {
                let f = read_u32(p, off);
                off += 4;
                f
            } else {
                None
            };
            let has_dur = flags & 0x000100 != 0;
            let has_size = flags & 0x000200 != 0;
            let has_flags = flags & 0x000400 != 0;
            let has_cto = flags & 0x000800 != 0;

            for i in 0..sample_count {
                let dur = if has_dur {
                    let v = read_u32(p, off).unwrap_or(default_dur);
                    off += 4;
                    v
                } else {
                    default_dur
                };
                let size = if has_size {
                    let v = read_u32(p, off).unwrap_or(default_size);
                    off += 4;
                    v
                } else {
                    default_size
                };
                let sflags = if has_flags {
                    let v = read_u32(p, off).unwrap_or(default_flags);
                    off += 4;
                    v
                } else if i == 0 {
                    first_sample_flags.unwrap_or(default_flags)
                } else {
                    default_flags
                };
                let cto: i64 = if has_cto {
                    let raw = read_u32(p, off).unwrap_or(0);
                    off += 4;
                    if ver == 0 {
                        raw as i64
                    } else {
                        (raw as i32) as i64
                    }
                } else {
                    0
                };
                let sample_offset = cursor_off as usize;
                let sample_size = size as usize;
                cursor_off = cursor_off.saturating_add(size as u64);
                let dts = cur_dts;
                cur_dts = cur_dts.saturating_add(dur as u64);
                if sample_offset.saturating_add(sample_size) > file_len {
                    continue;
                }
                let pts = (dts as i64).saturating_add(cto).max(0) as u64;
                let is_non_sync = (sflags & 0x0001_0000) != 0;
                let depends_on = (sflags >> 24) & 0x03;
                let is_keyframe = !is_non_sync && (depends_on == 2 || i == 0);
                out.push(Mp4Sample {
                    offset: sample_offset,
                    size: sample_size,
                    dts_sec: dts as f64 / ts,
                    pts_sec: pts as f64 / ts,
                    is_keyframe,
                });
            }
        }
        *running_dts = cur_dts;
    }
}

pub fn sample_to_annex_b(
    file_bytes: &[u8],
    sample: &Mp4Sample,
    nal_len_size: usize,
    sps_pps_annex_b: &[u8],
    include_sps_pps: bool,
    out: &mut Vec<u8>,
) -> bool {
    out.clear();
    if sample.offset.saturating_add(sample.size) > file_bytes.len() {
        return false;
    }
    if include_sps_pps {
        out.extend_from_slice(sps_pps_annex_b);
    }
    let slice = &file_bytes[sample.offset..sample.offset + sample.size];
    let mut pos = 0usize;
    let mut wrote_nal = false;
    while pos + nal_len_size <= slice.len() {
        let mut nal_len = 0usize;
        for b in &slice[pos..pos + nal_len_size] {
            nal_len = (nal_len << 8) | (*b as usize);
        }
        pos += nal_len_size;
        if nal_len == 0 || pos + nal_len > slice.len() {
            break;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&slice[pos..pos + nal_len]);
        pos += nal_len;
        wrote_nal = true;
    }
    wrote_nal
}
