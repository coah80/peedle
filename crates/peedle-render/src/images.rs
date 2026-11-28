use std::collections::HashMap;
use std::sync::OnceLock;

use peedle_html::parse_html;

use crate::fonts::FontSystem;
use crate::framebuffer::Framebuffer;
use crate::svg::render_svg_node;

fn shared_svg_fonts() -> &'static FontSystem {
    static FONTS: OnceLock<FontSystem> = OnceLock::new();
    FONTS.get_or_init(FontSystem::load_system_fonts)
}

#[derive(Clone, Debug)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl DecodedImage {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if let Ok(s) = std::str::from_utf8(bytes) {
            let trimmed = s.trim_start();
            if trimmed.starts_with("<svg") || trimmed.starts_with("<?xml") && trimmed.contains("<svg") {
                return Self::decode_svg(trimmed);
            }
        }
        let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
        let rgba = img.to_rgba8();
        Ok(Self {
            width: rgba.width(),
            height: rgba.height(),
            rgba: rgba.into_raw(),
        })
    }

    fn decode_svg(src: &str) -> Result<Self, String> {
        let doc = parse_html(src);
        let svgs = doc.get_elements_by_tag_name(doc.root, "svg");
        let svg_id = *svgs.first().ok_or_else(|| "no svg element".to_string())?;
        let el = doc.element(svg_id).ok_or_else(|| "missing svg".to_string())?;
        let vb_dims = el
            .get_attr("viewBox")
            .or_else(|| el.get_attr("viewbox"))
            .and_then(|vb| {
                let nums: Vec<f32> = vb
                    .split(|c: char| c.is_ascii_whitespace() || c == ',')
                    .filter(|s| !s.is_empty())
                    .filter_map(|s| s.parse::<f32>().ok())
                    .collect();
                if nums.len() >= 4 && nums[2] > 0.0 && nums[3] > 0.0 {
                    Some((nums[2], nums[3]))
                } else {
                    None
                }
            });
        let raw_w = el
            .get_attr("width")
            .and_then(|v| v.trim_end_matches("px").parse::<f32>().ok())
            .filter(|&v| v > 0.0);
        let raw_h = el
            .get_attr("height")
            .and_then(|v| v.trim_end_matches("px").parse::<f32>().ok())
            .filter(|&v| v > 0.0);
        let (mut wf, mut hf) = match (raw_w, raw_h, vb_dims) {
            (Some(w), Some(h), _) => (w, h),
            (Some(w), None, Some((vw, vh))) => (w, w * (vh / vw)),
            (None, Some(h), Some((vw, vh))) => (h * (vw / vh), h),
            (None, None, Some((vw, vh))) => (vw, vh),
            (Some(w), None, None) => (w, 64.0),
            (None, Some(h), None) => (64.0, h),
            (None, None, None) => (64.0, 64.0),
        };
        let max_dim = wf.max(hf);
        if max_dim > 1024.0 {
            let sc = 1024.0 / max_dim;
            wf *= sc;
            hf *= sc;
        }
        let w = wf.round().max(1.0) as u32;
        let h = hf.round().max(1.0) as u32;
        let mut fb = Framebuffer::new(w, h);
        render_svg_node(&doc, svg_id, &mut fb, shared_svg_fonts(), 0.0, 0.0, w as f32, h as f32);
        Ok(Self {
            width: w,
            height: h,
            rgba: fb.data,
        })
    }
}

#[derive(Clone, Default)]
pub struct ImageCache {
    entries: HashMap<String, DecodedImage>,
}

impl ImageCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn insert(&mut self, url: String, img: DecodedImage) {
        self.entries.insert(url, img);
    }

    pub fn get(&self, url: &str) -> Option<&DecodedImage> {
        self.entries.get(url)
    }

    pub fn contains(&self, url: &str) -> bool {
        self.entries.contains_key(url)
    }
}
