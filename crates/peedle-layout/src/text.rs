use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use fontdue::{Font, FontSettings};

#[derive(Clone)]
pub struct TextMeasurer {
    sans: Option<Arc<Font>>,
    sans_bold: Option<Arc<Font>>,
    serif: Option<Arc<Font>>,
    mono: Option<Arc<Font>>,
    width_cache: HashMap<(u8, u32, char), f32>,
}

impl Default for TextMeasurer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextMeasurer {
    pub fn new() -> Self {
        Self {
            sans: load_first(&[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
            ]),
            sans_bold: load_first(&[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
            ]),
            serif: load_first(&[
                "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
            ]),
            mono: load_first(&[
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
            ]),
            width_cache: HashMap::new(),
        }
    }

    fn pick_kind(&self, families: &[String], weight: u16) -> u8 {
        for fam in families {
            let low = fam.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
            if low.contains("mono") || low.contains("courier") || low.contains("consolas") {
                return 3;
            }
            if low == "serif" || low.contains("times") || low.contains("georgia") {
                return 2;
            }
            if low == "sans-serif" || low.contains("arial") || low.contains("helvetica") || low.contains("inter") {
                return if weight >= 600 { 1 } else { 0 };
            }
        }
        if weight >= 600 { 1 } else { 0 }
    }

    fn font_for(&self, kind: u8) -> Option<&Arc<Font>> {
        match kind {
            1 => self.sans_bold.as_ref().or(self.sans.as_ref()),
            2 => self.serif.as_ref().or(self.sans.as_ref()),
            3 => self.mono.as_ref().or(self.sans.as_ref()),
            _ => self.sans.as_ref(),
        }
    }

    pub fn measure_str(&mut self, text: &str, families: &[String], font_size: f32, weight: u16) -> f32 {
        let kind = self.pick_kind(families, weight);
        let px_key = (font_size.max(4.0) * 4.0).round() as u32;
        let px = px_key as f32 / 4.0;
        let mut total = 0.0f32;
        for c in text.chars() {
            if let Some(&adv) = self.width_cache.get(&(kind, px_key, c)) {
                total += adv;
            } else {
                let adv = self
                    .font_for(kind)
                    .map(|f| f.metrics(c, px).advance_width)
                    .unwrap_or(px * 0.55);
                self.width_cache.insert((kind, px_key, c), adv);
                total += adv;
            }
        }
        total
    }
}

fn load_first(paths: &[&str]) -> Option<Arc<Font>> {
    for &p in paths {
        if Path::new(p).exists() {
            if let Ok(bytes) = std::fs::read(p) {
                if let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) {
                    return Some(Arc::new(font));
                }
            }
        }
    }
    None
}
