use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use peedle_net::{HttpClient, HttpMethod, HttpRequest};
use peedle_url::Url;

use crate::audio_decode::{StreamingAudioDecoder, OUTPUT_CHANNELS, OUTPUT_SAMPLE_RATE};
use crate::audio_sink::AudioSink;
use crate::mp4_demux::{parse_mp4_video_track, Mp4VideoTrack};
use crate::video_decode::{H264VideoDecoder, VideoFrame};

#[derive(Clone, Debug)]
pub struct MediaSnapshot {
    pub loading: bool,
    pub playing: bool,
    pub ended: bool,
    pub current_time: f64,
    pub duration: f64,
    pub buffered_sec: f64,
    pub volume: f32,
    pub muted: bool,
    pub playback_rate: f32,
    pub video_width: u32,
    pub video_height: u32,
    pub has_audio: bool,
    pub error: Option<String>,
    pub frame_seq: u64,
}

impl Default for MediaSnapshot {
    fn default() -> Self {
        Self {
            loading: true,
            playing: false,
            ended: false,
            current_time: 0.0,
            duration: 0.0,
            buffered_sec: 0.0,
            volume: 1.0,
            muted: false,
            playback_rate: 1.0,
            video_width: 0,
            video_height: 0,
            has_audio: false,
            error: None,
            frame_seq: 0,
        }
    }
}

enum PlayerCmd {
    Play,
    Pause,
    Seek(f64),
    SetVolume(f32),
    SetMuted(bool),
    SetRate(f32),
    SwitchSources {
        video_url: String,
        audio_url: Option<String>,
        resume_time: f64,
    },
    Stop,
}

struct SharedState {
    snapshot: MediaSnapshot,
    current_frame: Option<Arc<VideoFrame>>,
    cmds: VecDeque<PlayerCmd>,
}

