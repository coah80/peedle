use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use fontdue::{Font, FontSettings, Metrics};

use crate::framebuffer::{Framebuffer, RgbaColor};

static SYSTEM_FONT_CACHE: OnceLock<FontSystem> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontFaceKey {
    SansRegular,
    SansBold,
    SansItalic,
    SerifRegular,
    SerifBold,
    MonoRegular,
    MonoBold,
    Custom(usize),
    Fallback(usize),
}

#[derive(Clone)]
pub struct FontSystem {
    faces: HashMap<FontFaceKey, Arc<Font>>,
    custom_names: Vec<(String, FontFaceKey)>,
    fallback_keys: Vec<FontFaceKey>,
    glyph_cache: Arc<Mutex<HashMap<(FontFaceKey, char, u32), (Metrics, Vec<u8>)>>>,
}

impl Default for FontSystem {
    fn default() -> Self {
        Self::load_system_fonts()
    }
}

impl FontSystem {
    pub fn load_system_fonts() -> Self {
        let base = SYSTEM_FONT_CACHE.get_or_init(Self::load_system_fonts_uncached);
        Self {
            faces: base.faces.clone(),
            custom_names: Vec::new(),
            fallback_keys: base.fallback_keys.clone(),
            glyph_cache: Arc::clone(&base.glyph_cache),
        }
    }

