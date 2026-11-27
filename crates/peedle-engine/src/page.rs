use std::collections::HashMap;
use std::time::Instant;

use peedle_css::{compute_styles, parse_stylesheet, StyleMap, Stylesheet, Viewport};
use peedle_html::{parse_html, Document, NodeId};
use peedle_layout::{layout_document, LayoutTree};
use peedle_net::HttpClient;
use peedle_render::{paint_layout, Canvas2D, DecodedImage, FontSystem, Framebuffer, ImageCache};
use peedle_url::Url;

use crate::internal_pages::render_internal_page;

#[derive(Clone, Debug)]
pub struct NetworkEntry {
    pub method: String,
    pub url: String,
    pub status: u16,
    pub content_type: String,
    pub bytes: usize,
    pub duration_ms: u64,
}

#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
}

pub struct Page {
    pub url: String,
    pub base_url: Option<Url>,
    pub document: Document,
    pub stylesheets: Vec<Stylesheet>,
    pub styles: StyleMap,
    pub layout: LayoutTree,
    pub images: ImageCache,
    pub image_sizes: HashMap<String, (u32, u32)>,
    pub canvases: HashMap<NodeId, Canvas2D>,
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub hovered_node: Option<NodeId>,
    pub hovered_href: Option<String>,
    pub focused_node: Option<NodeId>,
    pub active_node: Option<NodeId>,
    pub console_logs: Vec<String>,
    pub network_log: Vec<NetworkEntry>,
    pub history: Vec<HistoryEntry>,
    pub history_index: usize,
    pub client: HttpClient,
    pub fonts: FontSystem,
    pub dark_mode: bool,
    pub dirty_style: bool,
    pub dirty_layout: bool,
}

impl Page {
    pub fn new(viewport_w: f32, viewport_h: f32) -> Self {
        let mut page = Self {
            url: "peedle://newtab".to_string(),
            base_url: None,
            document: Document::new(),
            stylesheets: Vec::new(),
            styles: StyleMap::default(),
            layout: LayoutTree::default(),
            images: ImageCache::new(),
            image_sizes: HashMap::new(),
            canvases: HashMap::new(),
            viewport_w,
            viewport_h,
            scroll_x: 0.0,
            scroll_y: 0.0,
            hovered_node: None,
            hovered_href: None,
            focused_node: None,
            active_node: None,
            console_logs: Vec::new(),
            network_log: Vec::new(),
            history: Vec::new(),
            history_index: 0,
            client: HttpClient::new(),
            fonts: FontSystem::load_system_fonts(),
            dark_mode: false,
            dirty_style: true,
            dirty_layout: true,
        };
        let _ = page.navigate("peedle://newtab");
        page
    }

    pub fn title(&self) -> String {
        let t = self.document.title();
        if t.is_empty() {
            self.url.clone()
        } else {
            t
        }
    }

    pub fn navigate(&mut self, raw_input: &str) -> Result<(), String> {
        let target = normalize_navigation_input(raw_input);
        self.load_url_internal(&target, true)
    }

    pub fn reload(&mut self) -> Result<(), String> {
        let u = self.url.clone();
        self.load_url_internal(&u, false)
    }

    pub fn can_go_back(&self) -> bool {
        self.history_index > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.history_index + 1 < self.history.len()
    }

    pub fn go_back(&mut self) -> Result<(), String> {
        if self.can_go_back() {
            self.history_index -= 1;
            let u = self.history[self.history_index].url.clone();
            self.load_url_internal(&u, false)?;
        }
        Ok(())
    }

    pub fn go_forward(&mut self) -> Result<(), String> {
        if self.can_go_forward() {
            self.history_index += 1;
            let u = self.history[self.history_index].url.clone();
            self.load_url_internal(&u, false)?;
        }
        Ok(())
    }

