use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use fontdue::{Font, FontSettings};

#[derive(Clone, Debug)]
pub enum ShapedGlyph {
    Char(char),
    GlyphId(u16),
}

#[derive(Clone, Debug, Default)]
pub struct FontLigatures {
    by_first: HashMap<u16, Vec<(Vec<u16>, u16)>>,
}

impl FontLigatures {
    pub fn apply(&self, font: &Font, text: &str) -> Vec<ShapedGlyph> {
        let chars: Vec<(char, u16)> = text
            .chars()
            .map(|c| (c, font.lookup_glyph_index(c)))
            .collect();
        let mut out = Vec::with_capacity(chars.len());
        let mut i = 0;
        while i < chars.len() {
            let (ch, gid) = chars[i];
            let mut matched = None;
            if gid != 0 {
                if let Some(cands) = self.by_first.get(&gid) {
                    for (comps, target) in cands {
                        if i + 1 + comps.len() <= chars.len()
                            && comps
                                .iter()
                                .enumerate()
                                .all(|(k, &cg)| chars[i + 1 + k].1 == cg)
                        {
                            matched = Some((comps.len(), *target));
                            break;
                        }
                    }
                }
            }
            if let Some((comp_len, lig_gid)) = matched {
                out.push(ShapedGlyph::GlyphId(lig_gid));
                i += 1 + comp_len;
            } else {
                out.push(ShapedGlyph::Char(ch));
                i += 1;
            }
        }
        out
    }
}

pub fn extract_gsub_ligatures(font_bytes: &[u8]) -> Option<Arc<FontLigatures>> {
    let face = ttf_parser::Face::parse(font_bytes, 0).ok()?;
    let gsub = face.tables().gsub?;
    let mut by_first: HashMap<u16, Vec<(Vec<u16>, u16)>> = HashMap::new();
    for lookup in gsub.lookups {
        for sub in lookup.subtables.into_iter::<ttf_parser::gsub::SubstitutionSubtable>() {
            if let ttf_parser::gsub::SubstitutionSubtable::Ligature(lig_sub) = sub {
                let mut first_gids = Vec::new();
                match lig_sub.coverage {
                    ttf_parser::opentype_layout::Coverage::Format1 { glyphs } => {
                        for g in glyphs {
                            first_gids.push(g.0);
                        }
                    }
                    ttf_parser::opentype_layout::Coverage::Format2 { records } => {
                        for rec in records {
                            for g in rec.start.0..=rec.end.0 {
                                first_gids.push(g);
                            }
                        }
                    }
                }
                for (first_gid, lig_set) in first_gids.into_iter().zip(lig_sub.ligature_sets) {
                    let entry = by_first.entry(first_gid).or_default();
                    for lig in lig_set {
                        let comps: Vec<u16> = lig.components.into_iter().map(|g| g.0).collect();
                        if !comps.is_empty() {
                            entry.push((comps, lig.glyph.0));
                        }
                    }
                }
            }
        }
    }
    if by_first.is_empty() {
        return None;
    }
    for list in by_first.values_mut() {
        list.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    }
    Some(Arc::new(FontLigatures { by_first }))
}

static WEB_FONT_CACHE: OnceLock<Mutex<Vec<(String, u16, Arc<Font>, Option<Arc<FontLigatures>>)>>> =
    OnceLock::new();

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

pub fn register_web_font_arc(family: &str, font: Arc<Font>) {
    register_web_font_arc_weighted_with_ligatures(family, 400, font, None);
}

pub fn register_web_font_arc_weighted(family: &str, weight: u16, font: Arc<Font>) {
    register_web_font_arc_weighted_with_ligatures(family, weight, font, None);
}