pub struct MediaPlayer {
    shared: Arc<Mutex<SharedState>>,
    stop_flag: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl MediaPlayer {
    pub fn open(
        client: HttpClient,
        video_url: String,
        audio_url: Option<String>,
        autoplay: bool,
    ) -> Self {
        let mut snap = MediaSnapshot::default();
        snap.playing = autoplay;
        let shared = Arc::new(Mutex::new(SharedState {
            snapshot: snap,
            current_frame: None,
            cmds: VecDeque::new(),
        }));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let sh = Arc::clone(&shared);
        let sf = Arc::clone(&stop_flag);
        let worker = thread::spawn(move || {
            run_player_thread(client, video_url, audio_url, autoplay, sh, sf);
        });
        Self {
            shared,
            stop_flag,
            worker: Some(worker),
        }
    }

    pub fn play(&self) {
        let mut st = self.shared.lock().unwrap();
        st.snapshot.playing = true;
        st.snapshot.ended = false;
        st.cmds.push_back(PlayerCmd::Play);
    }

    pub fn pause(&self) {
        let mut st = self.shared.lock().unwrap();
        st.snapshot.playing = false;
        st.cmds.push_back(PlayerCmd::Pause);
    }

    pub fn toggle_play(&self) {
        let playing = self.shared.lock().unwrap().snapshot.playing;
        if playing {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn seek(&self, secs: f64) {
        let mut st = self.shared.lock().unwrap();
        let dur = st.snapshot.duration;
        let target = if dur > 0.0 {
            secs.clamp(0.0, dur)
        } else {
            secs.max(0.0)
        };
        st.snapshot.current_time = target;
        st.snapshot.ended = false;
        st.cmds.push_back(PlayerCmd::Seek(target));
    }

    pub fn set_volume(&self, vol: f32) {
        let v = vol.clamp(0.0, 1.0);
        let mut st = self.shared.lock().unwrap();
        st.snapshot.volume = v;
        if v > 0.0 {
            st.snapshot.muted = false;
        }
        st.cmds.push_back(PlayerCmd::SetVolume(v));
    }

    pub fn set_muted(&self, muted: bool) {
        let mut st = self.shared.lock().unwrap();
        st.snapshot.muted = muted;
        st.cmds.push_back(PlayerCmd::SetMuted(muted));
    }

    pub fn toggle_mute(&self) {
        let m = self.shared.lock().unwrap().snapshot.muted;
        self.set_muted(!m);
    }

    pub fn set_playback_rate(&self, rate: f32) {
        let r = rate.clamp(0.25, 4.0);
        let mut st = self.shared.lock().unwrap();
        st.snapshot.playback_rate = r;
        st.cmds.push_back(PlayerCmd::SetRate(r));
    }

    pub fn switch_sources(&self, video_url: String, audio_url: Option<String>) {
        let mut st = self.shared.lock().unwrap();
        let resume_time = st.snapshot.current_time;
        st.snapshot.loading = true;
        st.cmds.push_back(PlayerCmd::SwitchSources {
            video_url,
            audio_url,
            resume_time,
        });
    }

    pub fn snapshot(&self) -> MediaSnapshot {
        self.shared.lock().unwrap().snapshot.clone()
    }

    pub fn current_frame(&self) -> Option<Arc<VideoFrame>> {
        self.shared.lock().unwrap().current_frame.clone()
    }
}

impl Drop for MediaPlayer {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Ok(mut st) = self.shared.lock() {
            st.cmds.push_back(PlayerCmd::Stop);
        }
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

struct LoadedMedia {
    video_bytes: Vec<u8>,
    video_track: Option<Mp4VideoTrack>,
    audio_decoder: Option<StreamingAudioDecoder>,
    duration_sec: f64,
}

fn parse_content_range_total(val: &str) -> Option<usize> {
    let slash = val.rfind('/')?;
    val[slash + 1..].trim().parse::<usize>().ok()
}

fn media_headers_for_url(url: &str, range: Option<String>) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    if let Some(r) = range {
        headers.push(("Range".to_string(), r));
    }
    if url.contains("googlevideo.com") && url.contains("c=ANDROID") {
        headers.push((
            "User-Agent".to_string(),
            "com.google.android.youtube/20.10.38 (Linux; U; Android 14) gzip".to_string(),
        ));
    }
    headers
}

fn fetch_url_bytes(client: &HttpClient, url: &str) -> Result<Vec<u8>, String> {
    if let Some(path) = url.strip_prefix("file://") {
        let clean = if cfg!(windows)
            && path.starts_with('/')
            && path.len() > 2
            && path.as_bytes()[2] == b':'
        {
            &path[1..]
        } else {
            path
        };
        return std::fs::read(clean).map_err(|e| format!("file read error: {}", e));
    }
    let parsed = Url::parse(url).map_err(|e| format!("bad url: {}", e))?;
    let chunk_size: usize = 4 * 1024 * 1024;
    let first_req = HttpRequest {
        method: HttpMethod::Get,
        url: parsed.clone(),
        headers: media_headers_for_url(url, Some(format!("bytes=0-{}", chunk_size - 1))),
        body: None,
        max_redirects: 10,
    };
    let first = client
        .send(first_req)
        .map_err(|e| format!("http fetch error: {}", e))?;
    if first.status >= 400 {
        let fallback_req = HttpRequest {
            method: HttpMethod::Get,
            url: parsed,
            headers: media_headers_for_url(url, None),
            body: None,
            max_redirects: 10,
        };
        let fallback = client
            .send(fallback_req)
            .map_err(|e| format!("http fetch error: {}", e))?;
        if fallback.status >= 400 {
            return Err(format!("HTTP {}", fallback.status));
        }
        return Ok(fallback.body);
    }
    if first.status != 206 {
        return Ok(first.body);
    }
    let total = first
        .header("content-range")
        .and_then(parse_content_range_total)
        .unwrap_or(first.body.len());
    if first.body.len() >= total {
        return Ok(first.body);
    }
    let cap_total = total.min(128 * 1024 * 1024);
    let mut ranges = Vec::new();
    let mut off = first.body.len();
    while off < cap_total {
        let end = (off + chunk_size - 1).min(cap_total - 1);
        ranges.push((off, end));
        off = end + 1;
    }
    let mut out = Vec::with_capacity(cap_total);
    out.extend_from_slice(&first.body);
    let parts: Mutex<Vec<(usize, Vec<u8>)>> = Mutex::new(Vec::new());
    let queue = Mutex::new(VecDeque::from(ranges));
    let workers = queue.lock().unwrap().len().min(4);
    let final_url = first.url;
    let final_url_str = final_url.href();
    thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let item = queue.lock().unwrap().pop_front();
                let Some((start, end)) = item else {
                    break;
                };
                let req = HttpRequest {
                    method: HttpMethod::Get,
                    url: final_url.clone(),
                    headers: media_headers_for_url(
                        &final_url_str,
                        Some(format!("bytes={}-{}", start, end)),
                    ),
                    body: None,
                    max_redirects: 10,
                };
                if let Ok(r) = client.send(req) {
                    if r.status < 400 && !r.body.is_empty() {
                        parts.lock().unwrap().push((start, r.body));
                    }
                }
            });
        }
    });
    let mut collected = parts.into_inner().unwrap();
    collected.sort_by_key(|(start, _)| *start);
    for (start, bytes) in collected {
        if start == out.len() {
            out.extend_from_slice(&bytes);
        }
    }
    Ok(out)
}

