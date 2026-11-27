use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::Instant;

use peedle_css::{parse_stylesheet, LayerBlockRule, Rule, Stylesheet, Viewport};
use peedle_dom::{BrowserJsEnv, SharedOriginStorage};
use peedle_html::{parse_html, Document, NodeId};
use peedle_net::HttpClient;
use peedle_render::{
    paint_layout, paint_layout_scaled, DecodedImage, FontSystem, Framebuffer, ImageCache, RgbaColor,
};
use peedle_url::Url;

use crate::internal_pages::{
    render_history_html_themed, render_internal_page_themed, render_view_source_html,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchEngine {
    DuckDuckGo,
    Google,
    Brave,
    Bing,
}

impl Default for SearchEngine {
    fn default() -> Self {
        SearchEngine::DuckDuckGo
    }
}

impl SearchEngine {
    pub fn as_id(self) -> &'static str {
        match self {
            SearchEngine::DuckDuckGo => "duckduckgo",
            SearchEngine::Google => "google",
            SearchEngine::Brave => "brave",
            SearchEngine::Bing => "bing",
        }
    }

    pub fn from_id(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "google" => SearchEngine::Google,
            "brave" => SearchEngine::Brave,
            "bing" => SearchEngine::Bing,
            _ => SearchEngine::DuckDuckGo,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SearchEngine::DuckDuckGo => "DuckDuckGo",
            SearchEngine::Google => "Google",
            SearchEngine::Brave => "Brave",
            SearchEngine::Bing => "Bing",
        }
    }

    pub fn form_action(self) -> &'static str {
        match self {
            SearchEngine::DuckDuckGo => "https://html.duckduckgo.com/html/",
            SearchEngine::Google => "https://www.google.com/search",
            SearchEngine::Brave => "https://search.brave.com/search",
            SearchEngine::Bing => "https://www.bing.com/search",
        }
    }

    pub fn query_param(self) -> &'static str {
        "q"
    }

    pub fn search_url(self, query: &str) -> String {
        let encoded = peedle_url::percent::utf8_percent_encode(
            query,
            peedle_url::percent::EncodeSet::FormUrlEncoded,
        );
        match self {
            SearchEngine::DuckDuckGo => format!("https://html.duckduckgo.com/html/?q={}", encoded),
            SearchEngine::Google => format!("https://www.google.com/search?q={}", encoded),
            SearchEngine::Brave => format!("https://search.brave.com/search?q={}", encoded),
            SearchEngine::Bing => format!("https://www.bing.com/search?q={}", encoded),
        }
    }
}

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