    fn load_url_internal(&mut self, target: &str, push_history: bool) -> Result<(), String> {
        self.network_log.clear();
        self.scroll_x = 0.0;
        self.scroll_y = 0.0;
        self.hovered_node = None;
        self.hovered_href = None;
        self.focused_node = None;
        self.active_node = None;
        self.canvases.clear();

        if let Some((_title, html)) = render_internal_page(target) {
            self.url = target.to_string();
            self.base_url = None;
            self.load_html_string(&html, target);
            if push_history {
                self.push_history_entry();
            }
            return Ok(());
        }

        let start = Instant::now();
        let resp = match self.client.get(target) {
            Ok(r) => r,
            Err(e) => {
                let err_html = format!(
                    "<!DOCTYPE html><html><head><title>Load Error</title><style>body{{font-family:sans-serif;padding:48px;background:#0f1117;color:#e6edf3;}}.box{{max-width:600px;margin:0 auto;background:#171c26;border:1px solid #334155;border-radius:12px;padding:24px;}}h1{{margin:0 0 12px 0;font-size:22px;color:#f87171;}}p{{color:#cbd5e1;font-family:monospace;}}</style></head><body><div class=\"box\"><h1>Could not load page</h1><p>{}</p><p>{}</p></div></body></html>",
                    escape_text(target),
                    escape_text(&e.to_string())
                );
                self.url = target.to_string();
                self.base_url = Url::parse(target).ok();
                self.load_html_string(&err_html, target);
                if push_history {
                    self.push_history_entry();
                }
                return Err(e.to_string());
            }
        };

        let elapsed = start.elapsed().as_millis() as u64;
        let final_url = resp.url.href();
        self.network_log.push(NetworkEntry {
            method: "GET".to_string(),
            url: final_url.clone(),
            status: resp.status,
            content_type: resp.content_type().to_string(),
            bytes: resp.body.len(),
            duration_ms: elapsed,
        });

        self.url = final_url.clone();
        self.base_url = Some(resp.url.clone());

        let ct = resp.content_type().to_ascii_lowercase();
        let html = if ct.contains("image/") {
            if let Ok(decoded) = DecodedImage::decode(&resp.body) {
                self.image_sizes
                    .insert(final_url.clone(), (decoded.width, decoded.height));
                self.images.insert(final_url.clone(), decoded);
            }
            format!(
                "<!DOCTYPE html><html><body style=\"margin:0;background:#111;display:flex;justify-content:center;align-items:center;min-height:100vh;\"><img src=\"{}\"></body></html>",
                escape_text(&final_url)
            )
        } else if ct.contains("text/plain") || ct.contains("application/json") {
            format!(
                "<!DOCTYPE html><html><body style=\"margin:16px;font-family:monospace;white-space:pre-wrap;\">{}</body></html>",
                escape_text(&resp.text())
            )
        } else {
            resp.text()
        };

        self.load_html_string(&html, &final_url);
        if let Some(frag) = resp.url.fragment.as_deref() {
            if !frag.is_empty() {
                self.scroll_to_fragment(frag);
            }
        }
        if push_history {
            self.push_history_entry();
        }
        Ok(())
    }

    pub fn scroll_to_fragment(&mut self, fragment: &str) {
        self.recompute_if_needed();
        if let Some(nid) = self.document.get_element_by_id(fragment) {
            if let Some(rect) = self.layout.bounding_rect(nid) {
                self.scroll_y = rect.y;
                self.clamp_scroll();
            }
        }
    }

    pub fn load_html_string(&mut self, html: &str, url: &str) {
        self.url = url.to_string();
        if self.base_url.is_none() {
            self.base_url = Url::parse(url).ok();
        }
        self.document = parse_html(html);
        self.document.url = url.to_string();
        self.collect_stylesheets_and_resources();
        self.dirty_style = true;
        self.dirty_layout = true;
        self.recompute_if_needed();
    }

    fn push_history_entry(&mut self) {
        if !self.history.is_empty() && self.history_index + 1 < self.history.len() {
            self.history.truncate(self.history_index + 1);
        }
        self.history.push(HistoryEntry {
            url: self.url.clone(),
            title: self.title(),
        });
        self.history_index = self.history.len() - 1;
    }