fn hint_from_url(url: &str) -> Option<&'static str> {
    let low = url.to_ascii_lowercase();
    if low.contains(".webm") || low.contains("mime=video%2fwebm") || low.contains("mime=audio%2fwebm") {
        Some("webm")
    } else if low.contains(".ogg") || low.contains(".opus") {
        Some("ogg")
    } else if low.contains(".mp3") {
        Some("mp3")
    } else if low.contains(".wav") {
        Some("wav")
    } else if low.contains(".flac") {
        Some("flac")
    } else {
        Some("mp4")
    }
}

fn load_media_sources(
    client: &HttpClient,
    video_url: &str,
    audio_url: Option<&str>,
) -> Result<LoadedMedia, String> {
    let (v_res, a_res) = if let Some(aurl) = audio_url {
        let c1 = client.clone();
        let c2 = client.clone();
        let vu = video_url.to_string();
        let au = aurl.to_string();
        let t1 = thread::spawn(move || fetch_url_bytes(&c1, &vu));
        let t2 = thread::spawn(move || fetch_url_bytes(&c2, &au));
        (
            t1.join().unwrap_or_else(|_| Err("video fetch panic".into())),
            Some(t2.join().unwrap_or_else(|_| Err("audio fetch panic".into()))),
        )
    } else {
        (fetch_url_bytes(client, video_url), None)
    };

    let video_bytes = v_res?;
    let video_track = parse_mp4_video_track(&video_bytes);
    let audio_decoder = match a_res {
        Some(Ok(abytes)) => StreamingAudioDecoder::open(abytes, audio_url.and_then(hint_from_url)).ok(),
        Some(Err(_)) => StreamingAudioDecoder::open(video_bytes.clone(), hint_from_url(video_url)).ok(),
        None => StreamingAudioDecoder::open(video_bytes.clone(), hint_from_url(video_url)).ok(),
    };
    if video_track.is_none() && audio_decoder.is_none() {
        return Err("unsupported media format or codec".to_string());
    }
    let v_dur = video_track.as_ref().map(|t| t.duration_sec).unwrap_or(0.0);
    let a_dur = audio_decoder.as_ref().map(|a| a.duration_sec()).unwrap_or(0.0);
    Ok(LoadedMedia {
        video_bytes,
        video_track,
        audio_decoder,
        duration_sec: v_dur.max(a_dur),
    })
}

fn find_keyframe_index(track: &Mp4VideoTrack, target_sec: f64) -> usize {
    let mut best_kf = 0usize;
    for (i, s) in track.samples.iter().enumerate() {
        if s.dts_sec > target_sec + 0.001 {
            break;
        }
        if s.is_keyframe {
            best_kf = i;
        }
    }
    best_kf
}