#[derive(Clone, Debug)]
pub struct DownloadPayload {
    pub filename: String,
    pub url: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub enum ClickAction {
    None,
    Navigated(String),
    OpenNewTab(String),
    OpenSelect {
        node_id: NodeId,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        options: Vec<String>,
        selected: usize,
    },
    Download(DownloadPayload),
}

pub enum PreparedNavigation {
    Html {
        final_url: String,
        base_url: Option<Url>,
        html: String,
        raw_source: String,
        fragment: Option<String>,
        network_log: Vec<NetworkEntry>,
        predecoded_images: Vec<(String, DecodedImage)>,
        favicon: Option<DecodedImage>,
    },
    Download(DownloadPayload),
    Internal {
        url: String,
    },
}

pub fn fetch_navigation_background(
    client: &HttpClient,
    raw_input: &str,
    search_engine: SearchEngine,
    dark_mode: bool,
) -> PreparedNavigation {
    let target = normalize_navigation_input_with_engine(raw_input, search_engine);
    if target.starts_with("peedle://") || target.starts_with("about:") {
        return PreparedNavigation::Internal { url: target };
    }
    if let Some(src_url) = target.strip_prefix("view-source:") {
        let actual = normalize_navigation_input_with_engine(src_url, search_engine);
        let raw_text = client
            .get(&actual)
            .map(|r| r.text())
            .unwrap_or_else(|e| format!("Failed to fetch source: {}", e));
        let html = render_view_source_html(&actual, &raw_text, dark_mode);
        return PreparedNavigation::Html {
            final_url: target,
            base_url: Url::parse(&actual).ok(),
            html,
            raw_source: raw_text,
            fragment: None,
            network_log: Vec::new(),
            predecoded_images: Vec::new(),
            favicon: None,
        };
    }

    let start = Instant::now();
    let resp = match client.get(&target) {
        Ok(r) => r,
        Err(e) => {
            let err_html = build_error_html(&target, &e.to_string(), dark_mode);
            return PreparedNavigation::Html {
                final_url: target.clone(),
                base_url: Url::parse(&target).ok(),
                html: err_html.clone(),
                raw_source: err_html,
                fragment: None,
                network_log: Vec::new(),
                predecoded_images: Vec::new(),
                favicon: None,
            };
        }
    };

    let elapsed = start.elapsed().as_millis() as u64;
    let final_url = resp.url.href();
    let ct = resp.content_type().to_ascii_lowercase();
    let disp = resp
        .header("content-disposition")
        .unwrap_or("")
        .to_ascii_lowercase();

    if is_downloadable_response(&ct, &disp, &resp.url) {
        let filename = extract_download_filename(&disp, &resp.url);
        return PreparedNavigation::Download(DownloadPayload {
            filename,
            url: final_url,
            mime: resp.content_type().to_string(),
            bytes: resp.body,
        });
    }

    let mut net_log = vec![NetworkEntry {
        method: "GET".to_string(),
        url: final_url.clone(),
        status: resp.status,
        content_type: resp.content_type().to_string(),
        bytes: resp.body.len(),
        duration_ms: elapsed,
    }];

    let mut predecoded_images = Vec::new();
    let (html, raw_source) = if ct.contains("image/") {
        if let Ok(decoded) = DecodedImage::decode(&resp.body) {
            predecoded_images.push((final_url.clone(), decoded));
        }
        let h = format!(
            "<!DOCTYPE html><html><body style=\"margin:0;background:#111;display:flex;justify-content:center;align-items:center;min-height:100vh;\"><img src=\"{}\"></body></html>",
            escape_text(&final_url)
        );
        (h.clone(), h)
    } else if ct.contains("text/plain") || ct.contains("application/json") {
        let raw = resp.text();
        let h = format!(
            "<!DOCTYPE html><html><body style=\"margin:16px;font-family:monospace;white-space:pre-wrap;\">{}</body></html>",
            escape_text(&raw)
        );
        (h, raw)
    } else {
        let raw = resp.text();
        (raw.clone(), raw)
    };

    let base_url = Some(resp.url.clone());
    let fragment = resp.url.fragment.clone().filter(|f| !f.is_empty());

    let doc = parse_html(&html);
    let mut warm_urls: Vec<String> = Vec::new();
    let mut img_urls: Vec<(String, String)> = Vec::new();
    let mut seen_imgs: HashSet<String> = HashSet::new();
    let mut favicon_candidates: Vec<String> = Vec::new();

    for nid in doc.descendants(doc.root) {
        let Some(el) = doc.element(nid) else {
            continue;
        };
        if !el.is_html() {
            continue;
        }
        if el.tag_is("link") {
            let rel = el.get_attr("rel").unwrap_or("").to_ascii_lowercase();
            if let Some(href) = el.get_attr("href") {
                if let Some(resolved) = resolve_url_against(&base_url, href) {
                    if rel.split_ascii_whitespace().any(|t| t == "stylesheet") {
                        warm_urls.push(resolved);
                    } else if rel.contains("icon") && favicon_candidates.len() < 3 {
                        favicon_candidates.push(resolved);
                    }
                }
            }
        } else if el.tag_is("script") {
            if let Some(src) = el.get_attr("src") {
                if let Some(resolved) = resolve_url_against(&base_url, src) {
                    warm_urls.push(resolved);
                }
            }
        } else if el.tag_is("img") {
            if let Some(raw_src) = extract_img_source(&doc, nid, el) {
                if seen_imgs.insert(raw_src.clone()) {
                    if let Some(resolved) = resolve_url_against(&base_url, &raw_src) {
                        img_urls.push((raw_src, resolved));
                    }
                }
            }
        }
    }

    if let Some(ref base) = base_url {
        if let Ok(root_ico) = base.join("/favicon.ico") {
            favicon_candidates.push(root_ico.href());
        }
    }

    let warm_queue = Mutex::new(VecDeque::from(warm_urls.into_iter().take(160).collect::<Vec<_>>()));
    let img_queue = Mutex::new(VecDeque::from(img_urls.into_iter().take(72).collect::<Vec<_>>()));
    let decoded_out = Mutex::new(Vec::new());
    let log_out = Mutex::new(Vec::new());
    let fav_out: Mutex<Option<DecodedImage>> = Mutex::new(None);

    std::thread::scope(|s| {
        for _ in 0..6 {
            s.spawn(|| loop {
                if let Some(u) = warm_queue.lock().unwrap().pop_front() {
                    let _ = client.get_cached(&u);
                    continue;
                }
                if let Some((raw_src, resolved)) = img_queue.lock().unwrap().pop_front() {
                    let t0 = Instant::now();
                    if let Ok(r) = client.get_cached(&resolved) {
                        let entry = NetworkEntry {
                            method: "GET".to_string(),
                            url: resolved,
                            status: r.status,
                            content_type: r.content_type().to_string(),
                            bytes: r.body.len(),
                            duration_ms: t0.elapsed().as_millis() as u64,
                        };
                        let dec = if r.status < 400 {
                            DecodedImage::decode(&r.body).ok()
                        } else {
                            None
                        };
                        log_out.lock().unwrap().push(entry);
                        if let Some(img) = dec {
                            decoded_out.lock().unwrap().push((raw_src, img));
                        }
                    }
                    continue;
                }
                break;
            });
        }
        s.spawn(|| {
            for cand in &favicon_candidates {
                if let Ok(r) = client.get_cached(cand) {
                    if r.status < 400 && !r.body.is_empty() {
                        if let Ok(img) = DecodedImage::decode(&r.body) {
                            *fav_out.lock().unwrap() = Some(img);
                            break;
                        }
                    }
                }
            }
        });
    });

    predecoded_images.extend(decoded_out.into_inner().unwrap());
    net_log.extend(log_out.into_inner().unwrap());
    let favicon = fav_out.into_inner().unwrap();

    PreparedNavigation::Html {
        final_url,
        base_url,
        html,
        raw_source,
        fragment,
        network_log: net_log,
        predecoded_images,
        favicon,
    }
}

pub struct Page {
    pub url: String,
    pub base_url: Option<Url>,
    pub js_env: BrowserJsEnv,
    pub images: ImageCache,
    pub failed_images: HashSet<String>,
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
    pub origin_storage: Option<SharedOriginStorage>,
    pub fonts: FontSystem,
    pub favicon: Option<DecodedImage>,
    pub source_html: String,
    pub dark_mode: bool,
    pub search_engine: SearchEngine,
    pub scripts_enabled: bool,
    pub selection_start: Option<(f32, f32)>,
    pub selection_end: Option<(f32, f32)>,
}

impl Page {
    pub fn new(viewport_w: f32, viewport_h: f32) -> Self {
        Self::new_with_client(
            viewport_w,
            viewport_h,
            HttpClient::new(),
            None,
            false,
            SearchEngine::DuckDuckGo,
        )
    }

    pub fn new_with_client(
        viewport_w: f32,
        viewport_h: f32,
        client: HttpClient,
        origin_storage: Option<SharedOriginStorage>,
        dark_mode: bool,
        search_engine: SearchEngine,
    ) -> Self {
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
            failed_images: HashSet::new(),
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
            origin_storage,
            fonts: FontSystem::load_system_fonts(),
            favicon: None,
            source_html: String::new(),
            dark_mode,
            search_engine,
            scripts_enabled: true,
            selection_start: None,
            selection_end: None,
        };
        let _ = page.navigate("peedle://newtab");
        page
    }

    pub fn set_device_pixel_ratio(&mut self, dpr: f32) {
        self.js_env.dom.borrow_mut().device_pixel_ratio = dpr.max(1.0);
    }

    pub fn has_pending_timers(&self) -> bool {
        self.js_env.has_pending_timers()
    }

