use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use fontdue::{Font, FontSettings};

static WEB_FONT_CACHE: OnceLock<Mutex<Vec<(String, u16, Arc<Font>)>>> = OnceLock::new();

pub fn register_web_font_arc(family: &str, font: Arc<Font>) {
    register_web_font_arc_weighted(family, 400, font);
}

pub fn register_web_font_arc_weighted(family: &str, weight: u16, font: Arc<Font>) {
    let norm = family.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
    if norm.is_empty() {
        return;
    }
    let cache = WEB_FONT_CACHE.get_or_init(|| Mutex::new(Vec::new()));
    let mut guard = cache.lock().unwrap();
    if !guard.iter().any(|(_, _, f)| Arc::ptr_eq(f, &font)) {
        guard.push((norm, weight, font));
    }
}

struct SharedLayoutFonts {
    sans: Option<Arc<Font>>,
    sans_bold: Option<Arc<Font>>,
    sans_italic: Option<Arc<Font>>,
    serif: Option<Arc<Font>>,
    serif_bold: Option<Arc<Font>>,
    mono: Option<Arc<Font>>,
    mono_bold: Option<Arc<Font>>,
    fallbacks: Vec<Arc<Font>>,
}

static SHARED_LAYOUT_FONTS: OnceLock<SharedLayoutFonts> = OnceLock::new();

fn shared_fonts() -> &'static SharedLayoutFonts {
    SHARED_LAYOUT_FONTS.get_or_init(|| SharedLayoutFonts {
        sans: load_first(&[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
            "C:\\Windows\\Fonts\\segoeui.ttf",
            "C:\\Windows\\Fonts\\arial.ttf",
        ]),
        sans_bold: load_first(&[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
            "/usr/share/fonts/truetype/freefont/FreeSansBold.ttf",
            "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
            "/System/Library/Fonts/Supplemental/Verdana Bold.ttf",
            "C:\\Windows\\Fonts\\segoeuib.ttf",
            "C:\\Windows\\Fonts\\arialbd.ttf",
        ]),
        sans_italic: load_first(&[
            "/usr/share/fonts/truetype/liberation/LiberationSans-Italic.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Oblique.ttf",
            "/usr/share/fonts/truetype/freefont/FreeSansOblique.ttf",
            "/System/Library/Fonts/Supplemental/Arial Italic.ttf",
            "/System/Library/Fonts/Supplemental/Verdana Italic.ttf",
            "C:\\Windows\\Fonts\\segoeuii.ttf",
            "C:\\Windows\\Fonts\\ariali.ttf",
        ]),
        serif: load_first(&[
            "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
            "/usr/share/fonts/truetype/freefont/FreeSerif.ttf",
            "/System/Library/Fonts/Supplemental/Georgia.ttf",
            "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
            "C:\\Windows\\Fonts\\georgia.ttf",
            "C:\\Windows\\Fonts\\times.ttf",
        ]),
        serif_bold: load_first(&[
            "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSerif-Bold.ttf",
            "/usr/share/fonts/truetype/freefont/FreeSerifBold.ttf",
            "/System/Library/Fonts/Supplemental/Georgia Bold.ttf",
            "/System/Library/Fonts/Supplemental/Times New Roman Bold.ttf",
            "C:\\Windows\\Fonts\\georgiab.ttf",
            "C:\\Windows\\Fonts\\timesbd.ttf",
        ]),
        mono: load_first(&[
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
            "/usr/share/fonts/truetype/freefont/FreeMono.ttf",
            "/System/Library/Fonts/Monaco.ttf",
            "/System/Library/Fonts/Supplemental/Courier New.ttf",
            "/System/Library/Fonts/Supplemental/Andale Mono.ttf",
            "C:\\Windows\\Fonts\\consola.ttf",
            "C:\\Windows\\Fonts\\cour.ttf",
        ]),
        mono_bold: load_first(&[
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Bold.ttf",
            "/System/Library/Fonts/Supplemental/Courier New Bold.ttf",
            "C:\\Windows\\Fonts\\consolab.ttf",
            "C:\\Windows\\Fonts\\courbd.ttf",
        ]),
        fallbacks: load_all(&[
            "/usr/share/fonts/opentype/ipafont-gothic/ipagp.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/unifont/unifont.otf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/Apple Symbols.ttf",
            "C:\\Windows\\Fonts\\seguisym.ttf",
            "C:\\Windows\\Fonts\\msyh.ttc",
        ]),
    })
}