pub fn register_web_font_arc_weighted_with_ligatures(
    family: &str,
    weight: u16,
    font: Arc<Font>,
    ligs: Option<Arc<FontLigatures>>,
) {
    let norm = family.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
    if norm.is_empty() {
        return;
    }
    let cache = WEB_FONT_CACHE.get_or_init(|| Mutex::new(Vec::new()));
    let mut guard = cache.lock().unwrap();
    if !guard.iter().any(|(_, _, f, _)| Arc::ptr_eq(f, &font)) {
        guard.push((norm, weight, font, ligs));
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
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
            "C:\\Windows\\Fonts\\segoeui.ttf",
            "C:\\Windows\\Fonts\\arial.ttf",
        ]),
        sans_bold: load_first(&[
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
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
            "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
            "/usr/share/fonts/truetype/freefont/FreeSerif.ttf",
            "/System/Library/Fonts/Supplemental/Georgia.ttf",
            "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
            "C:\\Windows\\Fonts\\georgia.ttf",
            "C:\\Windows\\Fonts\\times.ttf",
        ]),
        serif_bold: load_first(&[
            "/usr/share/fonts/truetype/liberation/LiberationSerif-Bold.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf",
            "/usr/share/fonts/truetype/freefont/FreeSerifBold.ttf",
            "/System/Library/Fonts/Supplemental/Georgia Bold.ttf",
            "/System/Library/Fonts/Supplemental/Times New Roman Bold.ttf",
            "C:\\Windows\\Fonts\\georgiab.ttf",
            "C:\\Windows\\Fonts\\timesbd.ttf",
        ]),
        mono: load_first(&[
            "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/freefont/FreeMono.ttf",
            "/System/Library/Fonts/Monaco.ttf",
            "/System/Library/Fonts/Supplemental/Courier New.ttf",
            "/System/Library/Fonts/Supplemental/Andale Mono.ttf",
            "C:\\Windows\\Fonts\\consola.ttf",
            "C:\\Windows\\Fonts\\cour.ttf",
        ]),
        mono_bold: load_first(&[
            "/usr/share/fonts/truetype/liberation/LiberationMono-Bold.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
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
    custom_fonts: Vec<(String, u16, Arc<Font>, Option<Arc<FontLigatures>>)>,
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
                    .map(|(n, w, f, l)| (n.clone(), *w, Arc::clone(f), l.clone()))
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

    fn pick_kind(&self, families: &[String], weight: u16, italic: bool) -> u16 {
        let bold = weight >= 600;
        for fam in families {
            let low = fam.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
            if let Some((pos, _)) = self
                .custom_fonts
                .iter()
                .enumerate()
                .filter(|(_, (n, _, _, _))| n == &low)
                .min_by_key(|(_, (_, w, _, _))| (*w as i32 - weight as i32).abs())
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
                if bold {
                    return 1;
                }
                if italic && self.sans_italic.is_some() {
                    return 6;
                }
                return 0;
            }
        }
        if bold {
            1
        } else if italic && self.sans_italic.is_some() {
            6
        } else {
            0
        }
    }

    fn font_for(&self, kind: u16) -> Option<&Arc<Font>> {
        if kind >= 100 {
            if let Some((_, _, f, _)) = self.custom_fonts.get((kind - 100) as usize) {
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
            if let Some((fam, weight, _, _)) = self.custom_fonts.get((kind - 100) as usize) {
                let mut best: Option<(i32, &Arc<Font>)> = None;
                for (i, (n, w, f, _)) in self.custom_fonts.iter().enumerate() {
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
        self.measure_str_styled(text, families, font_size, weight, false)
    }

    pub fn measure_str_styled(
        &mut self,
        text: &str,
        families: &[String],
        font_size: f32,
        weight: u16,
        italic: bool,
    ) -> f32 {
        let kind = self.pick_kind(families, weight, italic);
        let px_key = (font_size.max(4.0) * 4.0).round() as u32;
        let px = px_key as f32 / 4.0;
        if kind >= 100 {
            if let Some((_, _, font, Some(ligs))) = self.custom_fonts.get((kind - 100) as usize) {
                let font_arc = Arc::clone(font);
                let ligs_arc = Arc::clone(ligs);
                let shaped = ligs_arc.apply(&font_arc, text);
                let mut total = 0.0f32;
                for sg in shaped {
                    match sg {
                        ShapedGlyph::GlyphId(gid) => {
                            total += font_arc.metrics_indexed(gid, px).advance_width;
                        }
                        ShapedGlyph::Char(c) => {
                            if is_default_ignorable(c) {
                                continue;
                            }
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
                    }
                }
                return total;
            }
        }
        let mut total = 0.0f32;
        for c in text.chars() {
            if is_default_ignorable(c) {
                continue;
            }
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
