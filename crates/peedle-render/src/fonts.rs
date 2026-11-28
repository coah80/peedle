use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use fontdue::{Font, FontSettings, Metrics};
use peedle_layout::{extract_gsub_ligatures, FontLigatures, ShapedGlyph};

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
    custom_names: Vec<(String, u16, bool, FontFaceKey)>,
    ligatures: HashMap<FontFaceKey, Arc<FontLigatures>>,
    fallback_keys: Vec<FontFaceKey>,
    glyph_cache: Arc<Mutex<HashMap<(FontFaceKey, char, u32), (Metrics, Vec<u8>)>>>,
}

impl Default for FontSystem {
    fn default() -> Self {
        Self::load_system_fonts()
    }
}

fn is_default_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFFC}'
    )
}

impl FontSystem {
    pub fn load_system_fonts() -> Self {
        let base = SYSTEM_FONT_CACHE.get_or_init(Self::load_system_fonts_uncached);
        Self {
            faces: base.faces.clone(),
            custom_names: Vec::new(),
            ligatures: HashMap::new(),
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
                    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
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
                    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
                    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
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
                    "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
                    "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
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
                    "/usr/share/fonts/truetype/liberation/LiberationSerif-Bold.ttf",
                    "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf",
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
                    "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
                    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
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
                    "/usr/share/fonts/truetype/liberation/LiberationMono-Bold.ttf",
                    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
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
            ligatures: HashMap::new(),
            fallback_keys,
            glyph_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn register_web_font(&mut self, family: &str, bytes: &[u8]) -> Option<Arc<Font>> {
        self.register_web_font_styled(family, 400, false, bytes)
    }

    pub fn register_web_font_weighted(
        &mut self,
        family: &str,
        weight: u16,
        bytes: &[u8],
    ) -> Option<Arc<Font>> {
        self.register_web_font_styled(family, weight, false, bytes)
    }

    pub fn register_web_font_styled(
        &mut self,
        family: &str,
        weight: u16,
        italic: bool,
        bytes: &[u8],
    ) -> Option<Arc<Font>> {
        self.register_web_font_styled_ex(family, weight, italic, bytes)
            .map(|(f, _)| f)
    }

    pub fn register_web_font_styled_ex(
        &mut self,
        family: &str,
        weight: u16,
        italic: bool,
        bytes: &[u8],
    ) -> Option<(Arc<Font>, Option<Arc<FontLigatures>>)> {
        let norm = family.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
        if norm.is_empty() {
            return None;
        }
        let decoded = decode_web_font(bytes);
        let font_bytes = decoded.as_deref().unwrap_or(bytes);
        if let Ok(font) = Font::from_bytes(font_bytes, FontSettings::default()) {
            let ligs = extract_gsub_ligatures(font_bytes);
            let arc = Arc::new(font);
            let key = FontFaceKey::Custom(self.custom_names.len());
            self.faces.insert(key, Arc::clone(&arc));
            if let Some(ref l) = ligs {
                self.ligatures.insert(key, Arc::clone(l));
            }
            self.custom_names.push((norm, weight, italic, key));
            return Some((arc, ligs));
        }
        None
    }

    pub fn select_face(&self, families: &[String], weight: u16, italic: bool) -> FontFaceKey {
        let bold = weight >= 600;
        for fam in families {
            let low = fam.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
            if let Some((_, _, _, key)) = self
                .custom_names
                .iter()
                .filter(|(n, _, _, _)| n == &low)
                .min_by_key(|(_, w, it, _)| {
                    (
                        if *it == italic { 0 } else { 1 },
                        (*w as i32 - weight as i32).abs(),
                    )
                })
            {
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
        if primary.lookup_glyph_index(c) != 0 {
            return Some((key, primary));
        }
        if let FontFaceKey::Custom(_) = key {
            if let Some((fam, weight, italic, _)) =
                self.custom_names.iter().find(|(_, _, _, k)| *k == key)
            {
                let mut best: Option<((i32, i32), FontFaceKey, &Arc<Font>)> = None;
                for (n, w, it, k) in &self.custom_names {
                    if *k != key && n == fam {
                        if let Some(f) = self.faces.get(k) {
                            if f.lookup_glyph_index(c) != 0 {
                                let score = (
                                    if *it == *italic { 0 } else { 1 },
                                    (*w as i32 - *weight as i32).abs(),
                                );
                                if best.map_or(true, |(bs, _, _)| score < bs) {
                                    best = Some((score, *k, f));
                                }
                            }
                        }
                    }
                }
                if let Some((_, bk, bf)) = best {
                    return Some((bk, bf));
                }
            }
            if let Some(sans) = self.faces.get(&FontFaceKey::SansRegular) {
                if sans.lookup_glyph_index(c) != 0 {
                    return Some((FontFaceKey::SansRegular, sans));
                }
            }
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
        let Some(primary) = self.font(key) else {
            return text.chars().count() as f32 * px * 0.55;
        };
        if let Some(ligs) = self.ligatures.get(&key) {
            let mut width = 0.0f32;
            for sg in ligs.apply(primary, text) {
                match sg {
                    ShapedGlyph::GlyphId(gid) => {
                        width += primary.metrics_indexed(gid, px).advance_width;
                    }
                    ShapedGlyph::Char(c) => {
                        if is_default_ignorable(c) {
                            continue;
                        }
                        if let Some((_, f)) = self.resolve_glyph_font(key, c) {
                            width += f.metrics(c, px).advance_width;
                        }
                    }
                }
            }
            return width;
        }
        let mut width = 0.0f32;
        for c in text.chars() {
            if is_default_ignorable(c) {
                continue;
            }
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
        let Some(primary) = self.font(key) else {
            return x;
        };
        let px_key = (px.max(4.0) * 4.0).round() as u32;
        let actual_px = px_key as f32 / 4.0;
        let mut pen_x = x;

        let mut cache = self.glyph_cache.lock().unwrap();
        if let Some(ligs) = self.ligatures.get(&key) {
            for sg in ligs.apply(primary, text) {
                match sg {
                    ShapedGlyph::GlyphId(gid) => {
                        let (metrics, bitmap) = primary.rasterize_indexed(gid, actual_px);
                        if metrics.width > 0 && metrics.height > 0 {
                            let gx = (pen_x + metrics.xmin as f32).round() as i32;
                            let gy = (baseline_y - metrics.ymin as f32 - metrics.height as f32).round() as i32;
                            fb.blit_alpha_mask(gx, gy, metrics.width, metrics.height, &bitmap, color);
                        }
                        pen_x += metrics.advance_width;
                    }
                    ShapedGlyph::Char(c) => {
                        if is_default_ignorable(c) {
                            continue;
                        }
                        let Some((use_key, use_font)) = self.resolve_glyph_font(key, c) else {
                            continue;
                        };
                        let (metrics, bitmap) = cache
                            .entry((use_key, c, px_key))
                            .or_insert_with(|| use_font.rasterize(c, actual_px));
                        if metrics.width > 0 && metrics.height > 0 {
                            let gx = (pen_x + metrics.xmin as f32).round() as i32;
                            let gy = (baseline_y - metrics.ymin as f32 - metrics.height as f32).round() as i32;
                            fb.blit_alpha_mask(gx, gy, metrics.width, metrics.height, bitmap, color);
                        }
                        pen_x += metrics.advance_width;
                    }
                }
            }
            return pen_x;
        }

        for c in text.chars() {
            if is_default_ignorable(c) {
                continue;
            }
            let Some((use_key, use_font)) = self.resolve_glyph_font(key, c) else {
                continue;
            };
            let (metrics, bitmap) = cache
                .entry((use_key, c, px_key))
                .or_insert_with(|| use_font.rasterize(c, actual_px));
            if metrics.width > 0 && metrics.height > 0 {
                let gx = (pen_x + metrics.xmin as f32).round() as i32;
                let gy = (baseline_y - metrics.ymin as f32 - metrics.height as f32).round() as i32;
                fb.blit_alpha_mask(gx, gy, metrics.width, metrics.height, bitmap, color);
            }
            pen_x += metrics.advance_width;
        }
        pen_x
    }
}

fn decode_web_font(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() >= 4 && &bytes[0..4] == b"wOF2" {
        return wuff::decompress_woff2(bytes).ok();
    }
    if bytes.len() >= 4 && &bytes[0..4] == b"wOFF" {
        return wuff::decompress_woff1(bytes)
            .ok()
            .or_else(|| decode_woff1(bytes));
    }
    None
}

fn decode_woff1(bytes: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    if bytes.len() < 44 || &bytes[0..4] != b"wOFF" {
        return None;
    }
    let flavor = &bytes[4..8];
    let num_tables = u16::from_be_bytes([bytes[12], bytes[13]]) as usize;
    if num_tables == 0 || 44 + num_tables * 20 > bytes.len() {
        return None;
    }
    let mut entry_selector = 0u16;
    let mut search_range = 16u16;
    while (search_range as usize) * 2 <= num_tables * 16 {
        search_range *= 2;
        entry_selector += 1;
    }
    let range_shift = (num_tables as u16).saturating_mul(16).saturating_sub(search_range);

    let mut tables: Vec<([u8; 4], u32, Vec<u8>)> = Vec::with_capacity(num_tables);
    for i in 0..num_tables {
        let base = 44 + i * 20;
        let tag = [bytes[base], bytes[base + 1], bytes[base + 2], bytes[base + 3]];
        let offset = u32::from_be_bytes([bytes[base + 4], bytes[base + 5], bytes[base + 6], bytes[base + 7]]) as usize;
        let comp_len = u32::from_be_bytes([bytes[base + 8], bytes[base + 9], bytes[base + 10], bytes[base + 11]]) as usize;
        let orig_len = u32::from_be_bytes([bytes[base + 12], bytes[base + 13], bytes[base + 14], bytes[base + 15]]) as usize;
        let checksum = u32::from_be_bytes([bytes[base + 16], bytes[base + 17], bytes[base + 18], bytes[base + 19]]);
        if offset.checked_add(comp_len)? > bytes.len() {
            return None;
        }
        let slice = &bytes[offset..offset + comp_len];
        let data = if comp_len < orig_len {
            let mut dec = flate2::read::ZlibDecoder::new(slice);
            let mut buf = Vec::with_capacity(orig_len);
            dec.read_to_end(&mut buf).ok()?;
            buf
        } else {
            slice.to_vec()
        };
        tables.push((tag, checksum, data));
    }

    let header_len = 12 + num_tables * 16;
    let mut out = Vec::with_capacity(header_len + tables.iter().map(|t| t.2.len() + 4).sum::<usize>());
    out.extend_from_slice(flavor);
    out.extend_from_slice(&(num_tables as u16).to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&range_shift.to_be_bytes());

    let mut cur_offset = header_len;
    for (tag, checksum, data) in &tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum.to_be_bytes());
        out.extend_from_slice(&(cur_offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let padded = (data.len() + 3) & !3;
        cur_offset += padded;
    }
    for (_, _, data) in &tables {
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    Some(out)
}
