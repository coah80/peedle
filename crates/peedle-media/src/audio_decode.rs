use std::io::Cursor;

use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::common::Tier;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Time;
use symphonia_opus::OpusDecoder;

pub const OUTPUT_SAMPLE_RATE: u32 = 48_000;
pub const OUTPUT_CHANNELS: u16 = 2;

pub struct StreamingAudioDecoder {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    src_rate: u32,
    src_channels: usize,
    duration_sec: f64,
    time_base_num: u64,
    time_base_den: u64,
    cur_pts_sec: f64,
    raw_f32: Vec<f32>,
    resample_phase: f64,
    last_l: f32,
    last_r: f32,
}

impl StreamingAudioDecoder {
    pub fn open(bytes: Vec<u8>, hint_ext: Option<&str>) -> Result<Self, String> {
        let mut registry = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut registry);
        registry.register_audio_decoder_at_tier::<OpusDecoder>(Tier::Fallback);

        let mut hint = Hint::new();
        if let Some(ext) = hint_ext {
            hint.with_extension(ext);
        }

        let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
        let reader = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| format!("audio probe failed: {}", e))?;

        let mut chosen = None;
        for track in reader.tracks() {
            if let Some(CodecParameters::Audio(ref ap)) = track.codec_params {
                chosen = Some((track.id, ap.clone(), track.time_base, track.num_frames));
                break;
            }
        }
        let (track_id, audio_params, time_base, num_frames) =
            chosen.ok_or_else(|| "no audio track found".to_string())?;

        let src_rate = audio_params.sample_rate.unwrap_or(OUTPUT_SAMPLE_RATE).max(8000);
        let src_channels = audio_params
            .channels
            .as_ref()
            .map(|c| c.count())
            .unwrap_or(2)
            .max(1);
        let (tb_num, tb_den) = time_base
            .map(|tb| (tb.numer.get() as u64, tb.denom.get() as u64))
            .unwrap_or((1, src_rate as u64));
        let duration_sec = num_frames
            .map(|nf| (nf as f64) * (tb_num as f64) / (tb_den.max(1) as f64))
            .unwrap_or(0.0);

        let decoder = registry
            .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
            .map_err(|e| format!("audio decoder init failed: {}", e))?;

        Ok(Self {
            reader,
            decoder,
            track_id,
            src_rate,
            src_channels,
            duration_sec,
            time_base_num: tb_num,
            time_base_den: tb_den.max(1),
            cur_pts_sec: 0.0,
            raw_f32: Vec::with_capacity(8192),
            resample_phase: 0.0,
            last_l: 0.0,
            last_r: 0.0,
        })
    }

    pub fn duration_sec(&self) -> f64 {
        self.duration_sec
    }

    pub fn current_pts_sec(&self) -> f64 {
        self.cur_pts_sec
    }

    pub fn seek_to(&mut self, secs: f64) {
        let clamped = secs.max(0.0);
        if let Some(time) = Time::try_from_secs_f64(clamped) {
            let _ = self.reader.seek(
                SeekMode::Coarse,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            );
        }
        self.decoder.reset();
        self.cur_pts_sec = clamped;
        self.resample_phase = 0.0;
        self.last_l = 0.0;
        self.last_r = 0.0;
    }

    pub fn decode_next_packet_stereo_i16(
        &mut self,
        volume: f32,
        muted: bool,
        out: &mut Vec<i16>,
    ) -> Result<bool, String> {
        loop {
            let pkt = match self.reader.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(false),
                Err(SymError::IoError(ref e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(false);
                }
                Err(SymError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => return Err(format!("packet read error: {}", e)),
            };
            if pkt.track_id != self.track_id {
                continue;
            }
            let ts = pkt.pts.get().max(0) as u64;
            self.cur_pts_sec =
                (ts as f64) * (self.time_base_num as f64) / (self.time_base_den as f64);

            let decoded = match self.decoder.decode(&pkt) {
                Ok(buf) => buf,
                Err(SymError::DecodeError(_)) => continue,
                Err(e) => return Err(format!("audio decode error: {}", e)),
            };
            let frames = decoded.frames();
            if frames == 0 {
                continue;
            }
            let spec = decoded.spec();
            let rate = spec.rate().max(8000);
            let ch = spec.channels().count().max(1);
            self.src_rate = rate;
            self.src_channels = ch;

            self.raw_f32.clear();
            decoded.copy_to_vec_interleaved::<f32>(&mut self.raw_f32);

            let gain = if muted { 0.0 } else { volume.clamp(0.0, 2.0) };
            let step = (rate as f64) / (OUTPUT_SAMPLE_RATE as f64);

            if rate == OUTPUT_SAMPLE_RATE {
                for f in 0..frames {
                    let base = f * ch;
                    let l = self.raw_f32.get(base).copied().unwrap_or(0.0) * gain;
                    let r = if ch >= 2 {
                        self.raw_f32.get(base + 1).copied().unwrap_or(l) * gain
                    } else {
                        l
                    };
                    out.push((l.clamp(-1.0, 1.0) * 32767.0) as i16);
                    out.push((r.clamp(-1.0, 1.0) * 32767.0) as i16);
                }
            } else {
                let mut pos = self.resample_phase;
                let max_pos = frames as f64;
                while pos < max_pos {
                    let idx = pos.floor() as usize;
                    let frac = (pos - idx as f64) as f32;
                    let (l0, r0) = if idx == 0 {
                        (self.last_l, self.last_r)
                    } else {
                        let b = (idx - 1) * ch;
                        let l = self.raw_f32.get(b).copied().unwrap_or(0.0);
                        let r = if ch >= 2 {
                            self.raw_f32.get(b + 1).copied().unwrap_or(l)
                        } else {
                            l
                        };
                        (l, r)
                    };
                    let b1 = idx.min(frames - 1) * ch;
                    let l1 = self.raw_f32.get(b1).copied().unwrap_or(0.0);
                    let r1 = if ch >= 2 {
                        self.raw_f32.get(b1 + 1).copied().unwrap_or(l1)
                    } else {
                        l1
                    };
                    let l = (l0 + (l1 - l0) * frac) * gain;
                    let r = (r0 + (r1 - r0) * frac) * gain;
                    out.push((l.clamp(-1.0, 1.0) * 32767.0) as i16);
                    out.push((r.clamp(-1.0, 1.0) * 32767.0) as i16);
                    pos += step;
                }
                self.resample_phase = pos - max_pos;
                let last_b = (frames - 1) * ch;
                self.last_l = self.raw_f32.get(last_b).copied().unwrap_or(0.0);
                self.last_r = if ch >= 2 {
                    self.raw_f32.get(last_b + 1).copied().unwrap_or(self.last_l)
                } else {
                    self.last_l
                };
            }
            return Ok(true);
        }
    }
}