    fn load_system_fonts_uncached() -> Self {
        let mut faces = HashMap::new();
        let candidates: &[(FontFaceKey, &[&str])] = &[
            (
                FontFaceKey::SansRegular,
                &[
                    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                    "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
                    "/System/Library/Fonts/Supplemental/Arial.ttf",
                    "/System/Library/Fonts/SFNS.ttf",
                    "/System/Library/Fonts/Helvetica.ttc",
                    "C:\\Windows\\Fonts\\segoeui.ttf",
                    "C:\\Windows\\Fonts\\arial.ttf",
                ],
            ),
            (
                FontFaceKey::SansBold,
                &[
                    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
                    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
                    "/usr/share/fonts/truetype/freefont/FreeSansBold.ttf",
                    "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
                    "/System/Library/Fonts/Supplemental/Verdana Bold.ttf",
                    "C:\\Windows\\Fonts\\segoeuib.ttf",
                    "C:\\Windows\\Fonts\\arialbd.ttf",
                ],
            ),
            (
                FontFaceKey::SansItalic,
                &[
                    "/usr/share/fonts/truetype/liberation/LiberationSans-Italic.ttf",
                    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Oblique.ttf",
                    "/usr/share/fonts/truetype/freefont/FreeSansOblique.ttf",
                    "/System/Library/Fonts/Supplemental/Arial Italic.ttf",
                    "/System/Library/Fonts/Supplemental/Verdana Italic.ttf",
                    "C:\\Windows\\Fonts\\segoeuii.ttf",
                    "C:\\Windows\\Fonts\\ariali.ttf",
                ],
            ),
            (
                FontFaceKey::SerifRegular,
                &[
                    "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
                    "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
                    "/usr/share/fonts/truetype/freefont/FreeSerif.ttf",
                    "/System/Library/Fonts/Supplemental/Georgia.ttf",
                    "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
                    "C:\\Windows\\Fonts\\georgia.ttf",
                    "C:\\Windows\\Fonts\\times.ttf",
                ],
            ),
            (
                FontFaceKey::SerifBold,
                &[
                    "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf",
                    "/usr/share/fonts/truetype/liberation/LiberationSerif-Bold.ttf",
                    "/usr/share/fonts/truetype/freefont/FreeSerifBold.ttf",
                    "/System/Library/Fonts/Supplemental/Georgia Bold.ttf",
                    "/System/Library/Fonts/Supplemental/Times New Roman Bold.ttf",
                    "C:\\Windows\\Fonts\\georgiab.ttf",
                    "C:\\Windows\\Fonts\\timesbd.ttf",
                ],
            ),
            (
                FontFaceKey::MonoRegular,
                &[
                    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
                    "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
                    "/usr/share/fonts/truetype/freefont/FreeMono.ttf",
                    "/System/Library/Fonts/Monaco.ttf",
                    "/System/Library/Fonts/Supplemental/Courier New.ttf",
                    "/System/Library/Fonts/Supplemental/Andale Mono.ttf",
                    "C:\\Windows\\Fonts\\consola.ttf",
                    "C:\\Windows\\Fonts\\cour.ttf",
                ],
            ),
            (
                FontFaceKey::MonoBold,
                &[
                    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
                    "/usr/share/fonts/truetype/liberation/LiberationMono-Bold.ttf",
                    "/System/Library/Fonts/Supplemental/Courier New Bold.ttf",
                    "C:\\Windows\\Fonts\\consolab.ttf",
                    "C:\\Windows\\Fonts\\courbd.ttf",
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

        let fallback_paths = [
            "/usr/share/fonts/opentype/ipafont-gothic/ipagp.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/unifont/unifont.otf",
            "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/Apple Symbols.ttf",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "C:\\Windows\\Fonts\\seguisym.ttf",
            "C:\\Windows\\Fonts\\msyh.ttc",
            "C:\\Windows\\Fonts\\msgothic.ttc",
        ];
        let mut fallback_keys = Vec::new();
        for &path in &fallback_paths {
            if Path::new(path).exists() {
                if let Ok(bytes) = std::fs::read(path) {
                    if let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) {
                        let fk = FontFaceKey::Fallback(fallback_keys.len());
                        faces.insert(fk, Arc::new(font));
                        fallback_keys.push(fk);
                    }
                }
            }
        }

        Self {
            faces,
            custom_names: Vec::new(),
            fallback_keys,
            glyph_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn register_web_font(&mut self, family: &str, bytes: &[u8]) -> bool {
        let norm = family.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
        if norm.is_empty() {
            return false;
        }
        if self.custom_names.iter().any(|(n, _)| n == &norm) {
            return true;
        }
        if let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) {
            let key = FontFaceKey::Custom(self.custom_names.len());
            self.faces.insert(key, Arc::new(font));
            self.custom_names.push((norm, key));
            return true;
        }
        false
    }

    pub fn select_face(&self, families: &[String], weight: u16, italic: bool) -> FontFaceKey {
        let bold = weight >= 600;
        for fam in families {
            let low = fam.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
            if let Some((_, key)) = self.custom_names.iter().find(|(n, _)| n == &low) {
                return *key;
            }
            if low.contains("mono")
                || low.contains("courier")
                || low.contains("consolas")
                || low.contains("menlo")
                || low.contains("monaco")
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
                || low.contains("-apple-system")
                || low.contains("blinkmacsystemfont")
                || low.contains("sf pro")
                || low.contains("avenir")
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

    fn resolve_glyph_font(&self, key: FontFaceKey, c: char) -> Option<(FontFaceKey, &Arc<Font>)> {
        let primary = self.font(key)?;
        if c.is_ascii() || primary.lookup_glyph_index(c) != 0 {
            return Some((key, primary));
        }
        for &fk in &self.fallback_keys {
            if let Some(fb) = self.faces.get(&fk) {
                if fb.lookup_glyph_index(c) != 0 {
                    return Some((fk, fb));
                }
            }
        }
        Some((key, primary))
    }

    pub fn measure_text(&self, text: &str, key: FontFaceKey, px: f32) -> f32 {
        if self.font(key).is_none() {
            return text.chars().count() as f32 * px * 0.55;
        }
        let mut width = 0.0f32;
        for c in text.chars() {
            if let Some((_, f)) = self.resolve_glyph_font(key, c) {
                width += f.metrics(c, px).advance_width;
            }
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
        if self.font(key).is_none() {
            return x;
        }
        let px_key = (px.max(4.0) * 4.0).round() as u32;
        let actual_px = px_key as f32 / 4.0;
        let mut pen_x = x;

        for c in text.chars() {
            let Some((use_key, use_font)) = self.resolve_glyph_font(key, c) else {
                continue;
            };
            let (metrics, bitmap) = {
                let mut cache = self.glyph_cache.lock().unwrap();
                if let Some(hit) = cache.get(&(use_key, c, px_key)) {
                    hit.clone()
                } else {
                    let entry = use_font.rasterize(c, actual_px);
                    cache.insert((use_key, c, px_key), entry.clone());
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
