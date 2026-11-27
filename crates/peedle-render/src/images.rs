use std::collections::HashMap;

use peedle_html::parse_html;

use crate::fonts::FontSystem;
use crate::framebuffer::Framebuffer;
use crate::svg::render_svg_node;

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
        let w = el
            .get_attr("width")
            .and_then(|v| v.trim_end_matches("px").parse::<f32>().ok())
            .unwrap_or(64.0)
            .clamp(1.0, 1024.0) as u32;
        let h = el
            .get_attr("height")
            .and_then(|v| v.trim_end_matches("px").parse::<f32>().ok())
            .unwrap_or(64.0)
            .clamp(1.0, 1024.0) as u32;
        let mut fb = Framebuffer::new(w, h);
        let fonts = FontSystem::load_system_fonts();
        render_svg_node(&doc, svg_id, &mut fb, &fonts, 0.0, 0.0, w as f32, h as f32);
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