fn run_player_thread(
    client: HttpClient,
    initial_video_url: String,
    initial_audio_url: Option<String>,
    autoplay: bool,
    shared: Arc<Mutex<SharedState>>,
    stop_flag: Arc<AtomicBool>,
) {
    let mut loaded = match load_media_sources(
        &client,
        &initial_video_url,
        initial_audio_url.as_deref(),
    ) {
        Ok(m) => m,
        Err(e) => {
            if let Ok(mut st) = shared.lock() {
                st.snapshot.loading = false;
                st.snapshot.playing = false;
                st.snapshot.error = Some(e);
            }
            return;
        }
    };

    let mut video_dec = H264VideoDecoder::new();
    let mut audio_sink = AudioSink::try_open(OUTPUT_SAMPLE_RATE, OUTPUT_CHANNELS);
    let mut playing = autoplay;
    let mut volume = 1.0f32;
    let mut muted = false;
    let mut rate = 1.0f32;
    let mut cur_time = 0.0f64;
    let mut sample_idx = 0usize;
    let mut frame_queue: VecDeque<Arc<VideoFrame>> = VecDeque::new();
    let mut pcm_chunk: Vec<i16> = Vec::with_capacity(8192);
    let mut pcm_backlog: VecDeque<i16> = VecDeque::with_capacity(16384);
    let mut audio_eof = loaded.audio_decoder.is_none();

    if let Some(ref vt) = loaded.video_track {
        if let Ok(mut st) = shared.lock() {
            st.snapshot.video_width = vt.width;
            st.snapshot.video_height = vt.height;
        }
        while sample_idx < vt.samples.len() && frame_queue.is_empty() {
            let s = &vt.samples[sample_idx];
            sample_idx += 1;
            if let Some(frame) = video_dec.decode_mp4_sample(
                &loaded.video_bytes,
                s,
                vt.nal_len_size,
                &vt.sps_pps_annex_b,
            ) {
                let arc = Arc::new(frame);
                if let Ok(mut st) = shared.lock() {
                    st.snapshot.video_width = arc.width;
                    st.snapshot.video_height = arc.height;
                    st.snapshot.frame_seq += 1;
                    st.current_frame = Some(Arc::clone(&arc));
                }
                frame_queue.push_back(arc);
                break;
            }
        }
    }

    if let Ok(mut st) = shared.lock() {
        st.snapshot.loading = false;
        st.snapshot.duration = loaded.duration_sec;
        st.snapshot.buffered_sec = loaded.duration_sec;
        st.snapshot.has_audio = loaded.audio_decoder.is_some();
        st.snapshot.playing = playing;
    }

    let mut last_wall = Instant::now();
    let mut audio_sent_samples: u64 = 0;
    let mut audio_base_time: f64 = 0.0;

    while !stop_flag.load(Ordering::Relaxed) {
        let mut pending_cmds = Vec::new();
        if let Ok(mut st) = shared.lock() {
            while let Some(c) = st.cmds.pop_front() {
                pending_cmds.push(c);
            }
        }
        for cmd in pending_cmds {
            match cmd {
                PlayerCmd::Stop => return,
                PlayerCmd::Play => {
                    if cur_time >= loaded.duration_sec && loaded.duration_sec > 0.1 {
                        cur_time = 0.0;
                        sample_idx = 0;
                        video_dec.reset();
                        frame_queue.clear();
                        pcm_backlog.clear();
                        if let Some(ref mut ad) = loaded.audio_decoder {
                            ad.seek_to(0.0);
                            audio_eof = false;
                        }
                        if let Some(ref mut sink) = audio_sink {
                            sink.flush();
                        }
                        audio_sent_samples = 0;
                        audio_base_time = 0.0;
                    }
                    playing = true;
                    last_wall = Instant::now();
                }
                PlayerCmd::Pause => {
                    playing = false;
                    if let Some(ref mut sink) = audio_sink {
                        sink.flush();
                    }
                    pcm_backlog.clear();
                    audio_base_time = cur_time;
                    audio_sent_samples = 0;
                }
                PlayerCmd::SetVolume(v) => {
                    volume = v;
                    if v > 0.0 {
                        muted = false;
                    }
                }
                PlayerCmd::SetMuted(m) => {
                    muted = m;
                }
                PlayerCmd::SetRate(r) => {
                    rate = r;
                }
                PlayerCmd::Seek(target) => {
                    cur_time = target.clamp(0.0, loaded.duration_sec.max(0.0));
                    video_dec.reset();
                    frame_queue.clear();
                    pcm_backlog.clear();
                    if let Some(ref mut sink) = audio_sink {
                        sink.flush();
                    }
                    audio_base_time = cur_time;
                    audio_sent_samples = 0;
                    if let Some(ref mut ad) = loaded.audio_decoder {
                        ad.seek_to(cur_time);
                        audio_eof = false;
                    }
                    if let Some(ref vt) = loaded.video_track {
                        sample_idx = find_keyframe_index(vt, cur_time);
                        let mut preview: Option<Arc<VideoFrame>> = None;
                        while sample_idx < vt.samples.len() {
                            let s = &vt.samples[sample_idx];
                            sample_idx += 1;
                            if let Some(frame) = video_dec.decode_mp4_sample(
                                &loaded.video_bytes,
                                s,
                                vt.nal_len_size,
                                &vt.sps_pps_annex_b,
                            ) {
                                let pts = frame.pts_sec;
                                let arc = Arc::new(frame);
                                preview = Some(Arc::clone(&arc));
                                if pts >= cur_time - 0.04 {
                                    frame_queue.push_back(arc);
                                    break;
                                }
                            }
                        }
                        if let Some(arc) = preview {
                            if let Ok(mut st) = shared.lock() {
                                st.snapshot.frame_seq += 1;
                                st.current_frame = Some(arc);
                            }
                        }
                    }
                    last_wall = Instant::now();
                }
                PlayerCmd::SwitchSources {
                    video_url,
                    audio_url,
                    resume_time,
                } => {
                    if let Some(ref mut sink) = audio_sink {
                        sink.flush();
                    }
                    pcm_backlog.clear();
                    if let Ok(new_loaded) =
                        load_media_sources(&client, &video_url, audio_url.as_deref())
                    {
                        loaded = new_loaded;
                        video_dec.reset();
                        frame_queue.clear();
                        cur_time = resume_time.clamp(0.0, loaded.duration_sec.max(0.0));
                        audio_base_time = cur_time;
                        audio_sent_samples = 0;
                        audio_eof = loaded.audio_decoder.is_none();
                        if let Some(ref mut ad) = loaded.audio_decoder {
                            ad.seek_to(cur_time);
                        }
                        if let Some(ref vt) = loaded.video_track {
                            sample_idx = find_keyframe_index(vt, cur_time);
                        } else {
                            sample_idx = 0;
                        }
                        if let Ok(mut st) = shared.lock() {
                            st.snapshot.loading = false;
                            st.snapshot.duration = loaded.duration_sec;
                            st.snapshot.buffered_sec = loaded.duration_sec;
                            st.snapshot.has_audio = loaded.audio_decoder.is_some();
                        }
                    } else if let Ok(mut st) = shared.lock() {
                        st.snapshot.loading = false;
                    }
                    last_wall = Instant::now();
                }
            }
        }

        if !playing {
            thread::sleep(Duration::from_millis(16));
            last_wall = Instant::now();
            continue;
        }

        let now = Instant::now();
        let dt = now.duration_since(last_wall).as_secs_f64().min(0.1);
        last_wall = now;
        cur_time += dt * (rate as f64);

        if let (Some(ref mut ad), Some(ref mut sink)) =
            (&mut loaded.audio_decoder, &mut audio_sink)
        {
            if !audio_eof {
                let target_samples = ((cur_time - audio_base_time + 0.12).max(0.0)
                    * (OUTPUT_SAMPLE_RATE as f64)) as u64;
                while audio_sent_samples < target_samples && !audio_eof {
                    while pcm_backlog.len() < 4096 {
                        pcm_chunk.clear();
                        match ad.decode_next_packet_stereo_i16(volume, muted, &mut pcm_chunk) {
                            Ok(true) => pcm_backlog.extend(pcm_chunk.iter().copied()),
                            Ok(false) | Err(_) => {
                                audio_eof = true;
                                break;
                            }
                        }
                    }
                    if pcm_backlog.is_empty() {
                        break;
                    }
                    let take = pcm_backlog.len().min(4096);
                    let mut send_buf = Vec::with_capacity(take);
                    for _ in 0..take {
                        if let Some(s) = pcm_backlog.pop_front() {
                            send_buf.push(s);
                        }
                    }
                    let frames_sent = (send_buf.len() / 2) as u64;
                    sink.write_pcm(&send_buf);
                    audio_sent_samples += frames_sent;
                }
            }
        }

        if let Some(ref vt) = loaded.video_track {
            let mut decoded_this_tick = 0usize;
            while frame_queue.len() < 8 && sample_idx < vt.samples.len() && decoded_this_tick < 4 {
                let s = &vt.samples[sample_idx];
                sample_idx += 1;
                decoded_this_tick += 1;
                if let Some(frame) = video_dec.decode_mp4_sample(
                    &loaded.video_bytes,
                    s,
                    vt.nal_len_size,
                    &vt.sps_pps_annex_b,
                ) {
                    let arc = Arc::new(frame);
                    let insert_pos = frame_queue
                        .iter()
                        .position(|f| f.pts_sec > arc.pts_sec)
                        .unwrap_or(frame_queue.len());
                    frame_queue.insert(insert_pos, arc);
                }
            }

            let mut latest_due: Option<Arc<VideoFrame>> = None;
            while let Some(front) = frame_queue.front() {
                if front.pts_sec <= cur_time + 0.012 {
                    latest_due = frame_queue.pop_front();
                } else {
                    break;
                }
            }
            if let Some(arc) = latest_due {
                if let Ok(mut st) = shared.lock() {
                    st.snapshot.video_width = arc.width;
                    st.snapshot.video_height = arc.height;
                    st.snapshot.frame_seq += 1;
                    st.current_frame = Some(arc);
                }
            }
        }

        let video_done = loaded
            .video_track
            .as_ref()
            .map_or(true, |vt| sample_idx >= vt.samples.len() && frame_queue.is_empty());
        let finished = if loaded.duration_sec > 0.0 {
            cur_time >= loaded.duration_sec && video_done && audio_eof
        } else {
            video_done && audio_eof
        };

        if let Ok(mut st) = shared.lock() {
            st.snapshot.current_time = cur_time.min(loaded.duration_sec.max(cur_time));
            if finished {
                playing = false;
                st.snapshot.playing = false;
                st.snapshot.ended = true;
            }
        }

        thread::sleep(Duration::from_millis(8));
    }
}
