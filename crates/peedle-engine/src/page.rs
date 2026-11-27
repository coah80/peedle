use std::time::Instant;

use peedle_css::parse_stylesheet;
use peedle_dom::BrowserJsEnv;
use peedle_html::{parse_html, Document, NodeId};
use peedle_net::HttpClient;
use peedle_render::{paint_layout, DecodedImage, FontSystem, Framebuffer, ImageCache};
use peedle_url::Url;

use crate::internal_pages::{render_history_html, render_internal_page};

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
    pub js_env: BrowserJsEnv,
    pub images: ImageCache,
    pub failed_images: std::collections::HashSet<String>,
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub hovered_node: Option<NodeId>,
    pub hovered_href: Option<String>,
    pub active_node: Option<NodeId>,
    pub console_logs: Vec<String>,
    pub network_log: Vec<NetworkEntry>,
    pub history: Vec<HistoryEntry>,
    pub history_index: usize,
    pub client: HttpClient,
    pub fonts: FontSystem,
    pub dark_mode: bool,
    pub scripts_enabled: bool,
}

impl Page {
    pub fn new(viewport_w: f32, viewport_h: f32) -> Self {
        let client = HttpClient::new();
        let js_env = BrowserJsEnv::new(
            Document::new(),
            "peedle://newtab".to_string(),
            viewport_w,
            viewport_h,
            client.clone(),
        );
        let mut page = Self {
            url: "peedle://newtab".to_string(),
            base_url: None,
            js_env,
            images: ImageCache::new(),
            failed_images: std::collections::HashSet::new(),
            viewport_w,
            viewport_h,
            scroll_x: 0.0,
            scroll_y: 0.0,
            hovered_node: None,
            hovered_href: None,
            active_node: None,
            console_logs: Vec::new(),
            network_log: Vec::new(),
            history: Vec::new(),
            history_index: 0,
            client,
            fonts: FontSystem::load_system_fonts(),
            dark_mode: false,
            scripts_enabled: true,
        };
        let _ = page.navigate("peedle://newtab");
        page
    }

    pub fn title(&self) -> String {
        let t = self.js_env.dom.borrow().doc.title();
        if t.is_empty() {
            self.url.clone()
        } else {
            t
        }
    }

    pub fn layout_box_count(&self) -> usize {
        self.js_env.dom.borrow().layout.boxes.len()
    }