#[derive(Clone)]
pub struct TextMeasurer {
    sans: Option<Arc<Font>>,
    sans_bold: Option<Arc<Font>>,
    sans_italic: Option<Arc<Font>>,
    serif: Option<Arc<Font>>,
    serif_bold: Option<Arc<Font>>,
    mono: Option<Arc<Font>>,
    mono_bold: Option<Arc<Font>>,
    fallbacks: Vec<Arc<Font>>,
    custom_fonts: Vec<(String, u16, Arc<Font>)>,
    width_cache: HashMap<(u16, u32, char), f32>,
}

impl Default for TextMeasurer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextMeasurer {
    pub fn new() -> Self {
        let base = shared_fonts();
        let custom_fonts = WEB_FONT_CACHE
            .get()
            .map(|m| {
                m.lock()
                    .unwrap()
                    .iter()
                    .map(|(n, w, f)| (n.clone(), *w, Arc::clone(f)))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            sans: base.sans.clone(),
            sans_bold: base.sans_bold.clone(),
            sans_italic: base.sans_italic.clone(),
            serif: base.serif.clone(),
            serif_bold: base.serif_bold.clone(),
            mono: base.mono.clone(),
            mono_bold: base.mono_bold.clone(),
            fallbacks: base.fallbacks.clone(),
            custom_fonts,
            width_cache: HashMap::new(),
        }
    }

    fn pick_kind(&self, families: &[String], weight: u16) -> u16 {
        let bold = weight >= 600;
        for fam in families {
            let low = fam.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
            if let Some((pos, _)) = self
                .custom_fonts
                .iter()
                .enumerate()
                .filter(|(_, (n, _, _))| n == &low)
                .min_by_key(|(_, (_, w, _))| (*w as i32 - weight as i32).abs())
            {
                return 100 + pos as u16;
            }
            if low.contains("mono")
                || low.contains("courier")
                || low.contains("consolas")
                || low.contains("menlo")
                || low.contains("monaco")
                || low.contains("fira code")
            {
                return if bold && self.mono_bold.is_some() { 5 } else { 3 };
            }
            if low == "serif"
                || low.contains("times")
                || low.contains("georgia")
                || low.contains("garamond")
            {
                return if bold && self.serif_bold.is_some() { 4 } else { 2 };
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
                return if bold { 1 } else { 0 };
            }
        }
        if bold { 1 } else { 0 }
    }

    fn font_for(&self, kind: u16) -> Option<&Arc<Font>> {
        if kind >= 100 {
            if let Some((_, _, f)) = self.custom_fonts.get((kind - 100) as usize) {
                return Some(f);
            }
        }
        match kind {
            1 => self.sans_bold.as_ref().or(self.sans.as_ref()),
            2 => self.serif.as_ref().or(self.sans.as_ref()),
            3 => self.mono.as_ref().or(self.sans.as_ref()),
            4 => self.serif_bold.as_ref().or(self.serif.as_ref()).or(self.sans.as_ref()),
            5 => self.mono_bold.as_ref().or(self.mono.as_ref()).or(self.sans.as_ref()),
            6 => self.sans_italic.as_ref().or(self.sans.as_ref()),
            _ => self.sans.as_ref(),
        }
    }

    fn resolve_glyph_font(&self, kind: u16, c: char) -> Option<&Arc<Font>> {
        let primary = self.font_for(kind)?;
        if primary.lookup_glyph_index(c) != 0 {
            return Some(primary);
        }
        if kind >= 100 {
            if let Some((fam, weight, _)) = self.custom_fonts.get((kind - 100) as usize) {
                let mut best: Option<(i32, &Arc<Font>)> = None;
                for (i, (n, w, f)) in self.custom_fonts.iter().enumerate() {
                    if i != (kind - 100) as usize && n == fam && f.lookup_glyph_index(c) != 0 {
                        let dist = (*w as i32 - *weight as i32).abs();
                        if best.map_or(true, |(bd, _)| dist < bd) {
                            best = Some((dist, f));
                        }
                    }
                }
                if let Some((_, bf)) = best {
                    return Some(bf);
                }
            }
            if let Some(ref sans) = self.sans {
                if sans.lookup_glyph_index(c) != 0 {
                    return Some(sans);
                }
            }
        }
        for fb in &self.fallbacks {
            if fb.lookup_glyph_index(c) != 0 {
                return Some(fb);
            }
        }
        Some(primary)
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
                    .resolve_glyph_font(kind, c)
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

fn load_all(paths: &[&str]) -> Vec<Arc<Font>> {
    let mut out = Vec::new();
    for &p in paths {
        if Path::new(p).exists() {
            if let Ok(bytes) = std::fs::read(p) {
                if let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) {
                    out.push(Arc::new(font));
                }
            }
        }
    }
    out
}
