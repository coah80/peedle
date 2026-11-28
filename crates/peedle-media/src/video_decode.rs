use rusty_h264_decoder::Decoder;

#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub pts_sec: f64,
    pub rgba: Vec<u8>,
}

pub struct H264VideoDecoder {
    decoder: Decoder,
    nal_buf: Vec<u8>,
    sent_sps_pps: bool,
}

impl Default for H264VideoDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl H264VideoDecoder {
    pub fn new() -> Self {
        Self {
            decoder: Decoder::new(),
            nal_buf: Vec::with_capacity(65536),
            sent_sps_pps: false,
        }
    }

    pub fn reset(&mut self) {
        self.decoder = Decoder::new();
        self.sent_sps_pps = false;
    }

    pub fn decode_mp4_sample(
        &mut self,
        file_bytes: &[u8],
        sample: &crate::mp4_demux::Mp4Sample,
        nal_len_size: usize,
        sps_pps_annex_b: &[u8],
    ) -> Option<VideoFrame> {
        let send_headers = !self.sent_sps_pps || sample.is_keyframe;
        if !crate::mp4_demux::sample_to_annex_b(
            file_bytes,
            sample,
            nal_len_size,
            sps_pps_annex_b,
            send_headers,
            &mut self.nal_buf,
        ) {
            return None;
        }
        if send_headers {
            self.sent_sps_pps = true;
        }
        match self.decoder.decode(&self.nal_buf) {
            Ok(Some(yuv)) => {
                let width = yuv.width as u32;
                let height = yuv.height as u32;
                let rgba = yuv420_to_rgba(yuv.width, yuv.height, &yuv.y, &yuv.u, &yuv.v);
                self.decoder.recycle_frame(yuv);
                Some(VideoFrame {
                    width,
                    height,
                    pts_sec: sample.pts_sec,
                    rgba,
                })
            }
            _ => None,
        }
    }
}

pub fn yuv420_to_rgba(w: usize, h: usize, y_plane: &[u8], u_plane: &[u8], v_plane: &[u8]) -> Vec<u8> {
    let cw = w / 2;
    let mut out = vec![255u8; w.saturating_mul(h).saturating_mul(4)];
    if w == 0 || h == 0 || y_plane.len() < w * h {
        return out;
    }
    let ch = h / 2;
    let has_chroma = cw > 0 && ch > 0 && u_plane.len() >= cw * ch && v_plane.len() >= cw * ch;

    for y in 0..h {
        let y_row = y * w;
        let c_row = (y >> 1) * cw;
        let dst_row = y * w * 4;
        for x in 0..w {
            let yv = y_plane[y_row + x] as i32;
            let (u, v) = if has_chroma {
                let ci = c_row + (x >> 1);
                (u_plane[ci] as i32 - 128, v_plane[ci] as i32 - 128)
            } else {
                (0, 0)
            };
            let c = (yv - 16).max(0);
            let r = ((298 * c + 409 * v + 128) >> 8).clamp(0, 255) as u8;
            let g = ((298 * c - 100 * u - 208 * v + 128) >> 8).clamp(0, 255) as u8;
            let b = ((298 * c + 516 * u + 128) >> 8).clamp(0, 255) as u8;
            let di = dst_row + x * 4;
            out[di] = r;
            out[di + 1] = g;
            out[di + 2] = b;
            out[di + 3] = 255;
        }
    }
    out
}