    pub fn outer_html(&self) -> String {
        let st = self.js_env.dom.borrow();
        st.doc.outer_html(st.doc.root)
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
        self.console_logs.clear();
        self.failed_images.clear();
        self.scroll_x = 0.0;
        self.scroll_y = 0.0;
        self.hovered_node = None;
        self.hovered_href = None;
        self.active_node = None;

        if target == "peedle://history" {
            let pairs: Vec<(String, String)> = self
                .history
                .iter()
                .map(|h| (h.url.clone(), h.title.clone()))
                .collect();
            let html = render_history_html(&pairs);
            self.url = target.to_string();
            self.base_url = None;
            self.load_html_string(&html, target);
            if push_history {
                self.push_history_entry();
            }
            return Ok(());
        }

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
        let mut predecoded_image: Option<(String, DecodedImage)> = None;
        let html = if ct.contains("image/") {
            if let Ok(decoded) = DecodedImage::decode(&resp.body) {
                predecoded_image = Some((final_url.clone(), decoded));
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

        if let Some((img_url, decoded)) = predecoded_image {
            self.images.insert(img_url, decoded);
        }

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
        let y_opt = {
            let st = self.js_env.dom.borrow();
            st.doc
                .get_element_by_id(fragment)
                .and_then(|nid| st.layout.bounding_rect(nid))
                .map(|r| r.y)
        };
        if let Some(y) = y_opt {
            self.scroll_y = y;
            self.clamp_scroll();
        }
    }

    pub fn load_html_string(&mut self, html: &str, url: &str) {
        self.url = url.to_string();
        if self.base_url.is_none() {
            self.base_url = Url::parse(url).ok();
        }
        let mut doc = parse_html(html);
        doc.url = url.to_string();

        self.js_env = BrowserJsEnv::new(
            doc,
            url.to_string(),
            self.viewport_w,
            self.viewport_h,
            self.client.clone(),
        );
        self.js_env.dom.borrow_mut().dark_mode = self.dark_mode;

        self.fetch_external_stylesheets();
        self.recompute_if_needed();
        self.fetch_missing_images();
        self.recompute_if_needed();

        if self.scripts_enabled {
            let _errs = self.js_env.run_document_scripts();
            self.sync_console_from_js();
            self.fetch_missing_images();
            self.recompute_if_needed();
            self.check_pending_navigation();
        }
    }

    pub fn eval_js(&mut self, code: &str) -> Result<String, String> {
        let res = self.js_env.eval(code);
        self.sync_console_from_js();
        self.fetch_missing_images();
        self.recompute_if_needed();
        self.check_pending_navigation();
        match res {
            Ok(v) => {
                let s = self
                    .js_env
                    .rt
                    .to_string(&v)
                    .unwrap_or_else(|_| format!("{:?}", v));
                self.console_logs.push(format!("=> {}", s));
                Ok(s)
            }
            Err(e) => {
                let msg = e.to_string();
                self.console_logs.push(format!("[Error] {}", msg));
                Err(msg)
            }
        }
    }

    pub fn advance_time_ms(&mut self, delta_ms: u64) {
        self.js_env.advance_time_ms(delta_ms);
        self.sync_console_from_js();
        self.fetch_missing_images();
        self.recompute_if_needed();
        self.check_pending_navigation();
    }

    fn sync_console_from_js(&mut self) {
        if !self.js_env.rt.console_lines.is_empty() {
            let drained: Vec<String> = self.js_env.rt.console_lines.drain(..).collect();
            self.console_logs.extend(drained);
            if self.console_logs.len() > 500 {
                let drop_n = self.console_logs.len() - 500;
                self.console_logs.drain(..drop_n);
            }
        }
    }

    fn check_pending_navigation(&mut self) {
        let pending = self.js_env.dom.borrow_mut().pending_navigation.take();
        if let Some(dest) = pending {
            if let Some(resolved) = self.resolve_subresource_url(&dest) {
                if resolved != self.url {
                    let _ = self.navigate(&resolved);
                }
            }
        }
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

    fn fetch_external_stylesheets(&mut self) {
        let css_links: Vec<(NodeId, String)> = {
            let st = self.js_env.dom.borrow();
            let mut out = Vec::new();
            for nid in st.doc.descendants(st.doc.root) {
                let Some(el) = st.doc.element(nid) else {
                    continue;
                };
                if !el.is_html() || !el.tag_is("link") {
                    continue;
                }
                let is_sheet = el.get_attr("rel").map_or(false, |r| {
                    r.split_ascii_whitespace()
                        .any(|tok| tok.eq_ignore_ascii_case("stylesheet"))
                });
                if is_sheet {
                    if let Some(media) = el.get_attr("media") {
                        let m = media.trim();
                        if !m.is_empty() {
                            let vp = peedle_css::Viewport {
                                width: self.viewport_w,
                                height: self.viewport_h,
                                dark_mode: self.dark_mode,
                            };
                            if !peedle_css::parse_media_query_list(m).matches(&vp) {
                                continue;
                            }
                        }
                    }
                    if let Some(href) = el.get_attr("href") {
                        if let Some(resolved) = self.resolve_subresource_url(href) {
                            out.push((nid, resolved));
                        }
                    }
                }
            }
            out
        };

        let tasks: Vec<(NodeId, String)> = css_links.into_iter().take(24).collect();
        let queue = std::sync::Mutex::new(std::collections::VecDeque::from(tasks));
        let results = std::sync::Mutex::new(Vec::new());
        let workers = queue.lock().unwrap().len().min(6);
        let client = &self.client;

        if workers > 0 {
            std::thread::scope(|s| {
                for _ in 0..workers {
                    s.spawn(|| loop {
                        let item = queue.lock().unwrap().pop_front();
                        let Some((nid, css_url)) = item else {
                            break;
                        };
                        let start = Instant::now();
                        if let Ok(resp) = client.get(&css_url) {
                            let elapsed = start.elapsed().as_millis() as u64;
                            let ct = resp.content_type().to_string();
                            let bytes = resp.body.len();
                            let status = resp.status;
                            let low = ct.to_ascii_lowercase();
                            let mime_ok = low.is_empty()
                                || low.contains("css")
                                || low.contains("text/plain")
                                || low.contains("octet-stream");
                            let parsed = if status < 400 && mime_ok {
                                Some(parse_stylesheet(&resp.text()))
                            } else {
                                None
                            };
                            let entry = NetworkEntry {
                                method: "GET".to_string(),
                                url: css_url,
                                status,
                                content_type: ct,
                                bytes,
                                duration_ms: elapsed,
                            };
                            results.lock().unwrap().push((nid, entry, parsed));
                        }
                    });
                }
            });
        }

        let mut sheets = std::collections::HashMap::new();
        for (nid, entry, parsed) in results.into_inner().unwrap() {
            self.network_log.push(entry);
            if let Some(sheet) = parsed {
                sheets.insert(nid, sheet);
            }
        }

        let mut st = self.js_env.dom.borrow_mut();
        st.external_stylesheets = sheets;
        st.dirty_style = true;
    }

    fn fetch_missing_images(&mut self) {
        let img_srcs: Vec<(String, Option<String>)> = {
            let st = self.js_env.dom.borrow();
            let mut out = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let mut push_src = |raw: String, resolved: Option<String>| {
                if seen.insert(raw.clone()) {
                    out.push((raw, resolved));
                }
            };
            for nid in st.doc.descendants(st.doc.root) {
                if let Some(el) = st.doc.element(nid) {
                    if el.is_html() && el.tag_is("img") {
                        if let Some(src) = el.get_attr("src") {
                            if !self.images.contains(src) {
                                push_src(src.to_string(), self.resolve_subresource_url(src));
                            } else if !st.image_sizes.contains_key(src) {
                                push_src(src.to_string(), None);
                            }
                        }
                    } else if el.is_html() && el.tag_is("object") {
                        if let Some(data_url) = el.get_attr("data") {
                            let low = data_url.to_ascii_lowercase();
                            let looks_image = low.starts_with("data:image/")
                                || low.ends_with(".png")
                                || low.ends_with(".gif")
                                || low.ends_with(".jpg")
                                || low.ends_with(".jpeg")
                                || low.ends_with(".webp");
                            if looks_image {
                                if !self.images.contains(data_url) {
                                    push_src(
                                        data_url.to_string(),
                                        self.resolve_subresource_url(data_url),
                                    );
                                } else if !st.image_sizes.contains_key(data_url) {
                                    push_src(data_url.to_string(), None);
                                }
                            }
                        }
                    }
                }
                for style in [st.styles.get(nid), st.styles.before(nid), st.styles.after(nid)]
                    .into_iter()
                    .flatten()
                {
                    if let peedle_css::BackgroundImage::Url(ref u) = style.background_image {
                        if !self.images.contains(u) {
                            push_src(u.clone(), self.resolve_subresource_url(u));
                        }
                    }
                    if let Some(ref mu) = style.mask_image {
                        if !self.images.contains(mu) {
                            push_src(mu.clone(), self.resolve_subresource_url(mu));
                        }
                    }
                }
            }
            out
        };

        let mut new_sizes = Vec::new();
        let mut to_fetch: Vec<(String, String)> = Vec::new();
        for (raw_src, resolved_opt) in img_srcs.into_iter().take(96) {
            if self.failed_images.contains(&raw_src) {
                continue;
            }
            if let Some(existing) = self.images.get(&raw_src) {
                new_sizes.push((raw_src, (existing.width, existing.height)));
                continue;
            }
            match resolved_opt {
                Some(resolved) => to_fetch.push((raw_src, resolved)),
                None => {
                    self.failed_images.insert(raw_src);
                }
            }
        }

        if !to_fetch.is_empty() {
            let queue = std::sync::Mutex::new(std::collections::VecDeque::from(to_fetch));
            let results = std::sync::Mutex::new(Vec::new());
            let workers = queue.lock().unwrap().len().min(8);
            let client = &self.client;

            std::thread::scope(|s| {
                for _ in 0..workers {
                    s.spawn(|| loop {
                        let item = queue.lock().unwrap().pop_front();
                        let Some((raw_src, resolved)) = item else {
                            break;
                        };
                        let start = Instant::now();
                        match client.get(&resolved) {
                            Ok(resp) => {
                                let entry = NetworkEntry {
                                    method: "GET".to_string(),
                                    url: resolved,
                                    status: resp.status,
                                    content_type: resp.content_type().to_string(),
                                    bytes: resp.body.len(),
                                    duration_ms: start.elapsed().as_millis() as u64,
                                };
                                let decoded = if resp.status < 400 {
                                    DecodedImage::decode(&resp.body).ok()
                                } else {
                                    None
                                };
                                results.lock().unwrap().push((raw_src, Some(entry), decoded));
                            }
                            Err(_) => {
                                results.lock().unwrap().push((raw_src, None, None));
                            }
                        }
                    });
                }
            });

            for (raw_src, entry_opt, decoded_opt) in results.into_inner().unwrap() {
                if let Some(entry) = entry_opt {
                    self.network_log.push(entry);
                }
                if let Some(decoded) = decoded_opt {
                    new_sizes.push((raw_src.clone(), (decoded.width, decoded.height)));
                    self.images.insert(raw_src, decoded);
                } else {
                    self.failed_images.insert(raw_src);
                }
            }
        }

        if !new_sizes.is_empty() {
            let mut st = self.js_env.dom.borrow_mut();
            for (k, sz) in new_sizes {
                st.image_sizes.insert(k, sz);
            }
            st.dirty_layout = true;
        }
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        if (self.viewport_w - w).abs() > 0.5 || (self.viewport_h - h).abs() > 0.5 {
            self.viewport_w = w.max(100.0);
            self.viewport_h = h.max(100.0);
            {
                let mut st = self.js_env.dom.borrow_mut();
                st.viewport_w = self.viewport_w;
                st.viewport_h = self.viewport_h;
                st.dirty_style = true;
                st.dirty_layout = true;
            }
            let _ = self.js_env.dispatch_dom_event(None, "resize", false, false);
            self.sync_console_from_js();
            self.recompute_if_needed();
        }
    }

    pub fn recompute_if_needed(&mut self) {
        let font_reqs: Vec<(String, String)> = {
            let mut st = self.js_env.dom.borrow_mut();
            st.dark_mode = self.dark_mode;
            let visited_urls: std::collections::HashSet<String> =
                st.subdoc_urls.values().cloned().collect();
            if !visited_urls.is_empty() {
                let mut changed = false;
                let ids: Vec<NodeId> = st.doc.descendants(st.doc.root);
                for nid in ids {
                    let href_opt = st
                        .doc
                        .element(nid)
                        .filter(|e| e.is_html() && e.tag_is("a") && !e.visited)
                        .and_then(|e| e.get_attr("href").map(|s| s.to_string()));
                    if let Some(href) = href_opt {
                        let resolved = self.resolve_subresource_url(&href).unwrap_or_else(|| href.clone());
                        if visited_urls.contains(&href) || visited_urls.contains(&resolved) {
                            if let Some(el) = st.doc.element_mut(nid) {
                                el.visited = true;
                                changed = true;
                            }
                        }
                    }
                }
                if changed {
                    st.dirty_style = true;
                }
            }
            st.ensure_layout();
            let mut out = Vec::new();
            for ff in &st.styles.font_faces {
                if let Some(ref fam) = ff.font_family {
                    for src in &ff.src {
                        if let Some(resolved) = self.resolve_subresource_url(src) {
                            if !self.failed_images.contains(&resolved) {
                                out.push((fam.clone(), resolved));
                            }
                        }
                    }
                }
            }
            out
        };
        for (fam, url) in font_reqs {
            self.failed_images.insert(url.clone());
            if let Ok(resp) = self.client.get(&url) {
                if resp.status < 400 {
                    let _ = self.fonts.register_web_font(&fam, &resp.body);
                }
            }
        }
        self.clamp_scroll();
    }

    pub fn render_to_framebuffer(&mut self, fb: &mut Framebuffer) {
        self.fetch_missing_images();
        self.recompute_if_needed();
        let st = self.js_env.dom.borrow();
        paint_layout(
            &st.doc,
            &st.layout,
            fb,
            &self.fonts,
            &self.images,
            &st.canvases,
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
        let mut st = self.js_env.dom.borrow_mut();
        st.scroll_x = self.scroll_x;
        st.scroll_y = self.scroll_y;
    }

    fn clamp_scroll(&mut self) {
        let (max_x, max_y) = {
            let st = self.js_env.dom.borrow();
            (
                (st.layout.content_width - self.viewport_w).max(0.0),
                (st.layout.content_height - self.viewport_h).max(0.0),
            )
        };
        self.scroll_x = self.scroll_x.clamp(0.0, max_x);
        self.scroll_y = self.scroll_y.clamp(0.0, max_y);
    }

    pub fn on_mouse_move(&mut self, vx: f32, vy: f32) -> bool {
        let hit = {
            let st = self.js_env.dom.borrow();
            st.layout.hit_test(vx + self.scroll_x, vy + self.scroll_y)
        };
        if hit == self.hovered_node {
            return false;
        }
        {
            let mut st = self.js_env.dom.borrow_mut();
            if let Some(prev) = self.hovered_node {
                if let Some(el) = st.doc.element_mut(prev) {
                    el.hovered = false;
                }
            }
            self.hovered_node = hit;
            self.hovered_href = hit.and_then(|nid| find_ancestor_href(&st.doc, nid));
            if let Some(cur) = hit {
                if let Some(el) = st.doc.element_mut(cur) {
                    el.hovered = true;
                }
            }
            st.dirty_style = true;
        }
        if let Some(cur) = hit {
            let _ = self.js_env.dispatch_dom_event(Some(cur), "mouseover", true, true);
            self.sync_console_from_js();
        }
        true
    }

    pub fn hovered_cursor(&self) -> peedle_css::Cursor {
        if self.hovered_href.is_some() {
            return peedle_css::Cursor::Pointer;
        }
        let Some(nid) = self.hovered_node else {
            return peedle_css::Cursor::Default;
        };
        let st = self.js_env.dom.borrow();
        if let Some(cs) = st.styles.get(nid) {
            if cs.cursor != peedle_css::Cursor::Auto {
                return cs.cursor;
            }
        }
        if let Some(el) = st.doc.element(nid) {
            if el.is_html() {
                if el.tag_is("button") || el.tag_is("summary") || el.tag_is("select") {
                    return peedle_css::Cursor::Pointer;
                }
                if el.tag_is("input") || el.tag_is("textarea") {
                    let ty = el.get_attr("type").unwrap_or("text").to_ascii_lowercase();
                    if matches!(ty.as_str(), "submit" | "button" | "reset" | "checkbox" | "radio") {
                        return peedle_css::Cursor::Pointer;
                    }
                    return peedle_css::Cursor::Text;
                }
            }
        }
        peedle_css::Cursor::Default
    }

    pub fn has_focused_input(&self) -> bool {
        self.js_env.dom.borrow().focused_node.is_some()
    }

    pub fn href_at(&self, vx: f32, vy: f32) -> Option<String> {
        let href = {
            let st = self.js_env.dom.borrow();
            let nid = st.layout.hit_test(vx + self.scroll_x, vy + self.scroll_y)?;
            find_ancestor_href(&st.doc, nid)?
        };
        if href.starts_with("javascript:") {
            return None;
        }
        if href.starts_with("peedle://") || href.starts_with("about:") {
            Some(href)
        } else {
            self.resolve_subresource_url(&href)
        }
    }

    pub fn on_click(&mut self, vx: f32, vy: f32) -> Option<String> {
        let hit = {
            let st = self.js_env.dom.borrow();
            st.layout.hit_test(vx + self.scroll_x, vy + self.scroll_y)
        };
        {
            let mut st = self.js_env.dom.borrow_mut();
            if let Some(prev) = st.focused_node {
                if let Some(el) = st.doc.element_mut(prev) {
                    el.focused = false;
                }
            }
            st.focused_node = None;
        }

        let nid = hit?;

        let _ = self.js_env.dispatch_dom_event(Some(nid), "mousedown", true, true);
        let _ = self.js_env.dispatch_dom_event(Some(nid), "mouseup", true, true);
        let allow_default = self
            .js_env
            .dispatch_dom_event(Some(nid), "click", true, true)
            .unwrap_or(true);
        self.sync_console_from_js();
        self.fetch_missing_images();

        let pending = self.js_env.dom.borrow_mut().pending_navigation.take();
        if let Some(pending) = pending {
            if let Some(dest) = self.resolve_subresource_url(&pending) {
                let _ = self.navigate(&dest);
                return Some(dest);
            }
        }

        if !allow_default {
            self.recompute_if_needed();
            return None;
        }

        let input_action = {
            let mut st = self.js_env.dom.borrow_mut();
            let mut act = 0u8;
            if let Some(el) = st.doc.element_mut(nid) {
                if el.is_html() && el.tag_is("input") {
                    let itype = el.get_attr("type").unwrap_or("text").to_ascii_lowercase();
                    if itype == "checkbox" {
                        let cur = el.is_checked();
                        el.checked = Some(!cur);
                        st.mark_dirty();
                        act = 1;
                    } else if itype == "radio" {
                        el.checked = Some(true);
                        st.mark_dirty();
                        act = 1;
                    } else if itype == "submit" {
                        act = 3;
                    } else {
                        el.focused = true;
                        st.focused_node = Some(nid);
                        st.dirty_style = true;
                        act = 2;
                    }
                } else if el.is_html() && el.tag_is("textarea") {
                    el.focused = true;
                    st.focused_node = Some(nid);
                    st.dirty_style = true;
                    act = 2;
                }
            }
            if act == 0 {
                if let Some(btn_id) = find_ancestor_tag(&st.doc, nid, "button") {
                    if let Some(bel) = st.doc.element(btn_id) {
                        let btype = bel.get_attr("type").unwrap_or("submit").to_ascii_lowercase();
                        if btype == "submit" {
                            act = 3;
                        }
                    }
                }
            }
            if act == 0 {
                if let Some(details_id) = find_ancestor_details_from_summary(&st.doc, nid) {
                    if let Some(det) = st.doc.element_mut(details_id) {
                        if det.has_attr("open") {
                            det.remove_attr("open");
                        } else {
                            det.set_attr("open", "");
                        }
                        st.mark_dirty();
                        act = 4;
                    }
                }
            }
            act
        };

        if input_action == 1 {
            let _ = self.js_env.dispatch_dom_event(Some(nid), "input", true, false);
            let _ = self.js_env.dispatch_dom_event(Some(nid), "change", true, false);
            self.sync_console_from_js();
            self.recompute_if_needed();
            return None;
        }
        if input_action == 2 {
            let _ = self.js_env.dispatch_dom_event(Some(nid), "focus", false, false);
            self.sync_console_from_js();
            self.recompute_if_needed();
            return None;
        }
        if input_action == 3 {
            let form_opt = {
                let st = self.js_env.dom.borrow();
                find_ancestor_tag(&st.doc, nid, "form")
            };
            if let Some(form_id) = form_opt {
                if let Some(dest) = self.submit_form(form_id) {
                    return Some(dest);
                }
            }
            self.recompute_if_needed();
            return None;
        }
        if input_action == 4 {
            self.recompute_if_needed();
            return None;
        }

        let href_opt = {
            let st = self.js_env.dom.borrow();
            find_ancestor_href(&st.doc, nid)
        };
        if let Some(href) = href_opt {
            if let Some(js_code) = href.strip_prefix("javascript:") {
                let _ = self.eval_js(js_code);
                return None;
            }
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
        self.recompute_if_needed();
        None
    }

    pub fn on_text_input(&mut self, ch: char) -> bool {
        let (fid_opt, submit_form_id) = {
            let mut st = self.js_env.dom.borrow_mut();
            let Some(fid) = st.focused_node else {
                return false;
            };
            let is_input_enter = ch == '\n'
                && st.doc.element(fid).map_or(false, |e| e.is_html() && e.tag_is("input"));
            if is_input_enter {
                let form_id = find_ancestor_tag(&st.doc, fid, "form");
                (Some(fid), form_id)
            } else if let Some(el) = st.doc.element_mut(fid) {
                let mut val = el.current_value();
                if ch == '\u{0008}' {
                    val.pop();
                } else if !ch.is_control() || (ch == '\n' && el.tag_is("textarea")) {
                    val.push(ch);
                }
                el.input_value = Some(val);
                st.dirty_layout = true;
                (Some(fid), None)
            } else {
                (None, None)
            }
        };
        if let Some(form_id) = submit_form_id {
            let _ = self.submit_form(form_id);
            return true;
        }
        if let Some(fid) = fid_opt {
            let _ = self.js_env.dispatch_dom_event(Some(fid), "input", true, false);
            self.sync_console_from_js();
            self.recompute_if_needed();
            return true;
        }
        false
    }

    pub fn submit_form(&mut self, form_id: NodeId) -> Option<String> {
        let allow = self
            .js_env
            .dispatch_dom_event(Some(form_id), "submit", true, true)
            .unwrap_or(true);
        self.sync_console_from_js();
        if !allow {
            self.recompute_if_needed();
            return None;
        }
        let (action, method, pairs) = {
            let st = self.js_env.dom.borrow();
            let Some(form_el) = st.doc.element(form_id) else {
                return None;
            };
            let action = form_el.get_attr("action").unwrap_or("").to_string();
            let method = form_el
                .get_attr("method")
                .unwrap_or("get")
                .to_ascii_lowercase();
            let mut pairs: Vec<(String, String)> = Vec::new();
            for desc in st.doc.descendants(form_id) {
                let Some(el) = st.doc.element(desc) else {
                    continue;
                };
                if !el.is_html() || el.is_disabled() {
                    continue;
                }
                let Some(name) = el.get_attr("name") else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                if el.tag_is("input") {
                    let ty = el.get_attr("type").unwrap_or("text").to_ascii_lowercase();
                    if matches!(ty.as_str(), "submit" | "button" | "reset" | "image" | "file") {
                        continue;
                    }
                    if matches!(ty.as_str(), "checkbox" | "radio") && !el.is_checked() {
                        continue;
                    }
                    pairs.push((name.to_string(), el.current_value()));
                } else if el.tag_is("textarea") {
                    pairs.push((name.to_string(), el.current_value()));
                } else if el.tag_is("select") {
                    let mut chosen = None;
                    for opt_id in st.doc.descendants(desc) {
                        if let Some(opt) = st.doc.element(opt_id) {
                            if opt.tag_is("option") {
                                let v = opt
                                    .get_attr("value")
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| st.doc.text_content(opt_id).trim().to_string());
                                if opt.has_attr("selected") {
                                    chosen = Some(v);
                                    break;
                                }
                                if chosen.is_none() {
                                    chosen = Some(v);
                                }
                            }
                        }
                    }
                    if let Some(v) = chosen {
                        pairs.push((name.to_string(), v));
                    }
                }
            }
            (action, method, pairs)
        };

        let encoded = encode_form_urlencoded(&pairs);
        let base_target = if action.is_empty() {
            self.url.clone()
        } else {
            self.resolve_subresource_url(&action).unwrap_or(action)
        };
        if method == "post" {
            let Ok(parsed_url) = Url::parse(&base_target) else {
                return None;
            };
            let req = peedle_net::HttpRequest {
                method: peedle_net::HttpMethod::Post,
                url: parsed_url,
                headers: vec![(
                    "Content-Type".to_string(),
                    "application/x-www-form-urlencoded".to_string(),
                )],
                body: Some(encoded.into_bytes()),
                max_redirects: 10,
            };
            if let Ok(resp) = self.client.send(req) {
                let final_url = resp.url.href();
                self.url = final_url.clone();
                self.base_url = Some(resp.url.clone());
                self.load_html_string(&resp.text(), &final_url);
                self.push_history_entry();
                return Some(final_url);
            }
            return None;
        }
        let dest = if encoded.is_empty() {
            base_target
        } else if base_target.contains('?') {
            format!("{}&{}", base_target, encoded)
        } else {
            format!("{}?{}", base_target, encoded)
        };
        let _ = self.navigate(&dest);
        Some(dest)
    }
}

fn find_ancestor_tag(doc: &Document, start: NodeId, tag: &str) -> Option<NodeId> {
    let mut cur = Some(start);
    while let Some(id) = cur {
        if let Some(el) = doc.element(id) {
            if el.is_html() && el.tag_is(tag) {
                return Some(id);
            }
        }
        cur = doc.node(id).parent;
    }
    None
}

fn find_ancestor_details_from_summary(doc: &Document, start: NodeId) -> Option<NodeId> {
    let summary_id = find_ancestor_tag(doc, start, "summary")?;
    let parent = doc.node(summary_id).parent?;
    if doc.element(parent).map_or(false, |e| e.is_html() && e.tag_is("details")) {
        Some(parent)
    } else {
        None
    }
}

fn encode_form_urlencoded(pairs: &[(String, String)]) -> String {
    let enc = |s: &str| -> String {
        let mut out = String::new();
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => {
                    out.push(b as char);
                }
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{:02X}", b)),
            }
        }
        out
    };
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn find_ancestor_href(doc: &Document, start: NodeId) -> Option<String> {
    let mut cur = Some(start);
    while let Some(id) = cur {
        if let Some(el) = doc.element(id) {
            if el.is_html() && el.tag_is("a") {
                if let Some(href) = el.get_attr("href") {
                    return Some(href.to_string());
                }
            }
        }
        cur = doc.node(id).parent;
    }
    None
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
    if trimmed.starts_with('/') {
        return format!("file://{}", trimmed);
    }
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return format!("file:///{}", trimmed.replace('\\', "/"));
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
