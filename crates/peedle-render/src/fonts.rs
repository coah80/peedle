use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use fontdue::{Font, FontSettings, Metrics};

use crate::framebuffer::{Framebuffer, RgbaColor};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontFaceKey {
    SansRegular,
    SansBold,
    SansItalic,
    SerifRegular,
    SerifBold,
    MonoRegular,
    MonoBold,
}

#[derive(Clone)]
pub struct FontSystem {
    faces: HashMap<FontFaceKey, Arc<Font>>,
    glyph_cache: Arc<Mutex<HashMap<(FontFaceKey, char, u32), (Metrics, Vec<u8>)>>>,
}

impl Default for FontSystem {
    fn default() -> Self {
        Self::load_system_fonts()
    }
}

impl FontSystem {
    pub fn load_system_fonts() -> Self {
        let mut faces = HashMap::new();
        let candidates: &[(FontFaceKey, &[&str])] = &[
            (
                FontFaceKey::SansRegular,
                &[
                    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                    "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
                    "/System/Library/Fonts/Helvetica.ttc",
                    "C:\\Windows\\Fonts\\arial.ttf",
                ],
            ),
            (
                FontFaceKey::SansBold,
                &[
                    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
                    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
                    "/usr/share/fonts/truetype/freefont/FreeSansBold.ttf",
                ],
            ),
            (
                FontFaceKey::SansItalic,
                &[
                    "/usr/share/fonts/truetype/liberation/LiberationSans-Italic.ttf",
                    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Oblique.ttf",
                    "/usr/share/fonts/truetype/freefont/FreeSansOblique.ttf",
                ],
            ),
            (
                FontFaceKey::SerifRegular,
                &[
                    "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
                    "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
                    "/usr/share/fonts/truetype/freefont/FreeSerif.ttf",
                ],
            ),
            (
                FontFaceKey::SerifBold,
                &[
                    "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf",
                    "/usr/share/fonts/truetype/liberation/LiberationSerif-Bold.ttf",
                    "/usr/share/fonts/truetype/freefont/FreeSerifBold.ttf",
                ],
            ),
            (
                FontFaceKey::MonoRegular,
                &[
                    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
                    "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
                    "/usr/share/fonts/truetype/freefont/FreeMono.ttf",
                ],
            ),
            (
                FontFaceKey::MonoBold,
                &[
                    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
                    "/usr/share/fonts/truetype/liberation/LiberationMono-Bold.ttf",
                ],
            ),
        ];

        for &(key, paths) in candidates {
            for &path in paths {
                if Path::new(path).exists() {
                    if let Ok(bytes) = std::fs::read(path) {
                        if let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) {
                            faces.insert(key, Arc::new(font));
                            break;
                        }
                    }
                }
            }
        }

        Self {
            faces,
            glyph_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn select_face(&self, families: &[String], weight: u16, italic: bool) -> FontFaceKey {
        let bold = weight >= 600;
        for fam in families {
            let low = fam.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
            if low.contains("mono")
                || low.contains("courier")
                || low.contains("consolas")
                || low.contains("menlo")
                || low.contains("fira code")
            {
                return if bold && self.faces.contains_key(&FontFaceKey::MonoBold) {
                    FontFaceKey::MonoBold
                } else {
                    FontFaceKey::MonoRegular
                };
            }
            if low == "serif"
                || low.contains("times")
                || low.contains("georgia")
                || low.contains("garamond")
            {
                return if bold && self.faces.contains_key(&FontFaceKey::SerifBold) {
                    FontFaceKey::SerifBold
                } else {
                    FontFaceKey::SerifRegular
                };
            }
            if low == "sans-serif"
                || low.contains("arial")
                || low.contains("helvetica")
                || low.contains("inter")
                || low.contains("roboto")
                || low.contains("system-ui")
                || low.contains("segoe")
            {
                if bold && self.faces.contains_key(&FontFaceKey::SansBold) {
                    return FontFaceKey::SansBold;
                }
                if italic && self.faces.contains_key(&FontFaceKey::SansItalic) {
                    return FontFaceKey::SansItalic;
                }
                return FontFaceKey::SansRegular;
            }
        }
        if bold && self.faces.contains_key(&FontFaceKey::SansBold) {
            FontFaceKey::SansBold
        } else if italic && self.faces.contains_key(&FontFaceKey::SansItalic) {
            FontFaceKey::SansItalic
        } else {
            FontFaceKey::SansRegular
        }
    }

    pub fn font(&self, key: FontFaceKey) -> Option<&Arc<Font>> {
        self.faces
            .get(&key)
            .or_else(|| self.faces.get(&FontFaceKey::SansRegular))
    }

    pub fn measure_text(&self, text: &str, key: FontFaceKey, px: f32) -> f32 {
        let Some(font) = self.font(key) else {
            return text.chars().count() as f32 * px * 0.55;
        };
        let mut width = 0.0f32;
        for c in text.chars() {
            width += font.metrics(c, px).advance_width;
        }
        width
    }

    pub fn line_ascent(&self, key: FontFaceKey, px: f32) -> f32 {
        if let Some(font) = self.font(key) {
            if let Some(lm) = font.horizontal_line_metrics(px) {
                return lm.ascent;
            }
        }
        px * 0.8
    }

    pub fn draw_text(
        &self,
        fb: &mut Framebuffer,
        text: &str,
        x: f32,
        baseline_y: f32,
        key: FontFaceKey,
        px: f32,
        color: RgbaColor,
    ) -> f32 {
        let Some(font) = self.font(key) else {
            return x;
        };
        let px_key = (px.max(4.0) * 4.0).round() as u32;
        let actual_px = px_key as f32 / 4.0;
        let mut pen_x = x;

        for c in text.chars() {
            let (metrics, bitmap) = {
                let mut cache = self.glyph_cache.lock().unwrap();
                if let Some(hit) = cache.get(&(key, c, px_key)) {
                    hit.clone()
                } else {
                    let entry = font.rasterize(c, actual_px);
                    cache.insert((key, c, px_key), entry.clone());
                    entry
                }
            };
            if metrics.width > 0 && metrics.height > 0 {
                let gx = (pen_x + metrics.xmin as f32).round() as i32;
                let gy = (baseline_y - metrics.ymin as f32 - metrics.height as f32).round() as i32;
                fb.blit_alpha_mask(gx, gy, metrics.width, metrics.height, &bitmap, color);
            }
            pen_x += metrics.advance_width;
        }
        pen_x
    }
}