    pub fn drain_pending_new_tabs(&mut self) -> Vec<String> {
        let mut st = self.js_env.dom.borrow_mut();
        std::mem::take(&mut st.pending_new_tabs)
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
        let target = normalize_navigation_input_with_engine(raw_input, self.search_engine);
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

    pub fn peek_back_url(&self) -> Option<String> {
        if self.can_go_back() {
            Some(self.history[self.history_index - 1].url.clone())
        } else {
            None
        }
    }

    pub fn peek_forward_url(&self) -> Option<String> {
        if self.can_go_forward() {
            Some(self.history[self.history_index + 1].url.clone())
        } else {
            None
        }
    }

    pub fn step_back_index(&mut self) {
        if self.can_go_back() {
            self.history_index -= 1;
        }
    }

    pub fn step_forward_index(&mut self) {
        if self.can_go_forward() {
            self.history_index += 1;
        }
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

    pub fn load_custom_internal(&mut self, url: &str, html: &str, push_history: bool) {
        self.network_log.clear();
        self.console_logs.clear();
        self.failed_images.clear();
        self.favicon = None;
        self.scroll_x = 0.0;
        self.scroll_y = 0.0;
        self.hovered_node = None;
        self.hovered_href = None;
        self.active_node = None;
        self.clear_selection();
        self.url = url.to_string();
        self.base_url = None;
        self.source_html = html.to_string();
        self.load_html_string(html, url);
        if push_history {
            self.push_history_entry();
        }
    }

    pub fn commit_prepared_navigation(
        &mut self,
        prep: PreparedNavigation,
        push_history: bool,
    ) -> Option<DownloadPayload> {
        match prep {
            PreparedNavigation::Download(dl) => Some(dl),
            PreparedNavigation::Internal { url } => {
                let _ = self.load_url_internal(&url, push_history);
                None
            }
            PreparedNavigation::Html {
                final_url,
                base_url,
                html,
                raw_source,
                fragment,
                network_log,
                predecoded_images,
                favicon,
            } => {
                self.network_log = network_log;
                self.console_logs.clear();
                self.failed_images.clear();
                self.scroll_x = 0.0;
                self.scroll_y = 0.0;
                self.hovered_node = None;
                self.hovered_href = None;
                self.active_node = None;
                self.clear_selection();
                self.url = final_url.clone();
                self.base_url = base_url;
                self.favicon = favicon;
                self.source_html = raw_source;
                for (img_url, decoded) in predecoded_images {
                    self.images.insert(img_url, decoded);
                }
                self.load_html_string(&html, &final_url);
                if let Some(frag) = fragment {
                    self.scroll_to_fragment(&frag);
                }
                if push_history {
                    self.push_history_entry();
                }
                None
            }
        }
    }

    fn load_url_internal(&mut self, target: &str, push_history: bool) -> Result<(), String> {
        self.network_log.clear();
        self.console_logs.clear();
        self.failed_images.clear();
        self.favicon = None;
        self.scroll_x = 0.0;
        self.scroll_y = 0.0;
        self.hovered_node = None;
        self.hovered_href = None;
        self.active_node = None;
        self.clear_selection();

        if target == "peedle://history" {
            let pairs: Vec<(String, String)> = self
                .history
                .iter()
                .map(|h| (h.url.clone(), h.title.clone()))
                .collect();
            let html = render_history_html_themed(&pairs, self.dark_mode);
            self.url = target.to_string();
            self.base_url = None;
            self.source_html = html.clone();
            self.load_html_string(&html, target);
            if push_history {
                self.push_history_entry();
            }
            return Ok(());
        }

        if let Some((_title, html)) =
            render_internal_page_themed(target, self.dark_mode, self.search_engine)
        {
            self.url = target.to_string();
            self.base_url = None;
            self.source_html = html.clone();
            self.load_html_string(&html, target);
            if push_history {
                self.push_history_entry();
            }
            return Ok(());
        }

        if let Some(src_url) = target.strip_prefix("view-source:") {
            let actual = normalize_navigation_input_with_engine(src_url, self.search_engine);
            let raw_text = self
                .client
                .get(&actual)
                .map(|r| r.text())
                .unwrap_or_else(|e| format!("Failed to fetch source: {}", e));
            let html = render_view_source_html(&actual, &raw_text, self.dark_mode);
            self.url = target.to_string();
            self.base_url = Url::parse(&actual).ok();
            self.source_html = raw_text;
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
                let err_html = build_error_html(target, &e.to_string(), self.dark_mode);
                self.url = target.to_string();
                self.base_url = Url::parse(target).ok();
                self.source_html = err_html.clone();
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
            let raw = resp.text();
            self.source_html = raw.clone();
            format!(
                "<!DOCTYPE html><html><body style=\"margin:16px;font-family:monospace;white-space:pre-wrap;\">{}</body></html>",
                escape_text(&raw)
            )
        } else {
            let raw = resp.text();
            self.source_html = raw.clone();
            raw
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

        let prev_dpr = self.js_env.dom.borrow().device_pixel_ratio;
        self.js_env = BrowserJsEnv::new(
            doc,
            url.to_string(),
            self.viewport_w,
            self.viewport_h,
            self.client.clone(),
        );
        {
            let mut st = self.js_env.dom.borrow_mut();
            st.dark_mode = self.dark_mode;
            st.device_pixel_ratio = prev_dpr;
            st.origin_storage = self.origin_storage.clone();
            if let Some(ref shared) = self.origin_storage {
                let origin = Url::parse(url)
                    .map(|u| u.origin())
                    .unwrap_or_else(|_| "null".to_string());
                if let Some(existing) = shared.lock().unwrap().get(&origin) {
                    st.local_storage = existing.clone();
                }
            }
        }

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
        resolve_url_against(&self.base_url, raw)
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

        let tasks: Vec<(NodeId, String)> = css_links.into_iter().take(128).collect();
        let queue = Mutex::new(VecDeque::from(tasks));
        let results = Mutex::new(Vec::new());
        let workers = queue.lock().unwrap().len().min(6);
        let client = &self.client;
        let vp = Viewport {
            width: self.viewport_w,
            height: self.viewport_h,
            dark_mode: self.dark_mode,
        };

        if workers > 0 {
            std::thread::scope(|s| {
                for _ in 0..workers {
                    s.spawn(|| loop {
                        let item = queue.lock().unwrap().pop_front();
                        let Some((nid, css_url)) = item else {
                            break;
                        };
                        let start = Instant::now();
                        if let Ok(resp) = client.get_cached(&css_url) {
                            let elapsed = start.elapsed().as_millis() as u64;
                            let ct = resp.content_type().to_string();
                            let bytes = resp.body.len();
                            let status = resp.status;
                            let low = ct.to_ascii_lowercase();
                            let mime_ok = low.is_empty()
                                || low.contains("css")
                                || low.contains("text/plain")
                                || low.contains("octet-stream");
                            let mut extra_logs = Vec::new();
                            let parsed = if status < 400 && mime_ok {
                                let mut sheet = parse_stylesheet(&resp.text());
                                let mut visited = HashSet::new();
                                visited.insert(css_url.clone());
                                expand_stylesheet_resources(
                                    client,
                                    &mut sheet,
                                    &css_url,
                                    &vp,
                                    0,
                                    &mut visited,
                                    &mut extra_logs,
                                );
                                Some(sheet)
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
                            results.lock().unwrap().push((nid, entry, extra_logs, parsed));
                        }
                    });
                }
            });
        }

        let mut sheets = HashMap::new();
        for (nid, entry, extra_logs, parsed) in results.into_inner().unwrap() {
            self.network_log.push(entry);
            self.network_log.extend(extra_logs);
            if let Some(sheet) = parsed {
                sheets.insert(nid, sheet);
            }
        }

        let mut st = self.js_env.dom.borrow_mut();
        st.external_stylesheets = sheets;
        st.dirty_style = true;
    }

    fn fetch_missing_images(&mut self) {
        let mut promoted_srcs: Vec<(NodeId, String)> = Vec::new();
        let img_srcs: Vec<(String, Option<String>)> = {
            let st = self.js_env.dom.borrow();
            let mut out = Vec::new();
            let mut seen = HashSet::new();
            let mut push_src = |raw: String, resolved: Option<String>| {
                if seen.insert(raw.clone()) {
                    out.push((raw, resolved));
                }
            };
            for nid in st.doc.descendants(st.doc.root) {
                if let Some(el) = st.doc.element(nid) {
                    if el.is_html() && el.tag_is("img") {
                        let direct_src = el.get_attr("src").filter(|s| !s.trim().is_empty());
                        let chosen = if let Some(s) = direct_src {
                            Some(s.to_string())
                        } else if let Some(alt_s) = extract_img_source(&st.doc, nid, el) {
                            promoted_srcs.push((nid, alt_s.clone()));
                            Some(alt_s)
                        } else {
                            None
                        };
                        if let Some(src) = chosen {
                            if !self.images.contains(&src) {
                                push_src(src.clone(), self.resolve_subresource_url(&src));
                            } else if !st.image_sizes.contains_key(&src) {
                                push_src(src, None);
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

        if !promoted_srcs.is_empty() {
            let mut st = self.js_env.dom.borrow_mut();
            for (nid, src) in promoted_srcs {
                if let Some(el) = st.doc.element_mut(nid) {
                    el.set_attr("src", &src);
                }
            }
            st.dirty_layout = true;
        }

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
            let queue = Mutex::new(VecDeque::from(to_fetch));
            let results = Mutex::new(Vec::new());
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
                        match client.get_cached(&resolved) {
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
        let font_reqs: Vec<(String, u16, bool, Vec<String>)> = {
            let mut st = self.js_env.dom.borrow_mut();
            st.dark_mode = self.dark_mode;
            let visited_urls: HashSet<String> = st.subdoc_urls.values().cloned().collect();
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
                        let resolved = self
                            .resolve_subresource_url(&href)
                            .unwrap_or_else(|| href.clone());
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
            let mut out: Vec<(String, u16, bool, Vec<String>)> = Vec::new();
            for ff in &st.styles.font_faces {
                if let Some(ref fam) = ff.font_family {
                    let weight = parse_font_face_weight(ff.font_weight.as_deref());
                    let italic = ff.font_style.as_deref().map_or(false, |s| {
                        let low = s.trim().to_ascii_lowercase();
                        low.contains("italic") || low.contains("oblique")
                    });
                    let mut urls = Vec::new();
                    for src in &ff.src {
                        if let Some(resolved) = self.resolve_subresource_url(src) {
                            if !self.failed_images.contains(&resolved) {
                                urls.push(resolved);
                            }
                        }
                    }
                    if !urls.is_empty() {
                        out.push((fam.clone(), weight, italic, urls));
                    }
                }
            }
            out
        };
        let mut to_fetch: Vec<(String, u16, bool, Vec<String>)> = Vec::new();
        for (fam, weight, italic, urls) in font_reqs.into_iter().take(80) {
            for u in &urls {
                self.failed_images.insert(u.clone());
            }
            to_fetch.push((fam, weight, italic, urls));
        }
        let mut registered_any = false;
        if !to_fetch.is_empty() {
            let queue = Mutex::new(VecDeque::from(to_fetch));
            let fetched: Mutex<Vec<(String, u16, bool, Vec<u8>)>> = Mutex::new(Vec::new());
            let workers = queue.lock().unwrap().len().min(6);
            let client = &self.client;
            std::thread::scope(|s| {
                for _ in 0..workers {
                    s.spawn(|| loop {
                        let item = queue.lock().unwrap().pop_front();
                        let Some((fam, weight, italic, urls)) = item else {
                            break;
                        };
                        for url in urls {
                            if let Ok(resp) = client.get_cached(&url) {
                                if resp.status < 400 && !resp.body.is_empty() {
                                    fetched
                                        .lock()
                                        .unwrap()
                                        .push((fam, weight, italic, resp.body));
                                    break;
                                }
                            }
                        }
                    });
                }
            });
            for (fam, weight, italic, body) in fetched.into_inner().unwrap() {
                if let Some(arc) = self
                    .fonts
                    .register_web_font_styled(&fam, weight, italic, &body)
                {
                    if !italic {
                        peedle_layout::register_web_font_arc_weighted(&fam, weight, arc);
                    }
                    registered_any = true;
                }
            }
        }
        if registered_any {
            let mut st = self.js_env.dom.borrow_mut();
            st.dirty_layout = true;
            st.ensure_layout();
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

    pub fn render_to_framebuffer_scaled(&mut self, fb: &mut Framebuffer, scale: f32) {
        self.fetch_missing_images();
        self.recompute_if_needed();
        let sel_rects = self.selection_rects();
        let st = self.js_env.dom.borrow();
        paint_layout_scaled(
            &st.doc,
            &st.layout,
            fb,
            &self.fonts,
            &self.images,
            &st.canvases,
            self.scroll_x,
            self.scroll_y,
            scale,
        );
        let s = scale.max(0.25);
        for (rx, ry, rw, rh) in sel_rects {
            fb.fill_rect(
                (rx - self.scroll_x) * s,
                (ry - self.scroll_y) * s,
                rw * s,
                rh * s,
                RgbaColor::new(56, 189, 248, 85),
            );
        }
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

    pub fn clear_selection(&mut self) {
        self.selection_start = None;
        self.selection_end = None;
    }

    pub fn set_selection_viewport(&mut self, start_vx_vy: (f32, f32), end_vx_vy: (f32, f32)) {
        self.selection_start = Some((start_vx_vy.0 + self.scroll_x, start_vx_vy.1 + self.scroll_y));
        self.selection_end = Some((end_vx_vy.0 + self.scroll_x, end_vx_vy.1 + self.scroll_y));
    }

    pub fn select_word_at(&mut self, vx: f32, vy: f32) {
        let dx = vx + self.scroll_x;
        let dy = vy + self.scroll_y;
        let st = self.js_env.dom.borrow();
        for b in &st.layout.boxes {
            for frag in &b.text_fragments {
                if dx >= frag.x - 2.0
                    && dx <= frag.x + frag.width + 2.0
                    && dy >= frag.y - 2.0
                    && dy <= frag.y + frag.height + 2.0
                {
                    self.selection_start = Some((frag.x, frag.y + frag.height * 0.5));
                    self.selection_end = Some((frag.x + frag.width, frag.y + frag.height * 0.5));
                    return;
                }
            }
        }
    }

    pub fn select_all_text(&mut self) {
        let st = self.js_env.dom.borrow();
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for b in &st.layout.boxes {
            for frag in &b.text_fragments {
                min_y = min_y.min(frag.y);
                max_y = max_y.max(frag.y + frag.height);
            }
        }
        if min_y.is_finite() && max_y.is_finite() {
            self.selection_start = Some((0.0, min_y));
            self.selection_end = Some((1_000_000.0, max_y));
        }
    }

    fn normalized_selection(&self) -> Option<((f32, f32), (f32, f32))> {
        let (a, b) = (self.selection_start?, self.selection_end?);
        if (a.0 - b.0).abs() < 3.0 && (a.1 - b.1).abs() < 3.0 {
            return None;
        }
        if a.1 < b.1 - 6.0 || ((a.1 - b.1).abs() <= 6.0 && a.0 <= b.0) {
            Some((a, b))
        } else {
            Some((b, a))
        }
    }

    pub fn selection_rects(&self) -> Vec<(f32, f32, f32, f32)> {
        let Some(((sx, sy), (ex, ey))) = self.normalized_selection() else {
            return Vec::new();
        };
        let st = self.js_env.dom.borrow();
        let mut rects = Vec::new();
        for b in &st.layout.boxes {
            for frag in &b.text_fragments {
                let fy_mid = frag.y + frag.height * 0.5;
                if fy_mid < sy - frag.height * 0.5 || fy_mid > ey + frag.height * 0.5 {
                    continue;
                }
                let same_start_line = (fy_mid - sy).abs() <= frag.height * 0.75;
                let same_end_line = (fy_mid - ey).abs() <= frag.height * 0.75;
                let left = if same_start_line { sx.max(frag.x) } else { frag.x };
                let right = if same_end_line {
                    ex.min(frag.x + frag.width)
                } else {
                    frag.x + frag.width
                };
                if right > left + 0.5 {
                    rects.push((left, frag.y, right - left, frag.height));
                }
            }
        }
        rects
    }

    pub fn selected_text(&self) -> String {
        let Some(((sx, sy), (ex, ey))) = self.normalized_selection() else {
            return String::new();
        };
        let st = self.js_env.dom.borrow();
        let mut hits: Vec<(f32, f32, String)> = Vec::new();
        for b in &st.layout.boxes {
            for frag in &b.text_fragments {
                let fy_mid = frag.y + frag.height * 0.5;
                if fy_mid < sy - frag.height * 0.5 || fy_mid > ey + frag.height * 0.5 {
                    continue;
                }
                let same_start_line = (fy_mid - sy).abs() <= frag.height * 0.75;
                let same_end_line = (fy_mid - ey).abs() <= frag.height * 0.75;
                let left = if same_start_line { sx.max(frag.x) } else { frag.x };
                let right = if same_end_line {
                    ex.min(frag.x + frag.width)
                } else {
                    frag.x + frag.width
                };
                if right <= left + 0.5 || frag.width <= 0.1 {
                    continue;
                }
                let chars: Vec<char> = frag.text.chars().collect();
                if chars.is_empty() {
                    continue;
                }
                let f0 = ((left - frag.x) / frag.width).clamp(0.0, 1.0);
                let f1 = ((right - frag.x) / frag.width).clamp(0.0, 1.0);
                let i0 = (f0 * chars.len() as f32).floor() as usize;
                let i1 = (f1 * chars.len() as f32).ceil() as usize;
                let slice: String = chars[i0.min(chars.len())..i1.min(chars.len())]
                    .iter()
                    .collect();
                if !slice.is_empty() {
                    hits.push((frag.y, frag.x, slice));
                }
            }
        }
        hits.sort_by(|a, b| {
            if (a.0 - b.0).abs() > 6.0 {
                a.0.total_cmp(&b.0)
            } else {
                a.1.total_cmp(&b.1)
            }
        });
        let mut out = String::new();
        let mut prev_y: Option<f32> = None;
        for (y, _x, txt) in hits {
            if let Some(py) = prev_y {
                if (y - py).abs() > 6.0 {
                    out.push('\n');
                } else if !out.ends_with(' ') && !txt.starts_with(' ') {
                    out.push(' ');
                }
            }
            out.push_str(&txt);
            prev_y = Some(y);
        }
        out
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
                if el.tag_is("button") || el.tag_is("summary") || el.tag_is("select") || el.tag_is("label") {
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

    pub fn focus_next_input(&mut self, reverse: bool) -> bool {
        let mut st = self.js_env.dom.borrow_mut();
        let focusable: Vec<NodeId> = st
            .doc
            .descendants(st.doc.root)
            .into_iter()
            .filter(|&nid| {
                st.doc.element(nid).map_or(false, |el| {
                    if !el.is_html() || el.is_disabled() {
                        return false;
                    }
                    if el.tag_is("textarea") || el.tag_is("select") {
                        return true;
                    }
                    if el.tag_is("input") {
                        let ty = el.get_attr("type").unwrap_or("text").to_ascii_lowercase();
                        return ty != "hidden";
                    }
                    false
                })
            })
            .collect();
        if focusable.is_empty() {
            return false;
        }
        let next = match st.focused_node.and_then(|f| focusable.iter().position(|&x| x == f)) {
            Some(pos) => {
                if reverse {
                    if pos == 0 { focusable.len() - 1 } else { pos - 1 }
                } else {
                    (pos + 1) % focusable.len()
                }
            }
            None => {
                if reverse { focusable.len() - 1 } else { 0 }
            }
        };
        if let Some(prev) = st.focused_node {
            if let Some(el) = st.doc.element_mut(prev) {
                el.focused = false;
            }
        }
        let target = focusable[next];
        if let Some(el) = st.doc.element_mut(target) {
            el.focused = true;
        }
        st.focused_node = Some(target);
        st.dirty_style = true;
        true
    }

    pub fn select_option(&mut self, select_id: NodeId, option_index: usize) {
        {
            let mut st = self.js_env.dom.borrow_mut();
            let opt_ids: Vec<NodeId> = st
                .doc
                .descendants(select_id)
                .into_iter()
                .filter(|&id| st.doc.element(id).map_or(false, |e| e.is_html() && e.tag_is("option")))
                .collect();
            for (i, oid) in opt_ids.into_iter().enumerate() {
                if let Some(el) = st.doc.element_mut(oid) {
                    if i == option_index {
                        el.set_attr("selected", "");
                    } else {
                        el.remove_attr("selected");
                    }
                }
            }
            st.mark_dirty();
        }
        let _ = self.js_env.dispatch_dom_event(Some(select_id), "input", true, false);
        let _ = self.js_env.dispatch_dom_event(Some(select_id), "change", true, false);
        self.sync_console_from_js();
        self.recompute_if_needed();
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
        match self.on_click_ex(vx, vy, true) {
            ClickAction::Navigated(u) | ClickAction::OpenNewTab(u) => Some(u),
            _ => None,
        }
    }

    pub fn on_click_ex(&mut self, vx: f32, vy: f32, execute_nav: bool) -> ClickAction {
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

        let Some(mut nid) = hit else {
            return ClickAction::None;
        };

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
                if execute_nav {
                    let _ = self.navigate(&dest);
                }
                return ClickAction::Navigated(dest);
            }
        }

        if !allow_default {
            self.recompute_if_needed();
            return ClickAction::None;
        }

        let resolved_label_target = {
            let st = self.js_env.dom.borrow();
            resolve_label_control(&st.doc, nid)
        };
        if let Some(ctrl_id) = resolved_label_target {
            nid = ctrl_id;
        }

        let select_popup = {
            let st = self.js_env.dom.borrow();
            if st.doc.element(nid).map_or(false, |e| e.is_html() && e.tag_is("select")) {
                let mut options = Vec::new();
                let mut selected = 0usize;
                for oid in st.doc.descendants(nid) {
                    if let Some(opt) = st.doc.element(oid) {
                        if opt.is_html() && opt.tag_is("option") {
                            let label = st.doc.text_content(oid).trim().to_string();
                            if opt.has_attr("selected") {
                                selected = options.len();
                            }
                            options.push(label);
                        }
                    }
                }
                if !options.is_empty() {
                    let r = st.layout.bounding_rect(nid).unwrap_or_default();
                    Some(ClickAction::OpenSelect {
                        node_id: nid,
                        x: r.x - self.scroll_x,
                        y: r.y - self.scroll_y,
                        w: r.width.max(120.0),
                        h: r.height.max(24.0),
                        options,
                        selected,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(sel) = select_popup {
            return sel;
        }

        let input_action = {
            let mut st = self.js_env.dom.borrow_mut();
            let mut act = 0u8;
            let mut radio_name: Option<String> = None;
            if let Some(el) = st.doc.element_mut(nid) {
                if el.is_html() && el.tag_is("input") {
                    let itype = el.get_attr("type").unwrap_or("text").to_ascii_lowercase();
                    if itype == "checkbox" {
                        let cur = el.is_checked();
                        el.checked = Some(!cur);
                        st.mark_dirty();
                        act = 1;
                    } else if itype == "radio" {
                        radio_name = el.get_attr("name").map(|s| s.to_string());
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
            if let Some(rname) = radio_name {
                if !rname.is_empty() {
                    let all_ids = st.doc.descendants(st.doc.root);
                    for oid in all_ids {
                        if oid == nid {
                            continue;
                        }
                        if let Some(oel) = st.doc.element_mut(oid) {
                            if oel.is_html()
                                && oel.tag_is("input")
                                && oel
                                    .get_attr("type")
                                    .map_or(false, |t| t.eq_ignore_ascii_case("radio"))
                                && oel.get_attr("name") == Some(rname.as_str())
                            {
                                oel.checked = Some(false);
                            }
                        }
                    }
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
            return ClickAction::None;
        }
        if input_action == 2 {
            let _ = self.js_env.dispatch_dom_event(Some(nid), "focus", false, false);
            self.sync_console_from_js();
            self.recompute_if_needed();
            return ClickAction::None;
        }
        if input_action == 3 {
            let form_opt = {
                let st = self.js_env.dom.borrow();
                find_ancestor_tag(&st.doc, nid, "form")
            };
            if let Some(form_id) = form_opt {
                if let Some(dest) = self.submit_form_ex(form_id, execute_nav) {
                    return ClickAction::Navigated(dest);
                }
            }
            self.recompute_if_needed();
            return ClickAction::None;
        }
        if input_action == 4 {
            self.recompute_if_needed();
            return ClickAction::None;
        }

        let (href_opt, open_blank) = {
            let st = self.js_env.dom.borrow();
            find_ancestor_link_info(&st.doc, nid)
        };
        if let Some(href) = href_opt {
            if let Some(js_code) = href.strip_prefix("javascript:") {
                let _ = self.eval_js(js_code);
                return ClickAction::None;
            }
            let resolved = if href.starts_with("peedle://")
                || href.starts_with("about:")
                || href.starts_with("view-source:")
            {
                Some(href)
            } else {
                self.resolve_subresource_url(&href)
            };
            if let Some(dest) = resolved {
                if open_blank {
                    return ClickAction::OpenNewTab(dest);
                }
                if execute_nav {
                    let _ = self.navigate(&dest);
                }
                return ClickAction::Navigated(dest);
            }
        }
        self.recompute_if_needed();
        ClickAction::None
    }

    pub fn on_text_input(&mut self, ch: char) -> bool {
        let (fid_opt, submit_form_id) = {
            let mut st = self.js_env.dom.borrow_mut();
            let Some(fid) = st.focused_node else {
                return false;
            };
            let is_input_enter = ch == '\n'
                && st
                    .doc
                    .element(fid)
                    .map_or(false, |e| e.is_html() && e.tag_is("input"));
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
        self.submit_form_ex(form_id, true)
    }

    pub fn submit_form_ex(&mut self, form_id: NodeId, execute_get_nav: bool) -> Option<String> {
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
                let raw = resp.text();
                self.source_html = raw.clone();
                self.load_html_string(&raw, &final_url);
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
        if execute_get_nav {
            let _ = self.navigate(&dest);
        }
        Some(dest)
    }
}

fn resolve_url_against(base_url: &Option<Url>, raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("javascript:") {
        return None;
    }
    if let Some(ref base) = base_url {
        base.join(trimmed).ok().map(|u| u.href())
    } else {
        Url::parse(trimmed).ok().map(|u| u.href())
    }
}

fn extract_img_source(
    doc: &Document,
    nid: NodeId,
    el: &peedle_html::ElementData,
) -> Option<String> {
    if let Some(src) = el.get_attr("src").filter(|s| !s.trim().is_empty()) {
        return Some(src.trim().to_string());
    }
    if let Some(dsrc) = el
        .get_attr("data-src")
        .or_else(|| el.get_attr("data-lazy-src"))
        .filter(|s| !s.trim().is_empty())
    {
        return Some(dsrc.trim().to_string());
    }
    if let Some(srcset) = el.get_attr("srcset").and_then(first_srcset_url) {
        return Some(srcset);
    }
    if let Some(parent) = doc.node(nid).parent {
        if doc
            .element(parent)
            .map_or(false, |pe| pe.is_html() && pe.tag_is("picture"))
        {
            for child in doc.element_children(parent) {
                if let Some(cel) = doc.element(child) {
                    if cel.is_html() && cel.tag_is("source") {
                        if let Some(u) = cel.get_attr("srcset").and_then(first_srcset_url) {
                            return Some(u);
                        }
                    }
                }
            }
        }
    }
    None
}

fn first_srcset_url(srcset: &str) -> Option<String> {
    for part in srcset.split(',') {
        let candidate = part.trim().split_ascii_whitespace().next().unwrap_or("");
        if !candidate.is_empty() {
            return Some(candidate.to_string());
        }
    }
    None
}

fn is_downloadable_response(content_type: &str, content_disposition: &str, url: &Url) -> bool {
    if content_disposition.contains("attachment") {
        return true;
    }
    let mime = content_type.split(';').next().unwrap_or("").trim();
    if matches!(
        mime,
        "application/octet-stream"
            | "application/zip"
            | "application/x-zip-compressed"
            | "application/gzip"
            | "application/x-gzip"
            | "application/x-7z-compressed"
            | "application/x-rar-compressed"
            | "application/pdf"
            | "application/x-msdownload"
            | "application/vnd.microsoft.portable-executable"
            | "application/x-apple-diskimage"
    ) {
        return true;
    }
    let path = url.pathname().to_ascii_lowercase();
    for ext in [".zip", ".7z", ".tar.gz", ".tgz", ".exe", ".msi", ".dmg", ".pkg", ".deb", ".rpm", ".pdf"] {
        if path.ends_with(ext) && !mime.contains("text/html") {
            return true;
        }
    }
    false
}

fn extract_download_filename(content_disposition: &str, url: &Url) -> String {
    if let Some(idx) = content_disposition.find("filename=") {
        let rest = &content_disposition[idx + 9..];
        let raw = rest
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        if !raw.is_empty() {
            return sanitize_filename(raw);
        }
    }
    let path = url.pathname();
    let seg = path.rsplit('/').next().unwrap_or("").trim();
    if !seg.is_empty() {
        sanitize_filename(seg)
    } else {
        "download.bin".to_string()
    }
}

fn sanitize_filename(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "download.bin".to_string()
    } else {
        trimmed.to_string()
    }
}

fn build_error_html(target: &str, err: &str, dark: bool) -> String {
    let (bg, card, border, fg, muted) = if dark {
        ("#0f1117", "#171c26", "#334155", "#e6edf3", "#cbd5e1")
    } else {
        ("#f8fafc", "#ffffff", "#e2e8f0", "#0f172a", "#475569")
    };
    format!(
        "<!DOCTYPE html><html><head><title>Load Error</title><style>body{{font-family:sans-serif;padding:48px;background:{bg};color:{fg};}}.box{{max-width:600px;margin:0 auto;background:{card};border:1px solid {border};border-radius:12px;padding:24px;}}h1{{margin:0 0 12px 0;font-size:22px;color:#f87171;}}p{{color:{muted};font-family:monospace;}}</style></head><body><div class=\"box\"><h1>Could not load page</h1><p>{u}</p><p>{e}</p></div></body></html>",
        bg = bg,
        card = card,
        border = border,
        fg = fg,
        muted = muted,
        u = escape_text(target),
        e = escape_text(err)
    )
}

fn resolve_label_control(doc: &Document, start: NodeId) -> Option<NodeId> {
    let label_id = find_ancestor_tag(doc, start, "label")?;
    if let Some(for_id) = doc.element(label_id).and_then(|e| e.get_attr("for")) {
        if !for_id.is_empty() {
            return doc.get_element_by_id(for_id);
        }
    }
    for desc in doc.descendants(label_id) {
        if let Some(el) = doc.element(desc) {
            if el.is_html() && (el.tag_is("input") || el.tag_is("textarea") || el.tag_is("select")) {
                return Some(desc);
            }
        }
    }
    None
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
    if doc
        .element(parent)
        .map_or(false, |e| e.is_html() && e.tag_is("details"))
    {
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
    find_ancestor_link_info(doc, start).0
}

fn find_ancestor_link_info(doc: &Document, start: NodeId) -> (Option<String>, bool) {
    let mut cur = Some(start);
    while let Some(id) = cur {
        if let Some(el) = doc.element(id) {
            if el.is_html() && el.tag_is("a") {
                if let Some(href) = el.get_attr("href") {
                    let blank = el
                        .get_attr("target")
                        .map_or(false, |t| t.eq_ignore_ascii_case("_blank"));
                    return (Some(href.to_string()), blank);
                }
            }
        }
        cur = doc.node(id).parent;
    }
    (None, false)
}

pub fn normalize_navigation_input(raw: &str) -> String {
    normalize_navigation_input_with_engine(raw, SearchEngine::DuckDuckGo)
}

pub fn normalize_navigation_input_with_engine(raw: &str, engine: SearchEngine) -> String {
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
        || trimmed.starts_with("view-source:")
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
    if trimmed.starts_with("localhost") || trimmed.starts_with("127.0.0.1") {
        return format!("http://{}", trimmed);
    }
    if trimmed.contains('.') && !trimmed.contains(' ') {
        return format!("https://{}", trimmed);
    }
    engine.search_url(trimmed)
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn parse_font_face_weight(raw: Option<&str>) -> u16 {
    let Some(s) = raw else {
        return 400;
    };
    let trimmed = s.trim().to_ascii_lowercase();
    if trimmed == "bold" {
        return 700;
    }
    if trimmed == "normal" {
        return 400;
    }
    if let Some(first) = trimmed.split_ascii_whitespace().next() {
        if let Ok(n) = first.parse::<u16>() {
            return n.clamp(100, 900);
        }
    }
    400
}

fn expand_stylesheet_resources(
    client: &HttpClient,
    sheet: &mut Stylesheet,
    sheet_url: &str,
    vp: &Viewport,
    depth: usize,
    visited: &mut HashSet<String>,
    logs: &mut Vec<NetworkEntry>,
) {
    let base = Url::parse(sheet_url).ok();
    let old_rules = std::mem::take(&mut sheet.rules);
    sheet.rules = expand_rule_list(client, old_rules, &base, vp, depth, visited, logs);
}

fn expand_rule_list(
    client: &HttpClient,
    rules: Vec<Rule>,
    base: &Option<Url>,
    vp: &Viewport,
    depth: usize,
    visited: &mut HashSet<String>,
    logs: &mut Vec<NetworkEntry>,
) -> Vec<Rule> {
    let mut out = Vec::with_capacity(rules.len());
    for rule in rules {
        match rule {
            Rule::Import(imp) => {
                if depth >= 4 {
                    continue;
                }
                if let Some(ref mq) = imp.media {
                    if !mq.matches(vp) {
                        continue;
                    }
                }
                let Some(resolved) = resolve_url_against(base, &imp.url) else {
                    continue;
                };
                if !visited.insert(resolved.clone()) {
                    continue;
                }
                let start = Instant::now();
                if let Ok(resp) = client.get_cached(&resolved) {
                    let elapsed = start.elapsed().as_millis() as u64;
                    let ct = resp.content_type().to_string();
                    let status = resp.status;
                    let bytes = resp.body.len();
                    logs.push(NetworkEntry {
                        method: "GET".to_string(),
                        url: resolved.clone(),
                        status,
                        content_type: ct.clone(),
                        bytes,
                        duration_ms: elapsed,
                    });
                    let low = ct.to_ascii_lowercase();
                    let mime_ok = low.is_empty()
                        || low.contains("css")
                        || low.contains("text/plain")
                        || low.contains("octet-stream");
                    if status < 400 && mime_ok {
                        let mut imported = parse_stylesheet(&resp.text());
                        expand_stylesheet_resources(
                            client,
                            &mut imported,
                            &resolved,
                            vp,
                            depth + 1,
                            visited,
                            logs,
                        );
                        if let Some(layer_name) = imp.layer {
                            out.push(Rule::LayerBlock(LayerBlockRule {
                                name: layer_name,
                                rules: imported.rules,
                            }));
                        } else {
                            out.extend(imported.rules);
                        }
                    }
                }
            }
            Rule::FontFace(mut ff) => {
                for src in &mut ff.src {
                    if let Some(resolved) = resolve_url_against(base, src) {
                        *src = resolved;
                    }
                }
                out.push(Rule::FontFace(ff));
            }
            Rule::Media { query, rules } => {
                let expanded = expand_rule_list(client, rules, base, vp, depth, visited, logs);
                out.push(Rule::Media {
                    query,
                    rules: expanded,
                });
            }
            Rule::Supports { condition, rules } => {
                let expanded = expand_rule_list(client, rules, base, vp, depth, visited, logs);
                out.push(Rule::Supports {
                    condition,
                    rules: expanded,
                });
            }
            Rule::LayerBlock(mut lb) => {
                lb.rules = expand_rule_list(client, lb.rules, base, vp, depth, visited, logs);
                out.push(Rule::LayerBlock(lb));
            }
            other => out.push(other),
        }
    }
    out
}