    fn resolve_subresource_url(&self, raw: &str) -> Option<String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("javascript:") {
            return None;
        }
        if let Some(ref base) = self.base_url {
            base.join(trimmed).ok().map(|u| u.href())
        } else {
            Url::parse(trimmed).ok().map(|u| u.href())
        }
    }

    fn collect_stylesheets_and_resources(&mut self) {
        self.stylesheets.clear();
        let nodes = self.document.descendants(self.document.root);
        let mut css_links = Vec::new();
        let mut img_srcs = Vec::new();

        for nid in nodes {
            let Some(el) = self.document.element(nid) else {
                continue;
            };
            if !el.is_html() {
                continue;
            }
            match el.tag_name.as_str() {
                "style" => {
                    let css_text = self.document.text_content(nid);
                    self.stylesheets.push(parse_stylesheet(&css_text));
                }
                "link" => {
                    let is_sheet = el
                        .get_attr("rel")
                        .map_or(false, |r| {
                            r.split_ascii_whitespace()
                                .any(|tok| tok.eq_ignore_ascii_case("stylesheet"))
                        });
                    if is_sheet {
                        if let Some(href) = el.get_attr("href") {
                            if let Some(resolved) = self.resolve_subresource_url(href) {
                                css_links.push(resolved);
                            }
                        }
                    }
                }
                "img" => {
                    if let Some(src) = el.get_attr("src") {
                        img_srcs.push((src.to_string(), self.resolve_subresource_url(src)));
                    }
                }
                _ => {}
            }
        }

        for css_url in css_links.into_iter().take(24) {
            let start = Instant::now();
            if let Ok(resp) = self.client.get(&css_url) {
                self.network_log.push(NetworkEntry {
                    method: "GET".to_string(),
                    url: css_url.clone(),
                    status: resp.status,
                    content_type: resp.content_type().to_string(),
                    bytes: resp.body.len(),
                    duration_ms: start.elapsed().as_millis() as u64,
                });
                if resp.status < 400 {
                    self.stylesheets.push(parse_stylesheet(&resp.text()));
                }
            }
        }

        for (raw_src, resolved_opt) in img_srcs.into_iter().take(48) {
            if self.images.contains(&raw_src) {
                continue;
            }
            let Some(resolved) = resolved_opt else {
                continue;
            };
            let start = Instant::now();
            if let Ok(resp) = self.client.get(&resolved) {
                self.network_log.push(NetworkEntry {
                    method: "GET".to_string(),
                    url: resolved.clone(),
                    status: resp.status,
                    content_type: resp.content_type().to_string(),
                    bytes: resp.body.len(),
                    duration_ms: start.elapsed().as_millis() as u64,
                });
                if resp.status < 400 {
                    if let Ok(decoded) = DecodedImage::decode(&resp.body) {
                        self.image_sizes
                            .insert(raw_src.clone(), (decoded.width, decoded.height));
                        self.images.insert(raw_src, decoded);
                    }
                }
            }
        }
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        if (self.viewport_w - w).abs() > 0.5 || (self.viewport_h - h).abs() > 0.5 {
            self.viewport_w = w.max(100.0);
            self.viewport_h = h.max(100.0);
            self.dirty_style = true;
            self.dirty_layout = true;
            self.recompute_if_needed();
        }
    }

    pub fn recompute_if_needed(&mut self) {
        if self.dirty_style {
            self.styles = compute_styles(
                &self.document,
                &self.stylesheets,
                Viewport {
                    width: self.viewport_w,
                    height: self.viewport_h,
                    dark_mode: self.dark_mode,
                },
            );
            self.dirty_style = false;
            self.dirty_layout = true;
        }
        if self.dirty_layout {
            self.layout = layout_document(
                &self.document,
                &self.styles,
                &self.image_sizes,
                self.viewport_w,
                self.viewport_h,
            );
            self.dirty_layout = false;
            self.clamp_scroll();
        }
    }

    pub fn render_to_framebuffer(&mut self, fb: &mut Framebuffer) {
        self.recompute_if_needed();
        paint_layout(
            &self.document,
            &self.layout,
            fb,
            &self.fonts,
            &self.images,
            &self.canvases,
            self.scroll_x,
            self.scroll_y,
        );
    }

    pub fn render_png_bytes(&mut self) -> Result<Vec<u8>, String> {
        let mut fb = Framebuffer::new(self.viewport_w.ceil() as u32, self.viewport_h.ceil() as u32);
        self.render_to_framebuffer(&mut fb);
        fb.encode_png()
    }

    pub fn save_screenshot(&mut self, path: &std::path::Path) -> Result<(), String> {
        let mut fb = Framebuffer::new(self.viewport_w.ceil() as u32, self.viewport_h.ceil() as u32);
        self.render_to_framebuffer(&mut fb);
        fb.save_png(path)
    }

    pub fn scroll_by(&mut self, dx: f32, dy: f32) {
        self.scroll_x += dx;
        self.scroll_y += dy;
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        let max_x = (self.layout.content_width - self.viewport_w).max(0.0);
        let max_y = (self.layout.content_height - self.viewport_h).max(0.0);
        self.scroll_x = self.scroll_x.clamp(0.0, max_x);
        self.scroll_y = self.scroll_y.clamp(0.0, max_y);
    }

    pub fn on_mouse_move(&mut self, vx: f32, vy: f32) -> bool {
        let hit = self.layout.hit_test(vx + self.scroll_x, vy + self.scroll_y);
        if hit == self.hovered_node {
            return false;
        }
        if let Some(prev) = self.hovered_node {
            if let Some(el) = self.document.element_mut(prev) {
                el.hovered = false;
            }
        }
        self.hovered_node = hit;
        self.hovered_href = hit.and_then(|nid| self.find_ancestor_href(nid));
        if let Some(cur) = hit {
            if let Some(el) = self.document.element_mut(cur) {
                el.hovered = true;
            }
        }
        self.dirty_style = true;
        true
    }

    pub fn on_click(&mut self, vx: f32, vy: f32) -> Option<String> {
        let hit = self.layout.hit_test(vx + self.scroll_x, vy + self.scroll_y);
        if let Some(prev) = self.focused_node {
            if let Some(el) = self.document.element_mut(prev) {
                el.focused = false;
            }
        }
        self.focused_node = None;

        let nid = hit?;
        if let Some(el) = self.document.element_mut(nid) {
            if el.is_html() && el.tag_is("input") {
                let itype = el.get_attr("type").unwrap_or("text").to_ascii_lowercase();
                if itype == "checkbox" {
                    let cur = el.is_checked();
                    el.checked = Some(!cur);
                    self.dirty_style = true;
                    self.dirty_layout = true;
                    return None;
                }
                if itype == "radio" {
                    el.checked = Some(true);
                    self.dirty_style = true;
                    self.dirty_layout = true;
                    return None;
                }
                el.focused = true;
                self.focused_node = Some(nid);
                self.dirty_style = true;
                return None;
            }
            if el.is_html() && el.tag_is("textarea") {
                el.focused = true;
                self.focused_node = Some(nid);
                self.dirty_style = true;
                return None;
            }
        }

        if let Some(href) = self.find_ancestor_href(nid) {
            let resolved = if href.starts_with("peedle://") || href.starts_with("about:") {
                Some(href)
            } else {
                self.resolve_subresource_url(&href)
            };
            if let Some(dest) = resolved {
                let _ = self.navigate(&dest);
                return Some(dest);
            }
        }
        None
    }

    pub fn on_text_input(&mut self, ch: char) -> bool {
        let Some(fid) = self.focused_node else {
            return false;
        };
        if let Some(el) = self.document.element_mut(fid) {
            let mut val = el.current_value();
            if ch == '\u{0008}' {
                val.pop();
            } else if !ch.is_control() || (ch == '\n' && el.tag_is("textarea")) {
                val.push(ch);
            }
            el.input_value = Some(val);
            self.dirty_layout = true;
            return true;
        }
        false
    }

    fn find_ancestor_href(&self, start: NodeId) -> Option<String> {
        let mut cur = Some(start);
        while let Some(id) = cur {
            if let Some(el) = self.document.element(id) {
                if el.is_html() && el.tag_is("a") {
                    if let Some(href) = el.get_attr("href") {
                        return Some(href.to_string());
                    }
                }
            }
            cur = self.document.node(id).parent;
        }
        None
    }
}

pub fn normalize_navigation_input(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "peedle://newtab".to_string();
    }
    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("file://")
        || trimmed.starts_with("data:")
        || trimmed.starts_with("about:")
        || trimmed.starts_with("peedle://")
    {
        return trimmed.to_string();
    }
    if trimmed.contains('.') && !trimmed.contains(' ') {
        return format!("https://{}", trimmed);
    }
    if trimmed.starts_with("localhost") || trimmed.starts_with("127.0.0.1") {
        return format!("http://{}", trimmed);
    }
    let encoded = peedle_url::percent::utf8_percent_encode(
        trimmed,
        peedle_url::percent::EncodeSet::FormUrlEncoded,
    );
    format!("https://duckduckgo.com/html/?q={}", encoded)
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
