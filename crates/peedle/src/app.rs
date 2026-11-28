use std::io::Write;
use std::num::NonZeroU32;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use peedle_engine::{
    fetch_navigation_background, render_bookmarks_html, render_downloads_html,
    render_history_html_themed, render_newtab_html, render_settings_html, ClickAction,
    DownloadPayload, Page, PreparedNavigation, SearchEngine, SettingsView,
};
use peedle_net::HttpClient;
use peedle_render::{ClipRect, FontFaceKey, FontSystem, Framebuffer, RgbaColor};
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowAttributes, WindowId};

use crate::chrome::{
    content_origin_x, content_viewport_height, content_viewport_width, draw_chrome,
    hit_test_chrome, omnibox_rect_for, split_divider_x, toolbar_top, top_chrome_height, ChromeHit,
    ChromeState, ContextMenuItem, DevToolsTab, DownloadToast, MenuAction, OmniboxSuggestion,
    PopupMenu, SelectPopup, SuggestionKind, TabVisual, SPLIT_DIVIDER_W,
};
use crate::profile::{reveal_in_file_manager, BrowserProfile, SavedBookmark};

pub enum AppEvent {
    NavigationReady {
        tab_id: u64,
        seq: u64,
        push_history: bool,
        prepared: PreparedNavigation,
    },
}

struct BrowserTab {
    id: u64,
    page: Page,
    loading: bool,
    load_seq: u64,
    load_started: Option<Instant>,
    last_media_seq: u64,
    pinned: bool,
    reader_saved_url: Option<String>,
}

pub struct BrowserApp {
    initial_urls: Vec<String>,
    proxy: EventLoopProxy<AppEvent>,
    profile: BrowserProfile,
    client: HttpClient,
    window: Option<Rc<Window>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    tabs: Vec<BrowserTab>,
    next_tab_id: u64,
    closed_tabs: Vec<String>,
    active_tab: usize,
    chrome: ChromeState,
    fonts: FontSystem,
    page_fb: Framebuffer,
    split_fb: Framebuffer,
    full_fb: Framebuffer,
    page_dirty: bool,
    split_dirty: bool,
    split_tab: Option<usize>,
    split_dragging: bool,
    cursor_x: f32,
    cursor_y: f32,
    modifiers: ModifiersState,
    scrollbar_dragging: bool,
    selecting_page_text: bool,
    select_drag_start: (f32, f32),
    last_click_time: Option<Instant>,
    last_click_pos: (f32, f32),
    scroll_vel_y: f32,
    last_tick: Instant,
    phys_w: u32,
    phys_h: u32,
    scale_factor: f32,
    settings_notice: Option<String>,
    window_fullscreen: bool,
}

impl BrowserApp {
    pub fn run(initial_urls: Vec<String>) -> Result<(), String> {
        let event_loop = EventLoop::<AppEvent>::with_user_event()
            .build()
            .map_err(|e| e.to_string())?;
        let proxy = event_loop.create_proxy();
        let profile = BrowserProfile::load();
        let client = HttpClient::with_shared_cookies(profile.cookies.clone());
        client.set_adblock_enabled(profile.settings.adblock_enabled);
        let mut chrome = ChromeState::default();
        chrome.dark_mode = profile.settings.dark_mode;
        chrome.bookmarks_visible = profile.settings.bookmarks_visible;
        chrome.bookmarks = profile.bookmarks.clone();
        chrome.zoom = profile.settings.default_zoom.clamp(0.5, 2.0);
        chrome.has_downloads = !profile.downloads.is_empty();
        chrome.shields_enabled = profile.settings.adblock_enabled;
        chrome.vertical_tabs = profile.settings.vertical_tabs;

        let mut app = Self {
            initial_urls,
            proxy,
            profile,
            client,
            window: None,
            surface: None,
            tabs: Vec::new(),
            next_tab_id: 1,
            closed_tabs: Vec::new(),
            active_tab: 0,
            chrome,
            fonts: FontSystem::load_system_fonts(),
            page_fb: Framebuffer::new(1, 1),
            split_fb: Framebuffer::new(1, 1),
            full_fb: Framebuffer::new(1, 1),
            page_dirty: true,
            split_dirty: true,
            split_tab: None,
            split_dragging: false,
            cursor_x: 0.0,
            cursor_y: 0.0,
            modifiers: ModifiersState::empty(),
            scrollbar_dragging: false,
            selecting_page_text: false,
            select_drag_start: (0.0, 0.0),
            last_click_time: None,
            last_click_pos: (0.0, 0.0),
            scroll_vel_y: 0.0,
            last_tick: Instant::now(),
            phys_w: 1280,
            phys_h: 820,
            scale_factor: 1.0,
            settings_notice: None,
            window_fullscreen: false,
        };
        event_loop.run_app(&mut app).map_err(|e| e.to_string())
    }

    fn logical_w(&self) -> f32 {
        (self.phys_w as f32 / self.scale_factor.max(1.0)).max(200.0)
    }

    fn logical_h(&self) -> f32 {
        (self.phys_h as f32 / self.scale_factor.max(1.0)).max(200.0)
    }

    fn active_tab_ref(&self) -> &BrowserTab {
        &self.tabs[self.active_tab]
    }

    fn active_page(&self) -> &Page {
        &self.tabs[self.active_tab].page
    }

    fn active_page_mut(&mut self) -> &mut Page {
        &mut self.tabs[self.active_tab].page
    }

    fn persist_session(&mut self) {
        let urls: Vec<String> = self.tabs.iter().map(|t| t.page.url.clone()).collect();
        self.profile.save_session(urls, self.active_tab);
        self.profile.save_all();
    }

    fn sync_omnibox(&mut self) {
        let url = self.active_page().url.clone();
        self.chrome.omnibox.set_text(&url);
        self.chrome.suggestions.clear();
        self.chrome.suggestion_index = None;
    }

    fn sync_window_title(&self) {
        if let Some(ref win) = self.window {
            let tab = self.active_tab_ref();
            if tab.loading {
                win.set_title("Loading... - Peedle");
            } else {
                win.set_title(&format!("{} - Peedle", tab.page.title()));
            }
        }
    }

    fn refresh_suggestions(&mut self) {
        if !self.chrome.omnibox_focused {
            self.chrome.suggestions.clear();
            self.chrome.suggestion_index = None;
            return;
        }
        let q = self.chrome.omnibox.text.trim();
        let low = q.to_ascii_lowercase();
        let mut out: Vec<OmniboxSuggestion> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for bm in &self.profile.bookmarks {
            if low.is_empty()
                || bm.title.to_ascii_lowercase().contains(&low)
                || bm.url.to_ascii_lowercase().contains(&low)
            {
                if seen.insert(bm.url.clone()) {
                    out.push(OmniboxSuggestion {
                        title: bm.title.clone(),
                        url: bm.url.clone(),
                        kind: SuggestionKind::Bookmark,
                    });
                }
            }
            if out.len() >= 4 {
                break;
            }
        }

        let mut hist_matches: Vec<_> = self
            .profile
            .history
            .iter()
            .filter(|h| {
                !seen.contains(&h.url)
                    && (low.is_empty()
                        || h.url.to_ascii_lowercase().contains(&low)
                        || h.title.to_ascii_lowercase().contains(&low))
            })
            .collect();
        hist_matches.sort_by(|a, b| b.visits.cmp(&a.visits));
        for h in hist_matches {
            if out.len() >= 6 {
                break;
            }
            if seen.insert(h.url.clone()) {
                out.push(OmniboxSuggestion {
                    title: h.title.clone(),
                    url: h.url.clone(),
                    kind: SuggestionKind::History,
                });
            }
        }

        if !q.is_empty() {
            let eng = self.profile.settings.engine();
            out.push(OmniboxSuggestion {
                title: format!("Search {} for \"{}\"", eng.label(), q),
                url: eng.search_url(q),
                kind: SuggestionKind::Search,
            });
        }

        self.chrome.suggestions = out;
        self.chrome.suggestion_index = None;
    }

    fn render_internal_into_tab(&mut self, tab_idx: usize, raw_url: &str, push_history: bool) -> bool {
        if raw_url == "peedle://newtab" || raw_url == "about:newtab" {
            let quick: Vec<(String, String)> = self
                .profile
                .bookmarks
                .iter()
                .take(8)
                .map(|b| (b.title.clone(), b.url.clone()))
                .collect();
            let html = render_newtab_html(
                self.profile.settings.dark_mode,
                self.profile.settings.engine(),
                &quick,
            );
            self.tabs[tab_idx].page.load_custom_internal("peedle://newtab", &html, push_history);
            self.tabs[tab_idx].loading = false;
            self.page_dirty = true;
            return true;
        }
        if raw_url == "peedle://history" {
            let pairs: Vec<(String, String)> = self
                .profile
                .history
                .iter()
                .map(|h| (h.url.clone(), h.title.clone()))
                .collect();
            let html = render_history_html_themed(&pairs, self.profile.settings.dark_mode);
            self.tabs[tab_idx].page.load_custom_internal("peedle://history", &html, push_history);
            self.tabs[tab_idx].loading = false;
            self.page_dirty = true;
            return true;
        }
        if raw_url.starts_with("peedle://bookmarks") {
            if let Some(q) = raw_url.split_once('?').map(|(_, q)| q) {
                if let Some(idx_s) = q.strip_prefix("remove=") {
                    if let Ok(idx) = idx_s.parse::<usize>() {
                        if idx < self.profile.bookmarks.len() {
                            self.profile.bookmarks.remove(idx);
                            self.profile.save_bookmarks();
                            self.chrome.bookmarks = self.profile.bookmarks.clone();
                        }
                    }
                }
            }
            let pairs: Vec<(String, String)> = self
                .profile
                .bookmarks
                .iter()
                .map(|b| (b.title.clone(), b.url.clone()))
                .collect();
            let html = render_bookmarks_html(&pairs, self.profile.settings.dark_mode);
            self.tabs[tab_idx].page.load_custom_internal("peedle://bookmarks", &html, push_history);
            self.tabs[tab_idx].loading = false;
            self.page_dirty = true;
            return true;
        }
        if raw_url.starts_with("peedle://downloads") {
            if let Some(q) = raw_url.split_once('?').map(|(_, q)| q) {
                if q == "clear=1" {
                    self.profile.downloads.clear();
                    self.profile.save_downloads();
                    self.chrome.has_downloads = false;
                } else if let Some(idx_s) = q.strip_prefix("open=") {
                    if let Ok(idx) = idx_s.parse::<usize>() {
                        if let Some(dl) = self.profile.downloads.get(idx) {
                            reveal_in_file_manager(&dl.path);
                        }
                    }
                }
            }
            let views = self.profile.download_views();
            let html = render_downloads_html(&views, self.profile.settings.dark_mode);
            self.tabs[tab_idx].page.load_custom_internal("peedle://downloads", &html, push_history);
            self.tabs[tab_idx].loading = false;
            self.page_dirty = true;
            return true;
        }
        if raw_url.starts_with("peedle://settings") {
            if let Some(q) = raw_url.split_once('?').map(|(_, q)| q) {
                self.apply_settings_query(q);
            }
            let view = SettingsView {
                dark_mode: self.profile.settings.dark_mode,
                search_engine: self.profile.settings.engine(),
                restore_session: self.profile.settings.restore_session,
                bookmarks_visible: self.profile.settings.bookmarks_visible,
                cookie_count: self.profile.cookies.lock().unwrap().len(),
                history_count: self.profile.history.len(),
                bookmark_count: self.profile.bookmarks.len(),
                status_notice: self.settings_notice.take(),
            };
            let html = render_settings_html(&view);
            self.tabs[tab_idx].page.load_custom_internal("peedle://settings", &html, push_history);
            self.tabs[tab_idx].loading = false;
            self.page_dirty = true;
            return true;
        }
        false
    }

    fn apply_settings_query(&mut self, query: &str) {
        for pair in query.split('&') {
            let Some((k, v)) = pair.split_once('=') else {
                continue;
            };
            match k {
                "theme" => {
                    let dark = v != "light";
                    self.profile.settings.dark_mode = dark;
                    self.chrome.dark_mode = dark;
                    for tab in &mut self.tabs {
                        tab.page.dark_mode = dark;
                        tab.page.js_env.dom.borrow_mut().dark_mode = dark;
                        tab.page.js_env.dom.borrow_mut().dirty_style = true;
                    }
                    self.profile.save_settings();
                    self.settings_notice = Some(format!(
                        "Theme switched to {}",
                        if dark { "Dark" } else { "Light" }
                    ));
                }
                "search" => {
                    let eng = SearchEngine::from_id(v);
                    self.profile.settings.search_engine = eng.as_id().to_string();
                    for tab in &mut self.tabs {
                        tab.page.search_engine = eng;
                    }
                    self.profile.save_settings();
                    self.settings_notice = Some(format!("Default search set to {}", eng.label()));
                }
                "restore" => {
                    self.profile.settings.restore_session = v == "1";
                    self.profile.save_settings();
                    self.settings_notice = Some("Startup preference saved".to_string());
                }
                "bookmarks_bar" => {
                    let vis = v == "1";
                    self.profile.settings.bookmarks_visible = vis;
                    self.chrome.bookmarks_visible = vis;
                    self.resize_all_tabs();
                    self.profile.save_settings();
                }
                "action" => match v {
                    "clear_history" => {
                        self.profile.history.clear();
                        self.profile.save_history();
                        self.settings_notice = Some("Browsing history cleared".to_string());
                    }
                    "clear_cookies" => {
                        self.profile.cookies.lock().unwrap().clear();
                        self.profile.save_cookies();
                        self.settings_notice = Some("Cookies cleared".to_string());
                    }
                    "clear_cache" => {
                        self.client.clear_cache();
                        self.settings_notice = Some("HTTP cache cleared".to_string());
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    fn start_navigation_for_tab(&mut self, tab_idx: usize, raw_input: &str, push_history: bool) {
        if tab_idx >= self.tabs.len() {
            return;
        }
        self.chrome.popup_menu = None;
        self.chrome.select_popup = None;
        let trimmed = raw_input.trim();
        if self.render_internal_into_tab(tab_idx, trimmed, push_history) {
            if tab_idx == self.active_tab {
                self.sync_omnibox();
                self.sync_window_title();
            }
            return;
        }
        if trimmed == "peedle://about"
            || trimmed == "about:peedle"
            || trimmed == "peedle://demo"
            || trimmed == "about:blank"
        {
            let _ = self.tabs[tab_idx].page.navigate(trimmed);
            self.tabs[tab_idx].loading = false;
            self.page_dirty = true;
            if tab_idx == self.active_tab {
                self.sync_omnibox();
                self.sync_window_title();
            }
            return;
        }

        let tab = &mut self.tabs[tab_idx];
        tab.load_seq = tab.load_seq.wrapping_add(1);
        tab.loading = true;
        tab.load_started = Some(Instant::now());
        let tab_id = tab.id;
        let seq = tab.load_seq;

        let client = self.client.clone();
        let input = trimmed.to_string();
        let engine = self.profile.settings.engine();
        let dark = self.profile.settings.dark_mode;
        let proxy = self.proxy.clone();

        if tab_idx == self.active_tab {
            self.sync_window_title();
        }

        std::thread::spawn(move || {
            let prepared = fetch_navigation_background(&client, &input, engine, dark);
            let _ = proxy.send_event(AppEvent::NavigationReady {
                tab_id,
                seq,
                push_history,
                prepared,
            });
        });
    }

    fn navigate_active(&mut self, target: &str) {
        let idx = self.active_tab;
        self.start_navigation_for_tab(idx, target, true);
    }

    fn stop_or_reload_active(&mut self) {
        let idx = self.active_tab;
        if self.tabs[idx].loading {
            self.tabs[idx].loading = false;
            self.tabs[idx].load_seq = self.tabs[idx].load_seq.wrapping_add(1);
            self.sync_window_title();
        } else {
            let url = self.tabs[idx].page.url.clone();
            self.start_navigation_for_tab(idx, &url, false);
        }
    }

    fn go_back_active(&mut self) {
        let idx = self.active_tab;
        if let Some(url) = self.tabs[idx].page.peek_back_url() {
            self.tabs[idx].page.step_back_index();
            self.start_navigation_for_tab(idx, &url, false);
        }
    }

    fn go_forward_active(&mut self) {
        let idx = self.active_tab;
        if let Some(url) = self.tabs[idx].page.peek_forward_url() {
            self.tabs[idx].page.step_forward_index();
            self.start_navigation_for_tab(idx, &url, false);
        }
    }

    fn handle_download(&mut self, dl: DownloadPayload) {
        if let Ok(saved) = self.profile.save_download(&dl) {
            self.chrome.has_downloads = true;
            self.chrome.download_toast = Some(DownloadToast {
                filename: saved.filename,
                path: saved.path,
                bytes: saved.bytes,
            });
        }
    }

    fn toggle_current_bookmark(&mut self) {
        let url = self.active_page().url.clone();
        if let Some(pos) = self.profile.bookmarks.iter().position(|b| b.url == url) {
            self.profile.bookmarks.remove(pos);
        } else {
            let title = self.active_page().title();
            self.profile.bookmarks.push(SavedBookmark { title, url });
        }
        self.profile.save_bookmarks();
        self.chrome.bookmarks = self.profile.bookmarks.clone();
    }

    fn run_find(&mut self, delta: i32) {
        let q = self.chrome.find.text.trim().to_ascii_lowercase();
        if q.is_empty() {
            self.chrome.find_match_count = 0;
            self.chrome.find_match_index = 0;
            self.page_dirty = true;
            return;
        }
        let ys: Vec<f32> = {
            let st = self.active_page().js_env.dom.borrow();
            let mut hits = Vec::new();
            for b in &st.layout.boxes {
                for frag in &b.text_fragments {
                    if frag.text.to_ascii_lowercase().contains(&q) {
                        hits.push(frag.y);
                    }
                }
            }
            hits
        };
        self.chrome.find_match_count = ys.len();
        if ys.is_empty() {
            self.chrome.find_match_index = 0;
            self.page_dirty = true;
            return;
        }
        if delta > 0 {
            self.chrome.find_match_index = (self.chrome.find_match_index + 1) % ys.len();
        } else if delta < 0 {
            self.chrome.find_match_index = if self.chrome.find_match_index == 0 {
                ys.len() - 1
            } else {
                self.chrome.find_match_index - 1
            };
        } else if self.chrome.find_match_index >= ys.len() {
            self.chrome.find_match_index = 0;
        }
        let target_y = (ys[self.chrome.find_match_index] - 80.0).max(0.0);
        let page = self.active_page_mut();
        page.scroll_y = target_y;
        page.scroll_by(0.0, 0.0);
        self.page_dirty = true;
    }

    fn open_new_tab(&mut self, url: &str) {
        let z = self.chrome.zoom.clamp(0.5, 2.0);
        let vw = (self.logical_w() / z).max(200.0);
        let vh = (content_viewport_height(self.logical_h(), &self.chrome) / z).max(100.0);
        let mut page = Page::new_with_client(
            vw,
            vh,
            self.client.clone(),
            Some(self.profile.origin_storage.clone()),
            self.profile.settings.dark_mode,
            self.profile.settings.engine(),
        );
        page.set_device_pixel_ratio(self.scale_factor);
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        self.tabs.push(BrowserTab {
            id,
            page,
            loading: false,
            load_seq: 0,
            load_started: None,
            last_media_seq: 0,
            pinned: false,
            reader_saved_url: None,
        });
        let idx = self.tabs.len() - 1;
        self.active_tab = idx;
        self.start_navigation_for_tab(idx, url, true);
        self.page_dirty = true;
        self.sync_omnibox();
        self.sync_window_title();
    }

    fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let closed_url = self.tabs[idx].page.url.clone();
        if !closed_url.is_empty() && closed_url != "peedle://newtab" {
            self.closed_tabs.push(closed_url);
            if self.closed_tabs.len() > 25 {
                self.closed_tabs.remove(0);
            }
        }
        if self.tabs.len() <= 1 {
            self.navigate_active("peedle://newtab");
            return;
        }
        self.tabs.remove(idx);
        if let Some(st) = self.split_tab {
            if st == idx {
                self.split_tab = None;
                self.chrome.split_active = false;
            } else if st > idx {
                self.split_tab = Some(st - 1);
            }
        }
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        if self.split_tab == Some(self.active_tab) {
            self.split_tab = None;
            self.chrome.split_active = false;
        }
        self.resize_all_tabs();
        self.page_dirty = true;
        self.split_dirty = true;
        self.sync_omnibox();
        self.sync_window_title();
    }

    fn reopen_closed_tab(&mut self) {
        if let Some(url) = self.closed_tabs.pop() {
            self.open_new_tab(&url);
        }
    }

    fn switch_tab_delta(&mut self, delta: isize) {
        if self.tabs.len() <= 1 {
            return;
        }
        let n = self.tabs.len() as isize;
        self.active_tab = (self.active_tab as isize + delta).rem_euclid(n) as usize;
        self.page_dirty = true;
        self.sync_omnibox();
        self.sync_window_title();
    }

    fn pane_layout(&self) -> (f32, f32, f32, f32) {
        let lw = self.logical_w();
        let ox = content_origin_x(&self.chrome);
        let cw = content_viewport_width(lw, &self.chrome);
        if self.chrome.split_active && self.split_tab.is_some() {
            let div_x = split_divider_x(lw, &self.chrome);
            let left_w = (div_x - ox - SPLIT_DIVIDER_W * 0.5).max(160.0);
            let right_x = div_x + SPLIT_DIVIDER_W * 0.5;
            let right_w = (lw - right_x).max(160.0);
            (ox, left_w, right_x, right_w)
        } else {
            (ox, cw, lw, 0.0)
        }
    }

    fn active_pane_origin_x(&self) -> f32 {
        let (lx, _, rx, rw) = self.pane_layout();
        if self.chrome.split_active && self.chrome.split_focus_right && rw > 0.0 {
            rx
        } else {
            lx
        }
    }

    fn toggle_vertical_tabs(&mut self) {
        self.chrome.vertical_tabs = !self.chrome.vertical_tabs;
        self.profile.settings.vertical_tabs = self.chrome.vertical_tabs;
        self.profile.save_settings();
        self.resize_all_tabs();
    }

    fn toggle_split_view(&mut self) {
        if self.chrome.split_active {
            self.chrome.split_active = false;
            self.chrome.split_focus_right = false;
            self.split_tab = None;
            self.resize_all_tabs();
            return;
        }
        if self.tabs.len() < 2 {
            let prev = self.active_tab;
            self.open_new_tab("peedle://newtab");
            let right = self.active_tab;
            self.active_tab = prev;
            self.split_tab = Some(right);
        } else {
            let other = (0..self.tabs.len())
                .find(|&i| i != self.active_tab)
                .unwrap_or(0);
            self.split_tab = Some(other);
        }
        self.chrome.split_active = true;
        self.chrome.split_focus_right = false;
        self.resize_all_tabs();
        self.sync_omnibox();
        self.sync_window_title();
    }

    fn split_with_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() || idx == self.active_tab {
            return;
        }
        self.split_tab = Some(idx);
        self.chrome.split_active = true;
        self.chrome.split_focus_right = false;
        self.resize_all_tabs();
    }

    fn toggle_shields(&mut self) {
        let next = !self.chrome.shields_enabled;
        self.chrome.shields_enabled = next;
        self.profile.settings.adblock_enabled = next;
        self.client.set_adblock_enabled(next);
        self.profile.save_settings();
        let url = self.active_page().url.clone();
        if url.starts_with("http://") || url.starts_with("https://") {
            let idx = self.active_tab;
            self.start_navigation_for_tab(idx, &url, false);
        }
    }

    fn toggle_reader_mode(&mut self) {
        let idx = self.active_tab;
        if let Some(orig) = self.tabs[idx].reader_saved_url.take() {
            self.chrome.reader_active = false;
            self.start_navigation_for_tab(idx, &orig, false);
            return;
        }
        let cur = self.tabs[idx].page.url.clone();
        if cur.starts_with("peedle://") || cur.starts_with("about:") {
            return;
        }
        let html = self.tabs[idx].page.build_reader_mode_html();
        self.tabs[idx].reader_saved_url = Some(cur.clone());
        self.tabs[idx].page.load_custom_internal(&cur, &html, false);
        self.chrome.reader_active = true;
        self.page_dirty = true;
        self.split_dirty = true;
    }

    fn toggle_pin_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let target_id = self.tabs[idx].id;
        let active_id = self.tabs[self.active_tab].id;
        let split_id = self.split_tab.and_then(|s| self.tabs.get(s).map(|t| t.id));
        self.tabs[idx].pinned = !self.tabs[idx].pinned;
        let mut pinned_list = Vec::new();
        let mut normal_list = Vec::new();
        for t in self.tabs.drain(..) {
            if t.pinned {
                pinned_list.push(t);
            } else {
                normal_list.push(t);
            }
        }
        self.tabs.extend(pinned_list);
        self.tabs.extend(normal_list);
        if let Some(pos) = self.tabs.iter().position(|t| t.id == active_id) {
            self.active_tab = pos;
        }
        if let Some(sid) = split_id {
            self.split_tab = self.tabs.iter().position(|t| t.id == sid);
        }
        let _ = target_id;
        self.page_dirty = true;
    }

    fn resize_all_tabs(&mut self) {
        let z = self.chrome.zoom.clamp(0.5, 2.0);
        let (_, left_w, _, right_w) = self.pane_layout();
        let full_w = content_viewport_width(self.logical_w(), &self.chrome);
        let vh = (content_viewport_height(self.logical_h(), &self.chrome) / z).max(100.0);
        let left_tab = if self.chrome.split_active && self.chrome.split_focus_right {
            self.split_tab
        } else {
            Some(self.active_tab)
        };
        let right_tab = if self.chrome.split_active {
            if self.chrome.split_focus_right {
                Some(self.active_tab)
            } else {
                self.split_tab
            }
        } else {
            None
        };
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            tab.page.set_device_pixel_ratio(self.scale_factor);
            let w_log = if self.chrome.split_active && right_w > 0.0 {
                if Some(i) == left_tab {
                    left_w
                } else if Some(i) == right_tab {
                    right_w
                } else {
                    full_w
                }
            } else {
                full_w
            };
            tab.page.resize((w_log / z).max(160.0), vh);
        }
        self.page_dirty = true;
        self.split_dirty = true;
    }

    fn scroll_to_scrollbar_y(&mut self, y: f32) {
        let top_h = top_chrome_height(&self.chrome);
        let vh = content_viewport_height(self.logical_h(), &self.chrome);
        let ratio = ((y - top_h) / vh.max(1.0)).clamp(0.0, 1.0);
        let page = self.active_page_mut();
        let content_h = page.js_env.dom.borrow().layout.content_height;
        let max_scroll = (content_h - page.viewport_h).max(0.0);
        page.scroll_y = ratio * max_scroll;
        page.scroll_by(0.0, 0.0);
        self.page_dirty = true;
    }

    fn open_browser_menu(&mut self) {
        let lw = self.logical_w();
        let mw = 248.0;
        let x = (lw - mw - 10.0).max(8.0);
        let y = toolbar_top(&self.chrome) + crate::chrome::TOOLBAR_H + 4.0;
        let items = vec![
            ContextMenuItem { label: "New tab   Ctrl+T".into(), action: MenuAction::NewTab, enabled: true },
            ContextMenuItem {
                label: "Reopen closed tab".into(),
                action: MenuAction::ReopenClosedTab,
                enabled: !self.closed_tabs.is_empty(),
            },
            ContextMenuItem {
                label: if self.chrome.vertical_tabs { "Horizontal tabs   Ctrl+Shift+V".into() } else { "Vertical tabs   Ctrl+Shift+V".into() },
                action: MenuAction::ToggleVerticalTabs,
                enabled: true,
            },
            ContextMenuItem {
                label: if self.chrome.split_active { "Close split view   Ctrl+Shift+S".into() } else { "Split view   Ctrl+Shift+S".into() },
                action: MenuAction::ToggleSplitView,
                enabled: true,
            },
            ContextMenuItem {
                label: if self.chrome.shields_enabled {
                    format!("Shields: ON ({} blocked)", self.client.blocked_count())
                } else {
                    "Shields: OFF".into()
                },
                action: MenuAction::ToggleShields,
                enabled: true,
            },
            ContextMenuItem {
                label: if self.chrome.reader_active { "Exit reader mode".into() } else { "Reader mode   Ctrl+Alt+R".into() },
                action: MenuAction::ToggleReaderMode,
                enabled: true,
            },
            ContextMenuItem { label: "History   Ctrl+H".into(), action: MenuAction::OpenUrl("peedle://history".into()), enabled: true },
            ContextMenuItem { label: "Downloads   Ctrl+J".into(), action: MenuAction::OpenUrl("peedle://downloads".into()), enabled: true },
            ContextMenuItem { label: "Bookmarks".into(), action: MenuAction::OpenUrl("peedle://bookmarks".into()), enabled: true },
            ContextMenuItem {
                label: if self.chrome.bookmarks_visible { "Hide bookmarks bar".into() } else { "Show bookmarks bar".into() },
                action: MenuAction::ToggleBookmarksBar,
                enabled: true,
            },
            ContextMenuItem { label: "Find in page   Ctrl+F".into(), action: MenuAction::OpenFind, enabled: true },
            ContextMenuItem {
                label: format!("Zoom in ({}%)", (self.chrome.zoom * 100.0).round() as i32),
                action: MenuAction::ZoomIn,
                enabled: true,
            },
            ContextMenuItem { label: "Zoom out".into(), action: MenuAction::ZoomOut, enabled: true },
            ContextMenuItem { label: "Reset zoom".into(), action: MenuAction::ZoomReset, enabled: true },
            ContextMenuItem {
                label: if self.chrome.dark_mode { "Switch to Light mode".into() } else { "Switch to Dark mode".into() },
                action: MenuAction::ToggleTheme,
                enabled: true,
            },
            ContextMenuItem { label: "DevTools   F12".into(), action: MenuAction::ToggleDevTools, enabled: true },
            ContextMenuItem { label: "View page source   Ctrl+U".into(), action: MenuAction::ViewSource, enabled: true },
            ContextMenuItem { label: "Save page as...   Ctrl+S".into(), action: MenuAction::SavePage, enabled: true },
            ContextMenuItem { label: "Settings".into(), action: MenuAction::OpenUrl("peedle://settings".into()), enabled: true },
            ContextMenuItem { label: "About Peedle".into(), action: MenuAction::OpenUrl("peedle://about".into()), enabled: true },
        ];
        self.chrome.popup_menu = Some(PopupMenu { x, y, w: mw, items });
    }

    fn open_context_menu(&mut self, x: f32, y: f32) {
        let hit = hit_test_chrome(
            x,
            y,
            self.logical_w(),
            self.logical_h(),
            self.tabs.len(),
            &self.chrome,
        );
        let mut items = Vec::new();
        match hit {
            ChromeHit::Tab(i) | ChromeHit::CloseTab(i) | ChromeHit::TabAudio(i) => {
                let is_pin = self.tabs.get(i).map_or(false, |t| t.pinned);
                let is_muted = self.tabs.get(i).map_or(false, |t| t.page.tab_muted);
                items.push(ContextMenuItem { label: "New tab".into(), action: MenuAction::NewTab, enabled: true });
                items.push(ContextMenuItem {
                    label: if is_pin { "Unpin tab".into() } else { "Pin tab".into() },
                    action: MenuAction::TogglePinTab(i),
                    enabled: true,
                });
                items.push(ContextMenuItem {
                    label: if is_muted { "Unmute tab".into() } else { "Mute tab".into() },
                    action: MenuAction::ToggleMuteTab(i),
                    enabled: true,
                });
                if i != self.active_tab {
                    items.push(ContextMenuItem {
                        label: "Split with active tab".into(),
                        action: MenuAction::SplitWithTab(i),
                        enabled: true,
                    });
                }
                items.push(ContextMenuItem { label: "Duplicate tab".into(), action: MenuAction::DuplicateTab(i), enabled: true });
                items.push(ContextMenuItem { label: "Reload tab".into(), action: MenuAction::ReloadTab(i), enabled: true });
                items.push(ContextMenuItem { label: "Close tab".into(), action: MenuAction::CloseTab(i), enabled: !is_pin });
                items.push(ContextMenuItem {
                    label: "Close other tabs".into(),
                    action: MenuAction::CloseOtherTabs(i),
                    enabled: self.tabs.len() > 1,
                });
                items.push(ContextMenuItem {
                    label: "Reopen closed tab".into(),
                    action: MenuAction::ReopenClosedTab,
                    enabled: !self.closed_tabs.is_empty(),
                });
            }
            ChromeHit::BookmarkItem(i) => {
                if let Some(bm) = self.profile.bookmarks.get(i) {
                    items.push(ContextMenuItem {
                        label: "Open in new tab".into(),
                        action: MenuAction::OpenLinkNewTab(bm.url.clone()),
                        enabled: true,
                    });
                    items.push(ContextMenuItem {
                        label: "Copy URL".into(),
                        action: MenuAction::CopyText(bm.url.clone()),
                        enabled: true,
                    });
                    items.push(ContextMenuItem {
                        label: "Delete bookmark".into(),
                        action: MenuAction::RemoveBookmark(i),
                        enabled: true,
                    });
                }
            }
            ChromeHit::Content => {
                let z = self.chrome.zoom.clamp(0.5, 2.0);
                let cx = x / z;
                let vy = (y - top_chrome_height(&self.chrome)) / z;
                if let Some(href) = self.active_page().href_at(cx, vy) {
                    items.push(ContextMenuItem {
                        label: "Open link in new tab".into(),
                        action: MenuAction::OpenLinkNewTab(href.clone()),
                        enabled: true,
                    });
                    items.push(ContextMenuItem {
                        label: "Copy link address".into(),
                        action: MenuAction::CopyText(href),
                        enabled: true,
                    });
                }
                let sel = self.active_page().selected_text();
                if !sel.trim().is_empty() {
                    let short: String = sel.trim().chars().take(20).collect();
                    items.push(ContextMenuItem {
                        label: "Copy".into(),
                        action: MenuAction::CopyText(sel.clone()),
                        enabled: true,
                    });
                    items.push(ContextMenuItem {
                        label: format!("Search \"{}\"", short),
                        action: MenuAction::SearchText(sel),
                        enabled: true,
                    });
                }
                items.push(ContextMenuItem { label: "Back".into(), action: MenuAction::Back, enabled: self.active_page().can_go_back() });
                items.push(ContextMenuItem { label: "Forward".into(), action: MenuAction::Forward, enabled: self.active_page().can_go_forward() });
                items.push(ContextMenuItem { label: "Reload".into(), action: MenuAction::Reload, enabled: true });
                items.push(ContextMenuItem { label: "Save page as...".into(), action: MenuAction::SavePage, enabled: true });
                items.push(ContextMenuItem { label: "View page source".into(), action: MenuAction::ViewSource, enabled: true });
                items.push(ContextMenuItem { label: "Inspect".into(), action: MenuAction::Inspect, enabled: true });
            }
            _ => return,
        }
        if items.is_empty() {
            return;
        }
        let mw = 216.0;
        let mh = items.len() as f32 * 30.0 + 10.0;
        let mx = x.min((self.logical_w() - mw - 6.0).max(4.0));
        let my = y.min((self.logical_h() - mh - 6.0).max(4.0));
        self.chrome.popup_menu = Some(PopupMenu { x: mx, y: my, w: mw, items });
    }

    fn execute_menu_action(&mut self, action: MenuAction) {
        self.chrome.popup_menu = None;
        match action {
            MenuAction::Back => self.go_back_active(),
            MenuAction::Forward => self.go_forward_active(),
            MenuAction::Reload => {
                let u = self.active_page().url.clone();
                let idx = self.active_tab;
                self.start_navigation_for_tab(idx, &u, false);
            }
            MenuAction::NewTab => {
                self.open_new_tab("peedle://newtab");
                self.chrome.omnibox_focused = true;
                self.chrome.omnibox.clear();
                self.refresh_suggestions();
            }
            MenuAction::ReopenClosedTab => self.reopen_closed_tab(),
            MenuAction::DuplicateTab(i) => {
                if let Some(tab) = self.tabs.get(i) {
                    let u = tab.page.url.clone();
                    self.open_new_tab(&u);
                }
            }
            MenuAction::ReloadTab(i) => {
                if let Some(tab) = self.tabs.get(i) {
                    let u = tab.page.url.clone();
                    self.start_navigation_for_tab(i, &u, false);
                }
            }
            MenuAction::CloseTab(i) => self.close_tab(i),
            MenuAction::TogglePinTab(i) => self.toggle_pin_tab(i),
            MenuAction::ToggleMuteTab(i) => {
                if let Some(tab) = self.tabs.get_mut(i) {
                    tab.page.toggle_tab_mute();
                }
                self.page_dirty = true;
            }
            MenuAction::SplitWithTab(i) => self.split_with_tab(i),
            MenuAction::ToggleVerticalTabs => self.toggle_vertical_tabs(),
            MenuAction::ToggleSplitView => self.toggle_split_view(),
            MenuAction::ToggleShields => self.toggle_shields(),
            MenuAction::ToggleReaderMode => self.toggle_reader_mode(),
            MenuAction::CloseOtherTabs(keep) => {
                if keep < self.tabs.len() {
                    let kept = self.tabs.remove(keep);
                    for t in self.tabs.drain(..) {
                        if t.page.url != "peedle://newtab" {
                            self.closed_tabs.push(t.page.url);
                        }
                    }
                    self.tabs.push(kept);
                    self.active_tab = 0;
                    self.page_dirty = true;
                    self.sync_omnibox();
                    self.sync_window_title();
                }
            }
            MenuAction::OpenLinkNewTab(url) => self.open_new_tab(&url),
            MenuAction::CopyText(txt) => write_clipboard(&txt),
            MenuAction::SearchText(q) => {
                let url = self.profile.settings.engine().search_url(q.trim());
                self.open_new_tab(&url);
            }
            MenuAction::SavePage => {
                let title = self.active_page().title();
                let url = self.active_page().url.clone();
                let html = if !self.active_page().source_html.is_empty() {
                    self.active_page().source_html.clone()
                } else {
                    self.active_page().outer_html()
                };
                if let Ok(saved) = self.profile.save_page_html(&title, &url, &html) {
                    self.chrome.has_downloads = true;
                    self.chrome.download_toast = Some(DownloadToast {
                        filename: saved.filename,
                        path: saved.path,
                        bytes: saved.bytes,
                    });
                }
            }
            MenuAction::ViewSource => {
                let u = self.active_page().url.clone();
                if !u.starts_with("view-source:") {
                    self.open_new_tab(&format!("view-source:{}", u));
                }
            }
            MenuAction::Inspect => {
                self.chrome.devtools_open = true;
                self.chrome.devtools_tab = DevToolsTab::Elements;
                self.resize_all_tabs();
            }
            MenuAction::OpenUrl(u) => self.open_new_tab(&u),
            MenuAction::ToggleBookmarksBar => {
                self.chrome.bookmarks_visible = !self.chrome.bookmarks_visible;
                self.profile.settings.bookmarks_visible = self.chrome.bookmarks_visible;
                self.profile.save_settings();
                self.resize_all_tabs();
            }
            MenuAction::OpenFind => {
                self.chrome.find_open = true;
                self.chrome.omnibox_focused = false;
                self.chrome.console_focused = false;
            }
            MenuAction::ToggleDevTools => {
                self.chrome.devtools_open = !self.chrome.devtools_open;
                self.resize_all_tabs();
            }
            MenuAction::ZoomIn => {
                self.chrome.zoom = (self.chrome.zoom + 0.1).min(2.0);
                self.resize_all_tabs();
            }
            MenuAction::ZoomOut => {
                self.chrome.zoom = (self.chrome.zoom - 0.1).max(0.5);
                self.resize_all_tabs();
            }
            MenuAction::ZoomReset => {
                self.chrome.zoom = 1.0;
                self.resize_all_tabs();
            }
            MenuAction::ToggleTheme => {
                let dark = !self.profile.settings.dark_mode;
                self.profile.settings.dark_mode = dark;
                self.chrome.dark_mode = dark;
                for tab in &mut self.tabs {
                    tab.page.dark_mode = dark;
                    tab.page.js_env.dom.borrow_mut().dark_mode = dark;
                    tab.page.js_env.dom.borrow_mut().dirty_style = true;
                }
                self.profile.save_settings();
                self.page_dirty = true;
            }
            MenuAction::RemoveBookmark(i) => {
                if i < self.profile.bookmarks.len() {
                    self.profile.bookmarks.remove(i);
                    self.profile.save_bookmarks();
                    self.chrome.bookmarks = self.profile.bookmarks.clone();
                }
            }
        }
    }

    fn update_cursor_icon(&self) {
        let Some(ref win) = self.window else {
            return;
        };
        let icon = match self.chrome.hovered_hit {
            ChromeHit::Tab(_)
            | ChromeHit::TabAudio(_)
            | ChromeHit::CloseTab(_)
            | ChromeHit::NewTab
            | ChromeHit::Back
            | ChromeHit::Forward
            | ChromeHit::Reload
            | ChromeHit::ZoomReset
            | ChromeHit::ToggleBookmark
            | ChromeHit::ToggleShields
            | ChromeHit::ToggleReaderMode
            | ChromeHit::ToggleVerticalTabs
            | ChromeHit::ToggleSplitView
            | ChromeHit::DownloadsBtn
            | ChromeHit::ToggleDevTools
            | ChromeHit::MenuBtn
            | ChromeHit::BookmarkItem(_)
            | ChromeHit::SuggestionItem(_)
            | ChromeHit::PopupMenuItem(_)
            | ChromeHit::SelectOptionItem(_)
            | ChromeHit::ToastReveal
            | ChromeHit::ToastDownloads
            | ChromeHit::ToastClose
            | ChromeHit::DevToolsTab(_) => CursorIcon::Pointer,
            ChromeHit::SplitDivider => CursorIcon::EwResize,
            ChromeHit::Omnibox | ChromeHit::ConsoleInput | ChromeHit::FindInput => CursorIcon::Text,
            ChromeHit::Content => match self.active_page().hovered_cursor() {
                peedle_css::Cursor::Pointer => CursorIcon::Pointer,
                peedle_css::Cursor::Text | peedle_css::Cursor::VerticalText => CursorIcon::Text,
                peedle_css::Cursor::Move => CursorIcon::Move,
                peedle_css::Cursor::Grab => CursorIcon::Grab,
                peedle_css::Cursor::Grabbing => CursorIcon::Grabbing,
                peedle_css::Cursor::NotAllowed | peedle_css::Cursor::NoDrop => CursorIcon::NotAllowed,
                peedle_css::Cursor::Crosshair => CursorIcon::Crosshair,
                peedle_css::Cursor::Wait => CursorIcon::Wait,
                peedle_css::Cursor::Progress => CursorIcon::Progress,
                peedle_css::Cursor::Help => CursorIcon::Help,
                peedle_css::Cursor::ZoomIn => CursorIcon::ZoomIn,
                peedle_css::Cursor::ZoomOut => CursorIcon::ZoomOut,
                _ => CursorIcon::Default,
            },
            ChromeHit::Scrollbar | ChromeHit::TabBarEmpty | ChromeHit::None => CursorIcon::Default,
        };
        win.set_cursor(icon);
    }

    fn paste_text(&mut self, raw: &str) {
        if self.chrome.omnibox_focused {
            self.chrome.omnibox.insert_str(raw.trim());
            self.refresh_suggestions();
        } else if self.chrome.find_open {
            self.chrome.find.insert_str(raw);
            self.run_find(0);
        } else if self.chrome.console_focused {
            self.chrome.console.insert_str(raw);
        } else if self.active_page().has_focused_input() {
            for c in raw.chars() {
                if !c.is_control() || c == '\n' {
                    self.active_page_mut().on_text_input(c);
                }
            }
            self.page_dirty = true;
        }
    }

    fn sync_audio_tab_flags(&mut self) {
        self.chrome.audio_tabs = self
            .tabs
            .iter()
            .map(|t| t.page.is_playing_audio() || t.page.tab_muted)
            .collect();
        self.chrome.pinned_tabs = self.tabs.iter().map(|t| t.pinned).collect();
        self.chrome.blocked_count = self.client.blocked_count();
        self.chrome.reader_active = self
            .tabs
            .get(self.active_tab)
            .map_or(false, |t| t.reader_saved_url.is_some());
    }

    fn toggle_window_fullscreen(&mut self) {
        self.window_fullscreen = !self.window_fullscreen;
        if let Some(ref win) = self.window {
            if self.window_fullscreen {
                win.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
            } else {
                win.set_fullscreen(None);
            }
        }
    }

    fn render(&mut self) {
        if self.phys_w == 0 || self.phys_h == 0 || self.tabs.is_empty() {
            return;
        }
        self.sync_audio_tab_flags();
        let s = self.scale_factor.max(1.0);
        let lh = self.logical_h();
        let video_fs = self.active_page().is_fullscreen_video();
        if video_fs {
            self.full_fb
                .resize_or_clear(self.phys_w, self.phys_h, RgbaColor::new(0, 0, 0, 255));
            self.tabs[self.active_tab]
                .page
                .render_to_framebuffer_scaled(&mut self.full_fb, s);
            self.page_dirty = false;
            if let Some(ref mut surface) = self.surface {
                if let (Some(w), Some(h)) = (NonZeroU32::new(self.phys_w), NonZeroU32::new(self.phys_h)) {
                    let _ = surface.resize(w, h);
                    if let Ok(mut buffer) = surface.buffer_mut() {
                        self.full_fb.to_xrgb_slice(&mut buffer);
                        let _ = buffer.present();
                    }
                }
            }
            return;
        }
        let top_h = top_chrome_height(&self.chrome);
        let vh = content_viewport_height(lh, &self.chrome);
        let z = self.chrome.zoom.clamp(0.5, 2.0);
        let eff_scale = (z * s).clamp(0.5, 4.0);
        let (left_x, left_w, right_x, right_w) = self.pane_layout();
        let (left_tab, right_tab) = if self.chrome.split_active && right_w > 0.0 {
            if self.chrome.split_focus_right {
                (self.split_tab.unwrap_or(self.active_tab), Some(self.active_tab))
            } else {
                (self.active_tab, self.split_tab)
            }
        } else {
            (self.active_tab, None)
        };
        let left_pw = (left_w * s).ceil().max(1.0) as u32;
        let target_ph = (vh * s).ceil().max(1.0) as u32;

        if self.page_dirty || self.page_fb.width != left_pw || self.page_fb.height != target_ph {
            self.page_fb.resize_or_clear(left_pw, target_ph, RgbaColor::WHITE);
            self.tabs[left_tab]
                .page
                .render_to_framebuffer_scaled(&mut self.page_fb, eff_scale);

            if left_tab == self.active_tab
                && self.chrome.find_open
                && !self.chrome.find.text.trim().is_empty()
            {
                let q = self.chrome.find.text.trim().to_ascii_lowercase();
                let page = &self.tabs[self.active_tab].page;
                let sx = page.scroll_x;
                let sy = page.scroll_y;
                let st = page.js_env.dom.borrow();
                let mut hit_idx = 0usize;
                for b in &st.layout.boxes {
                    for frag in &b.text_fragments {
                        if frag.text.to_ascii_lowercase().contains(&q) {
                            let color = if hit_idx == self.chrome.find_match_index {
                                RgbaColor::new(249, 115, 22, 130)
                            } else {
                                RgbaColor::new(250, 204, 21, 105)
                            };
                            self.page_fb.fill_rounded_rect(
                                (frag.x - sx - 1.0) * eff_scale,
                                (frag.y - sy) * eff_scale,
                                (frag.width + 2.0) * eff_scale,
                                frag.height * eff_scale,
                                [2.0 * eff_scale; 4],
                                color,
                            );
                            hit_idx += 1;
                        }
                    }
                }
            }
            self.page_dirty = false;
        }

        if let Some(rt_idx) = right_tab {
            let right_pw = (right_w * s).ceil().max(1.0) as u32;
            if self.split_dirty
                || self.split_fb.width != right_pw
                || self.split_fb.height != target_ph
            {
                self.split_fb.resize_or_clear(right_pw, target_ph, RgbaColor::WHITE);
                self.tabs[rt_idx]
                    .page
                    .render_to_framebuffer_scaled(&mut self.split_fb, eff_scale);
                self.split_dirty = false;
            }
        }

        self.full_fb
            .resize_or_clear(self.phys_w, self.phys_h, RgbaColor::new(17, 20, 27, 255));
        let top_phys = top_h * s;
        let vh_phys = vh * s;
        let lx_phys = left_x * s;
        let lw_phys = left_w * s;
        self.full_fb.push_clip(ClipRect {
            x: lx_phys.floor() as i32,
            y: top_phys.floor() as i32,
            w: lw_phys.ceil() as i32,
            h: vh_phys.ceil() as i32,
        });
        self.full_fb.blit_rgba_opaque(
            self.page_fb.width,
            self.page_fb.height,
            &self.page_fb.data,
            lx_phys,
            top_phys,
            lw_phys,
            vh_phys,
        );
        self.full_fb.pop_clip();

        if right_tab.is_some() {
            let rx_phys = right_x * s;
            let rw_phys = right_w * s;
            self.full_fb.push_clip(ClipRect {
                x: rx_phys.floor() as i32,
                y: top_phys.floor() as i32,
                w: rw_phys.ceil() as i32,
                h: vh_phys.ceil() as i32,
            });
            self.full_fb.blit_rgba_opaque(
                self.split_fb.width,
                self.split_fb.height,
                &self.split_fb.data,
                rx_phys,
                top_phys,
                rw_phys,
                vh_phys,
            );
            self.full_fb.pop_clip();
        }

        let visuals: Vec<TabVisual<'_>> = self
            .tabs
            .iter()
            .map(|t| {
                let elapsed = t
                    .load_started
                    .map(|st| st.elapsed().as_secs_f32())
                    .unwrap_or(0.0);
                let progress = 1.0 - (-elapsed * 1.4).exp();
                TabVisual {
                    page: &t.page,
                    loading: t.loading,
                    load_progress: progress,
                    playing_audio: t.page.is_playing_audio(),
                    muted: t.page.tab_muted,
                    pinned: t.pinned,
                }
            })
            .collect();

        draw_chrome(
            &mut self.full_fb,
            &self.fonts,
            &visuals,
            self.active_tab,
            &self.chrome,
            s,
        );

        if let Some(ref mut surface) = self.surface {
            if let (Some(w), Some(h)) = (NonZeroU32::new(self.phys_w), NonZeroU32::new(self.phys_h)) {
                let _ = surface.resize(w, h);
                if let Ok(mut buffer) = surface.buffer_mut() {
                    self.full_fb.to_xrgb_slice(&mut buffer);
                    let _ = buffer.present();
                }
            }
        }
    }

    fn request_redraw(&self) {
        if let Some(ref win) = self.window {
            win.request_redraw();
        }
    }

    fn needs_animation_tick(&self) -> bool {
        let any_loading = self.tabs.iter().any(|t| t.loading);
        let smooth_scroll = self.scroll_vel_y.abs() > 0.4;
        let active_timers = self
            .tabs
            .get(self.active_tab)
            .map_or(false, |t| t.page.has_pending_timers());
        let active_media = self
            .tabs
            .get(self.active_tab)
            .map_or(false, |t| t.page.has_active_media());
        any_loading || smooth_scroll || active_timers || active_media
    }
}

impl ApplicationHandler<AppEvent> for BrowserApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let mut attrs = WindowAttributes::default()
            .with_title("Peedle")
            .with_inner_size(LogicalSize::new(1320.0, 840.0));
        if let Some(win_icon) = crate::icon::build_window_icon() {
            attrs = attrs.with_window_icon(Some(win_icon));
        }
        let win = Rc::new(event_loop.create_window(attrs).unwrap());
        let ctx = Context::new(win.clone()).unwrap();
        let surface = Surface::new(&ctx, win.clone()).unwrap();

        self.scale_factor = (win.scale_factor() as f32).max(1.0);
        let sz = win.inner_size();
        if sz.width > 0 && sz.height > 0 {
            self.phys_w = sz.width;
            self.phys_h = sz.height;
        }
        self.window = Some(win);
        self.surface = Some(surface);

        let cli_explicit = !(self.initial_urls.len() == 1 && self.initial_urls[0] == "peedle://newtab");
        if cli_explicit {
            let urls = self.initial_urls.clone();
            for u in &urls {
                self.open_new_tab(u);
            }
        } else if self.profile.settings.restore_session && !self.profile.session.tabs.is_empty() {
            let restored = self.profile.session.tabs.clone();
            let saved_active = self.profile.session.active_tab;
            for u in &restored {
                self.open_new_tab(u);
            }
            if saved_active < self.tabs.len() {
                self.active_tab = saved_active;
                self.sync_omnibox();
                self.sync_window_title();
            }
        } else {
            self.open_new_tab("peedle://newtab");
        }
        self.render();
        self.request_redraw();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::NavigationReady {
                tab_id,
                seq,
                push_history,
                prepared,
            } => {
                let Some(tab_idx) = self.tabs.iter().position(|t| t.id == tab_id) else {
                    return;
                };
                if self.tabs[tab_idx].load_seq != seq {
                    return;
                }
                self.tabs[tab_idx].loading = false;
                let dl_opt = self.tabs[tab_idx]
                    .page
                    .commit_prepared_navigation(prepared, push_history);
                if let Some(dl) = dl_opt {
                    self.handle_download(dl);
                } else {
                    let final_url = self.tabs[tab_idx].page.url.clone();
                    let final_title = self.tabs[tab_idx].page.title();
                    self.profile.record_visit(&final_url, &final_title);
                    self.profile.save_cookies();
                    self.profile.save_origin_storage();
                }
                let new_tabs = self.tabs[tab_idx].page.drain_pending_new_tabs();
                for nu in new_tabs {
                    self.open_new_tab(&nu);
                }
                if tab_idx == self.active_tab {
                    self.page_dirty = true;
                    self.split_dirty = true;
                    self.sync_omnibox();
                    self.sync_window_title();
                } else if self.split_tab == Some(tab_idx) {
                    self.split_dirty = true;
                }
                self.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt_ms = now.duration_since(self.last_tick).as_millis() as u64;
        if self.needs_animation_tick() {
            if dt_ms >= 15 {
                self.last_tick = now;
                let mut need_draw = false;
                if self.tabs.iter().any(|t| t.loading) {
                    self.chrome.anim_phase =
                        (self.chrome.anim_phase + dt_ms as f32 * 0.008) % (std::f32::consts::TAU);
                    need_draw = true;
                }
                if self.scroll_vel_y.abs() > 0.4 {
                    let step = self.scroll_vel_y * 0.36;
                    self.scroll_vel_y *= 0.64;
                    if self.scroll_vel_y.abs() <= 0.4 {
                        self.scroll_vel_y = 0.0;
                    }
                    self.active_page_mut().scroll_by(0.0, step);
                    self.page_dirty = true;
                    self.split_dirty = true;
                    need_draw = true;
                }
                if !self.tabs.is_empty() && self.active_page().has_pending_timers() {
                    self.active_page_mut().advance_time_ms(dt_ms.min(50));
                    let new_tabs = self.active_page_mut().drain_pending_new_tabs();
                    for nu in new_tabs {
                        self.open_new_tab(&nu);
                    }
                    self.page_dirty = true;
                    self.split_dirty = true;
                    need_draw = true;
                }
                if !self.tabs.is_empty() && self.active_page().has_active_media() {
                    let seq = self.active_page().media_frame_seq();
                    if seq != self.tabs[self.active_tab].last_media_seq || dt_ms >= 32 {
                        self.tabs[self.active_tab].last_media_seq = seq;
                        self.page_dirty = true;
                        self.split_dirty = true;
                        need_draw = true;
                    }
                }
                if need_draw {
                    self.request_redraw();
                }
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(now + Duration::from_millis(16)));
        } else {
            self.last_tick = now;
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.persist_session();
                event_loop.exit();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale_factor = (scale_factor as f32).max(1.0);
                self.resize_all_tabs();
                self.request_redraw();
            }
            WindowEvent::Resized(sz) => {
                if sz.width > 0 && sz.height > 0 {
                    self.phys_w = sz.width;
                    self.phys_h = sz.height;
                    if let Some(ref win) = self.window {
                        self.scale_factor = (win.scale_factor() as f32).max(1.0);
                    }
                    self.resize_all_tabs();
                    self.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let s = self.scale_factor.max(1.0);
                self.cursor_x = position.x as f32 / s;
                self.cursor_y = position.y as f32 / s;
                if self.split_dragging {
                    let ox = content_origin_x(&self.chrome);
                    let cw = content_viewport_width(self.logical_w(), &self.chrome);
                    self.chrome.split_ratio = ((self.cursor_x - ox) / cw).clamp(0.25, 0.75);
                    self.resize_all_tabs();
                    self.request_redraw();
                    return;
                }
                if self.scrollbar_dragging {
                    self.scroll_to_scrollbar_y(self.cursor_y);
                    self.request_redraw();
                    return;
                }
                if self.selecting_page_text {
                    let z = self.chrome.zoom.clamp(0.5, 2.0);
                    let cx = (self.cursor_x - self.active_pane_origin_x()) / z;
                    let vy = (self.cursor_y - top_chrome_height(&self.chrome)) / z;
                    let start = self.select_drag_start;
                    self.active_page_mut().set_selection_viewport(start, (cx, vy));
                    self.page_dirty = true;
                    self.split_dirty = true;
                    self.request_redraw();
                    return;
                }
                if self.active_page().is_fullscreen_video() {
                    return;
                }
                let hit = hit_test_chrome(
                    self.cursor_x,
                    self.cursor_y,
                    self.logical_w(),
                    self.logical_h(),
                    self.tabs.len(),
                    &self.chrome,
                );
                let hit_changed = hit != self.chrome.hovered_hit;
                self.chrome.hovered_hit = hit;
                if hit == ChromeHit::Content {
                    let z = self.chrome.zoom.clamp(0.5, 2.0);
                    let cx = (self.cursor_x - self.active_pane_origin_x()) / z;
                    let vy = (self.cursor_y - top_chrome_height(&self.chrome)) / z;
                    let vh = content_viewport_height(self.logical_h(), &self.chrome) / z;
                    if vy >= 0.0 && vy < vh && self.active_page_mut().on_mouse_move(cx, vy) {
                        self.page_dirty = true;
                        self.split_dirty = true;
                        self.request_redraw();
                    }
                }
                self.update_cursor_icon();
                if hit_changed {
                    self.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let s = self.scale_factor.max(1.0);
                match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        if x.abs() > 0.01 {
                            self.active_page_mut().scroll_by(-x * 42.0, 0.0);
                            self.page_dirty = true;
                        }
                        self.scroll_vel_y = (self.scroll_vel_y - y * 92.0).clamp(-900.0, 900.0);
                    }
                    MouseScrollDelta::PixelDelta(p) => {
                        self.active_page_mut()
                            .scroll_by(-p.x as f32 / s, -p.y as f32 / s);
                        self.page_dirty = true;
                    }
                }
                self.request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.scrollbar_dragging = false;
                self.selecting_page_text = false;
                self.split_dragging = false;
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                self.chrome.select_popup = None;
                self.open_context_menu(self.cursor_x, self.cursor_y);
                self.request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Middle,
                ..
            } => {
                if self.active_page().is_fullscreen_video() {
                    return;
                }
                let hit = hit_test_chrome(
                    self.cursor_x,
                    self.cursor_y,
                    self.logical_w(),
                    self.logical_h(),
                    self.tabs.len(),
                    &self.chrome,
                );
                match hit {
                    ChromeHit::Tab(i) | ChromeHit::CloseTab(i) => {
                        self.close_tab(i);
                        self.request_redraw();
                    }
                    ChromeHit::Content => {
                        if self.chrome.split_active {
                            if let Some(other) = self.split_tab {
                                let div_x = split_divider_x(self.logical_w(), &self.chrome);
                                let clicked_right = self.cursor_x > div_x;
                                if clicked_right != self.chrome.split_focus_right {
                                    let old_active = self.active_tab;
                                    self.active_tab = other;
                                    self.split_tab = Some(old_active);
                                    self.chrome.split_focus_right = clicked_right;
                                    self.sync_omnibox();
                                    self.sync_window_title();
                                    self.page_dirty = true;
                                    self.split_dirty = true;
                                }
                            }
                        }
                        let z = self.chrome.zoom.clamp(0.5, 2.0);
                        let cx = (self.cursor_x - self.active_pane_origin_x()) / z;
                        let vy = (self.cursor_y - top_chrome_height(&self.chrome)) / z;
                        if let Some(url) = self.active_page().href_at(cx, vy) {
                            self.open_new_tab(&url);
                            self.request_redraw();
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let now = Instant::now();
                let is_double = self.last_click_time.map_or(false, |t| {
                    now.duration_since(t) < Duration::from_millis(380)
                        && (self.cursor_x - self.last_click_pos.0).abs() < 6.0
                        && (self.cursor_y - self.last_click_pos.1).abs() < 6.0
                });
                self.last_click_time = Some(now);
                self.last_click_pos = (self.cursor_x, self.cursor_y);

                if self.active_page().is_fullscreen_video() {
                    let (cx, cy) = (self.cursor_x, self.cursor_y);
                    let act = self.active_page_mut().on_click_ex(cx, cy, false);
                    self.page_dirty = true;
                    if let ClickAction::Navigated(dest) = act {
                        self.navigate_active(&dest);
                    }
                    self.request_redraw();
                    return;
                }

                let hit = hit_test_chrome(
                    self.cursor_x,
                    self.cursor_y,
                    self.logical_w(),
                    self.logical_h(),
                    self.tabs.len(),
                    &self.chrome,
                );

                if let ChromeHit::PopupMenuItem(i) = hit {
                    if let Some(act) = self
                        .chrome
                        .popup_menu
                        .as_ref()
                        .and_then(|m| m.items.get(i))
                        .filter(|it| it.enabled)
                        .map(|it| it.action.clone())
                    {
                        self.execute_menu_action(act);
                    } else {
                        self.chrome.popup_menu = None;
                    }
                    self.request_redraw();
                    return;
                }
                self.chrome.popup_menu = None;

                if let ChromeHit::SelectOptionItem(i) = hit {
                    if let Some(sel) = self.chrome.select_popup.take() {
                        self.active_page_mut().select_option(sel.node_id, i);
                        self.page_dirty = true;
                    }
                    self.request_redraw();
                    return;
                }
                self.chrome.select_popup = None;

                if let ChromeHit::SuggestionItem(i) = hit {
                    if let Some(url) = self.chrome.suggestions.get(i).map(|s| s.url.clone()) {
                        self.chrome.omnibox_focused = false;
                        self.chrome.suggestions.clear();
                        self.navigate_active(&url);
                    }
                    self.request_redraw();
                    return;
                }

                let was_omni = self.chrome.omnibox_focused;
                self.chrome.omnibox_focused = false;
                self.chrome.suggestions.clear();
                self.chrome.console_focused = false;

                match hit {
                    ChromeHit::Tab(i) => {
                        self.active_tab = i;
                        self.page_dirty = true;
                        self.sync_omnibox();
                        self.sync_window_title();
                    }
                    ChromeHit::TabAudio(i) => {
                        if let Some(tab) = self.tabs.get_mut(i) {
                            tab.page.toggle_tab_mute();
                        }
                        self.page_dirty = true;
                    }
                    ChromeHit::CloseTab(i) => self.close_tab(i),
                    ChromeHit::NewTab => {
                        self.open_new_tab("peedle://newtab");
                        self.chrome.omnibox_focused = true;
                        self.chrome.omnibox.clear();
                        self.refresh_suggestions();
                    }
                    ChromeHit::TabBarEmpty if is_double => {
                        self.open_new_tab("peedle://newtab");
                        self.chrome.omnibox_focused = true;
                        self.chrome.omnibox.clear();
                        self.refresh_suggestions();
                    }
                    ChromeHit::Back => self.go_back_active(),
                    ChromeHit::Forward => self.go_forward_active(),
                    ChromeHit::Reload => self.stop_or_reload_active(),
                    ChromeHit::Omnibox => {
                        self.chrome.omnibox_focused = true;
                        if !was_omni || is_double {
                            self.chrome.omnibox.select_all();
                        } else {
                            let (ox, _, _, _) = omnibox_rect_for(self.logical_w(), &self.chrome);
                            let rel_x = (self.cursor_x - (ox + 32.0)).max(0.0);
                            self.chrome.omnibox.set_cursor_from_x(
                                rel_x,
                                &self.fonts,
                                FontFaceKey::SansRegular,
                                13.0,
                            );
                        }
                        self.refresh_suggestions();
                    }
                    ChromeHit::ZoomReset => {
                        self.chrome.zoom = 1.0;
                        self.resize_all_tabs();
                    }
                    ChromeHit::ToggleBookmark => self.toggle_current_bookmark(),
                    ChromeHit::ToggleShields => self.toggle_shields(),
                    ChromeHit::ToggleReaderMode => self.toggle_reader_mode(),
                    ChromeHit::ToggleVerticalTabs => self.toggle_vertical_tabs(),
                    ChromeHit::ToggleSplitView => self.toggle_split_view(),
                    ChromeHit::SplitDivider => {
                        self.split_dragging = true;
                    }
                    ChromeHit::DownloadsBtn => self.open_new_tab("peedle://downloads"),
                    ChromeHit::MenuBtn => self.open_browser_menu(),
                    ChromeHit::BookmarkItem(i) => {
                        if let Some(bm) = self.profile.bookmarks.get(i).cloned() {
                            if self.modifiers.control_key() || self.modifiers.super_key() {
                                self.open_new_tab(&bm.url);
                            } else {
                                self.navigate_active(&bm.url);
                            }
                        }
                    }
                    ChromeHit::ToggleDevTools => {
                        self.chrome.devtools_open = !self.chrome.devtools_open;
                        self.resize_all_tabs();
                    }
                    ChromeHit::DevToolsTab(t) => {
                        self.chrome.devtools_tab = t;
                    }
                    ChromeHit::ConsoleInput => {
                        self.chrome.devtools_tab = DevToolsTab::Console;
                        self.chrome.console_focused = true;
                    }
                    ChromeHit::FindInput => {
                        self.chrome.find_open = true;
                    }
                    ChromeHit::ToastReveal => {
                        if let Some(ref t) = self.chrome.download_toast {
                            reveal_in_file_manager(&t.path);
                        }
                    }
                    ChromeHit::ToastDownloads => {
                        self.chrome.download_toast = None;
                        self.open_new_tab("peedle://downloads");
                    }
                    ChromeHit::ToastClose => {
                        self.chrome.download_toast = None;
                    }
                    ChromeHit::Scrollbar => {
                        self.scrollbar_dragging = true;
                        self.scroll_to_scrollbar_y(self.cursor_y);
                    }
                    ChromeHit::Content => {
                        if self.chrome.split_active {
                            if let Some(other) = self.split_tab {
                                let div_x = split_divider_x(self.logical_w(), &self.chrome);
                                let clicked_right = self.cursor_x > div_x;
                                if clicked_right != self.chrome.split_focus_right {
                                    let old_active = self.active_tab;
                                    self.active_tab = other;
                                    self.split_tab = Some(old_active);
                                    self.chrome.split_focus_right = clicked_right;
                                    self.sync_omnibox();
                                    self.sync_window_title();
                                    self.page_dirty = true;
                                    self.split_dirty = true;
                                }
                            }
                        }
                        let z = self.chrome.zoom.clamp(0.5, 2.0);
                        let cx = (self.cursor_x - self.active_pane_origin_x()) / z;
                        let vy = (self.cursor_y - top_chrome_height(&self.chrome)) / z;
                        let cmd = self.modifiers.super_key() || self.modifiers.control_key();
                        if cmd {
                            if let Some(url) = self.active_page().href_at(cx, vy) {
                                self.open_new_tab(&url);
                                self.request_redraw();
                                return;
                            }
                        }
                        if is_double && self.active_page().href_at(cx, vy).is_none() {
                            self.active_page_mut().select_word_at(cx, vy);
                            self.page_dirty = true;
                            self.request_redraw();
                            return;
                        }
                        self.active_page_mut().clear_selection();
                        let act = self.active_page_mut().on_click_ex(cx, vy, false);
                        self.page_dirty = true;
                        match act {
                            ClickAction::Navigated(dest) => self.navigate_active(&dest),
                            ClickAction::OpenNewTab(dest) => self.open_new_tab(&dest),
                            ClickAction::Download(dl) => self.handle_download(dl),
                            ClickAction::OpenSelect {
                                node_id,
                                x,
                                y,
                                w,
                                h,
                                options,
                                selected,
                            } => {
                                let top_h = top_chrome_height(&self.chrome);
                                self.chrome.select_popup = Some(SelectPopup {
                                    node_id,
                                    x: x * z,
                                    y: top_h + (y + h) * z + 2.0,
                                    w: (w * z).max(140.0),
                                    options,
                                    selected,
                                });
                            }
                            ClickAction::None => {
                                let new_tabs = self.active_page_mut().drain_pending_new_tabs();
                                for nu in new_tabs {
                                    self.open_new_tab(&nu);
                                }
                                if !self.active_page().has_focused_input() {
                                    self.selecting_page_text = true;
                                    self.select_drag_start = (cx, vy);
                                }
                            }
                        }
                    }
                    ChromeHit::TabBarEmpty
                    | ChromeHit::None
                    | ChromeHit::PopupMenuItem(_)
                    | ChromeHit::SelectOptionItem(_)
                    | ChromeHit::SuggestionItem(_) => {}
                }
                self.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                let cmd = self.modifiers.control_key() || self.modifiers.super_key();
                let alt = self.modifiers.alt_key();
                let shift = self.modifiers.shift_key();

                if matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
                    if self.active_page().is_fullscreen_video() {
                        self.active_page_mut().exit_fullscreen_video();
                        self.page_dirty = true;
                        self.request_redraw();
                        return;
                    }
                    if self.window_fullscreen {
                        self.toggle_window_fullscreen();
                        self.request_redraw();
                        return;
                    }
                    if self.chrome.popup_menu.take().is_some()
                        || self.chrome.select_popup.take().is_some()
                    {
                        self.request_redraw();
                        return;
                    }
                    if self.active_tab_ref().loading {
                        self.stop_or_reload_active();
                        self.request_redraw();
                        return;
                    }
                }

                if alt && !cmd {
                    match &event.logical_key {
                        Key::Named(NamedKey::ArrowLeft) => {
                            self.go_back_active();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowRight) => {
                            self.go_forward_active();
                            self.request_redraw();
                            return;
                        }
                        Key::Character(ch) if ch.eq_ignore_ascii_case("d") => {
                            self.chrome.omnibox_focused = true;
                            self.chrome.omnibox.select_all();
                            self.chrome.find_open = false;
                            self.chrome.console_focused = false;
                            self.refresh_suggestions();
                            self.request_redraw();
                            return;
                        }
                        _ => {}
                    }
                }

                if cmd {
                    if (alt || shift) && event.physical_key == PhysicalKey::Code(KeyCode::KeyI) {
                        self.chrome.devtools_open = !self.chrome.devtools_open;
                        self.resize_all_tabs();
                        self.request_redraw();
                        return;
                    }
                    if self.chrome.omnibox_focused {
                        match event.logical_key {
                            Key::Named(NamedKey::ArrowLeft) => {
                                self.chrome.omnibox.move_left(shift, true);
                                self.request_redraw();
                                return;
                            }
                            Key::Named(NamedKey::ArrowRight) => {
                                self.chrome.omnibox.move_right(shift, true);
                                self.request_redraw();
                                return;
                            }
                            Key::Named(NamedKey::Backspace) => {
                                self.chrome.omnibox.backspace(true);
                                self.refresh_suggestions();
                                self.request_redraw();
                                return;
                            }
                            Key::Named(NamedKey::Delete) => {
                                self.chrome.omnibox.delete(true);
                                self.refresh_suggestions();
                                self.request_redraw();
                                return;
                            }
                            _ => {}
                        }
                    }
                    match event.logical_key {
                        Key::Named(NamedKey::Tab) => {
                            self.switch_tab_delta(if shift { -1 } else { 1 });
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowLeft) => {
                            self.go_back_active();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowRight) => {
                            self.go_forward_active();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowUp) => {
                            let page = self.active_page_mut();
                            page.scroll_y = 0.0;
                            page.scroll_by(0.0, 0.0);
                            self.page_dirty = true;
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowDown) => {
                            let max_y = self.active_page().js_env.dom.borrow().layout.content_height;
                            let page = self.active_page_mut();
                            page.scroll_y = max_y;
                            page.scroll_by(0.0, 0.0);
                            self.page_dirty = true;
                            self.request_redraw();
                            return;
                        }
                        _ => {}
                    }
                    if let Key::Character(ref ch) = event.logical_key {
                        let lower = ch.to_ascii_lowercase();
                        match lower.as_str() {
                            "q" => {
                                self.persist_session();
                                event_loop.exit();
                                return;
                            }
                            "t" if shift => {
                                self.reopen_closed_tab();
                                self.request_redraw();
                                return;
                            }
                            "t" => {
                                self.open_new_tab("peedle://newtab");
                                self.chrome.omnibox_focused = true;
                                self.chrome.omnibox.clear();
                                self.refresh_suggestions();
                                self.request_redraw();
                                return;
                            }
                            "w" => {
                                if self.tabs.len() <= 1 {
                                    self.persist_session();
                                    event_loop.exit();
                                } else {
                                    let idx = self.active_tab;
                                    self.close_tab(idx);
                                    self.request_redraw();
                                }
                                return;
                            }
                            "l" => {
                                self.chrome.omnibox_focused = true;
                                self.chrome.omnibox.select_all();
                                self.chrome.find_open = false;
                                self.chrome.console_focused = false;
                                self.refresh_suggestions();
                                self.request_redraw();
                                return;
                            }
                            "a" => {
                                if self.chrome.omnibox_focused {
                                    self.chrome.omnibox.select_all();
                                } else if self.chrome.find_open {
                                    self.chrome.find.select_all();
                                } else if self.chrome.console_focused {
                                    self.chrome.console.select_all();
                                } else {
                                    self.active_page_mut().select_all_text();
                                    self.page_dirty = true;
                                }
                                self.request_redraw();
                                return;
                            }
                            "c" => {
                                let copied = if self.chrome.omnibox_focused {
                                    self.chrome.omnibox.selected_text()
                                } else if self.chrome.find_open {
                                    self.chrome.find.selected_text()
                                } else if self.chrome.console_focused {
                                    self.chrome.console.selected_text()
                                } else {
                                    let sel = self.active_page().selected_text();
                                    if !sel.is_empty() {
                                        sel
                                    } else {
                                        self.active_page().url.clone()
                                    }
                                };
                                write_clipboard(&copied);
                                return;
                            }
                            "x" => {
                                if self.chrome.omnibox_focused {
                                    write_clipboard(&self.chrome.omnibox.selected_text());
                                    if !self.chrome.omnibox.delete_selection() {
                                        self.chrome.omnibox.clear();
                                    }
                                    self.refresh_suggestions();
                                    self.request_redraw();
                                } else if self.chrome.find_open {
                                    write_clipboard(&self.chrome.find.selected_text());
                                    self.chrome.find.clear();
                                    self.run_find(0);
                                    self.request_redraw();
                                } else if self.chrome.console_focused {
                                    write_clipboard(&self.chrome.console.selected_text());
                                    self.chrome.console.clear();
                                    self.request_redraw();
                                }
                                return;
                            }
                            "v" if shift => {
                                self.toggle_vertical_tabs();
                                self.request_redraw();
                                return;
                            }
                            "v" => {
                                if let Some(clip) = read_clipboard() {
                                    self.paste_text(&clip);
                                    self.request_redraw();
                                }
                                return;
                            }
                            "r" if alt => {
                                self.toggle_reader_mode();
                                self.request_redraw();
                                return;
                            }
                            "r" => {
                                let u = self.active_page().url.clone();
                                let idx = self.active_tab;
                                self.start_navigation_for_tab(idx, &u, false);
                                self.request_redraw();
                                return;
                            }
                            "d" => {
                                self.toggle_current_bookmark();
                                self.request_redraw();
                                return;
                            }
                            "b" => {
                                self.chrome.bookmarks_visible = !self.chrome.bookmarks_visible;
                                self.profile.settings.bookmarks_visible = self.chrome.bookmarks_visible;
                                self.profile.save_settings();
                                self.resize_all_tabs();
                                self.request_redraw();
                                return;
                            }
                            "j" => {
                                self.open_new_tab("peedle://downloads");
                                self.request_redraw();
                                return;
                            }
                            "u" => {
                                self.execute_menu_action(MenuAction::ViewSource);
                                self.request_redraw();
                                return;
                            }
                            "s" if shift => {
                                self.toggle_split_view();
                                self.request_redraw();
                                return;
                            }
                            "s" => {
                                self.execute_menu_action(MenuAction::SavePage);
                                self.request_redraw();
                                return;
                            }
                            "o" if shift => {
                                self.open_new_tab("peedle://bookmarks");
                                self.request_redraw();
                                return;
                            }
                            "," => {
                                self.open_new_tab("peedle://settings");
                                self.request_redraw();
                                return;
                            }
                            "[" | "{" => {
                                if shift {
                                    self.switch_tab_delta(-1);
                                } else {
                                    self.go_back_active();
                                }
                                self.request_redraw();
                                return;
                            }
                            "]" | "}" => {
                                if shift {
                                    self.switch_tab_delta(1);
                                } else {
                                    self.go_forward_active();
                                }
                                self.request_redraw();
                                return;
                            }
                            "f" => {
                                self.chrome.find_open = true;
                                self.chrome.omnibox_focused = false;
                                self.chrome.suggestions.clear();
                                self.chrome.console_focused = false;
                                self.request_redraw();
                                return;
                            }
                            "g" => {
                                if self.chrome.find_open {
                                    self.run_find(if shift { -1 } else { 1 });
                                    self.request_redraw();
                                }
                                return;
                            }
                            "y" => {
                                self.open_new_tab("peedle://history");
                                self.request_redraw();
                                return;
                            }
                            "h" if !cfg!(target_os = "macos") => {
                                self.open_new_tab("peedle://history");
                                self.request_redraw();
                                return;
                            }
                            "=" | "+" => {
                                self.chrome.zoom = (self.chrome.zoom + 0.1).min(2.0);
                                self.resize_all_tabs();
                                self.request_redraw();
                                return;
                            }
                            "-" => {
                                self.chrome.zoom = (self.chrome.zoom - 0.1).max(0.5);
                                self.resize_all_tabs();
                                self.request_redraw();
                                return;
                            }
                            "0" => {
                                self.chrome.zoom = 1.0;
                                self.resize_all_tabs();
                                self.request_redraw();
                                return;
                            }
                            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" => {
                                if let Ok(n) = lower.parse::<usize>() {
                                    let idx = if n == 9 {
                                        self.tabs.len().saturating_sub(1)
                                    } else {
                                        n.saturating_sub(1)
                                    };
                                    if idx < self.tabs.len() {
                                        self.active_tab = idx;
                                        self.page_dirty = true;
                                        self.sync_omnibox();
                                        self.sync_window_title();
                                        self.request_redraw();
                                    }
                                }
                                return;
                            }
                            _ => return,
                        }
                    }
                    return;
                }

                if matches!(event.logical_key, Key::Named(NamedKey::F11)) {
                    self.toggle_window_fullscreen();
                    self.request_redraw();
                    return;
                }
                if matches!(event.logical_key, Key::Named(NamedKey::F12)) {
                    self.chrome.devtools_open = !self.chrome.devtools_open;
                    self.resize_all_tabs();
                    self.request_redraw();
                    return;
                }
                if matches!(event.logical_key, Key::Named(NamedKey::F5)) {
                    let u = self.active_page().url.clone();
                    let idx = self.active_tab;
                    self.start_navigation_for_tab(idx, &u, false);
                    self.request_redraw();
                    return;
                }

                if self.chrome.find_open {
                    match &event.logical_key {
                        Key::Named(NamedKey::Escape) => {
                            self.chrome.find_open = false;
                            self.page_dirty = true;
                        }
                        Key::Named(NamedKey::Enter) => {
                            self.run_find(if shift { -1 } else { 1 });
                        }
                        Key::Named(NamedKey::Backspace) => {
                            self.chrome.find.backspace(false);
                            self.run_find(0);
                        }
                        Key::Named(NamedKey::Delete) => {
                            self.chrome.find.delete(false);
                            self.run_find(0);
                        }
                        _ => {
                            if let Some(txt) = &event.text {
                                self.chrome.find.insert_str(txt);
                                self.run_find(0);
                            }
                        }
                    }
                    self.request_redraw();
                    return;
                }

                if self.chrome.console_focused {
                    match &event.logical_key {
                        Key::Named(NamedKey::Enter) => {
                            let code = self.chrome.console.text.clone();
                            self.chrome.console.clear();
                            if !code.trim().is_empty() {
                                let _ = self.active_page_mut().eval_js(&code);
                                self.page_dirty = true;
                                self.sync_window_title();
                            }
                        }
                        Key::Named(NamedKey::Escape) => {
                            self.chrome.console_focused = false;
                        }
                        Key::Named(NamedKey::Backspace) => {
                            self.chrome.console.backspace(false);
                        }
                        Key::Named(NamedKey::Delete) => {
                            self.chrome.console.delete(false);
                        }
                        Key::Named(NamedKey::ArrowLeft) => self.chrome.console.move_left(shift, false),
                        Key::Named(NamedKey::ArrowRight) => self.chrome.console.move_right(shift, false),
                        _ => {
                            if let Some(txt) = &event.text {
                                self.chrome.console.insert_str(txt);
                            }
                        }
                    }
                    self.request_redraw();
                    return;
                }

                if self.chrome.omnibox_focused {
                    match &event.logical_key {
                        Key::Named(NamedKey::Enter) => {
                            let target = if let Some(idx) = self.chrome.suggestion_index {
                                self.chrome
                                    .suggestions
                                    .get(idx)
                                    .map(|s| s.url.clone())
                                    .unwrap_or_else(|| self.chrome.omnibox.text.clone())
                            } else {
                                self.chrome.omnibox.text.clone()
                            };
                            self.chrome.omnibox_focused = false;
                            self.chrome.suggestions.clear();
                            self.chrome.suggestion_index = None;
                            self.navigate_active(&target);
                        }
                        Key::Named(NamedKey::Escape) => {
                            self.chrome.omnibox_focused = false;
                            self.chrome.suggestions.clear();
                            self.sync_omnibox();
                        }
                        Key::Named(NamedKey::ArrowDown) => {
                            if !self.chrome.suggestions.is_empty() {
                                let next = match self.chrome.suggestion_index {
                                    Some(i) => (i + 1) % self.chrome.suggestions.len(),
                                    None => 0,
                                };
                                self.chrome.suggestion_index = Some(next);
                                if let Some(s) = self.chrome.suggestions.get(next) {
                                    if s.kind != SuggestionKind::Search {
                                        self.chrome.omnibox.set_text(&s.url);
                                    }
                                }
                            }
                        }
                        Key::Named(NamedKey::ArrowUp) => {
                            if !self.chrome.suggestions.is_empty() {
                                let prev = match self.chrome.suggestion_index {
                                    Some(0) | None => self.chrome.suggestions.len() - 1,
                                    Some(i) => i - 1,
                                };
                                self.chrome.suggestion_index = Some(prev);
                                if let Some(s) = self.chrome.suggestions.get(prev) {
                                    if s.kind != SuggestionKind::Search {
                                        self.chrome.omnibox.set_text(&s.url);
                                    }
                                }
                            }
                        }
                        Key::Named(NamedKey::Tab) => {
                            if let Some(first) = self.chrome.suggestions.first() {
                                if first.kind != SuggestionKind::Search {
                                    self.chrome.omnibox.set_text(&first.url);
                                    self.refresh_suggestions();
                                }
                            }
                        }
                        Key::Named(NamedKey::Backspace) => {
                            self.chrome.omnibox.backspace(false);
                            self.refresh_suggestions();
                        }
                        Key::Named(NamedKey::Delete) => {
                            self.chrome.omnibox.delete(false);
                            self.refresh_suggestions();
                        }
                        Key::Named(NamedKey::ArrowLeft) => self.chrome.omnibox.move_left(shift, false),
                        Key::Named(NamedKey::ArrowRight) => self.chrome.omnibox.move_right(shift, false),
                        Key::Named(NamedKey::Home) => self.chrome.omnibox.move_home(shift),
                        Key::Named(NamedKey::End) => self.chrome.omnibox.move_end(shift),
                        _ => {
                            if let Some(txt) = &event.text {
                                self.chrome.omnibox.insert_str(txt);
                                self.refresh_suggestions();
                            }
                        }
                    }
                    self.request_redraw();
                    return;
                }

                let page_input = self.active_page().has_focused_input();
                if !page_input {
                    let media_key_str = match &event.logical_key {
                        Key::Named(NamedKey::Space) => Some(" "),
                        Key::Named(NamedKey::ArrowLeft) => Some("ArrowLeft"),
                        Key::Named(NamedKey::ArrowRight) => Some("ArrowRight"),
                        Key::Character(ch) => Some(ch.as_str()),
                        _ => None,
                    };
                    if let Some(mk) = media_key_str {
                        if self.active_page_mut().handle_media_key(mk) {
                            self.page_dirty = true;
                            self.request_redraw();
                            return;
                        }
                    }
                }
                match &event.logical_key {
                    Key::Named(NamedKey::Tab) => {
                        if self.active_page_mut().focus_next_input(shift) {
                            self.page_dirty = true;
                        }
                    }
                    Key::Named(NamedKey::PageDown) => {
                        self.scroll_vel_y += 380.0;
                    }
                    Key::Named(NamedKey::PageUp) => {
                        self.scroll_vel_y -= 380.0;
                    }
                    Key::Named(NamedKey::Home) if !page_input => {
                        let page = self.active_page_mut();
                        page.scroll_y = 0.0;
                        page.scroll_by(0.0, 0.0);
                        self.page_dirty = true;
                    }
                    Key::Named(NamedKey::End) if !page_input => {
                        let max_y = self.active_page().js_env.dom.borrow().layout.content_height;
                        let page = self.active_page_mut();
                        page.scroll_y = max_y;
                        page.scroll_by(0.0, 0.0);
                        self.page_dirty = true;
                    }
                    Key::Named(NamedKey::Space) if !page_input => {
                        let step = (content_viewport_height(self.logical_h(), &self.chrome) * 0.85)
                            .max(200.0);
                        self.scroll_vel_y += if shift { -step } else { step };
                    }
                    Key::Named(NamedKey::ArrowDown) => {
                        self.active_page_mut().scroll_by(0.0, 52.0);
                        self.page_dirty = true;
                    }
                    Key::Named(NamedKey::ArrowUp) => {
                        self.active_page_mut().scroll_by(0.0, -52.0);
                        self.page_dirty = true;
                    }
                    Key::Named(NamedKey::Backspace) => {
                        if self.active_page_mut().on_text_input('\u{0008}') {
                            self.page_dirty = true;
                        }
                    }
                    Key::Named(NamedKey::Enter) => {
                        let prev_url = self.active_page().url.clone();
                        if self.active_page_mut().on_text_input('\n') {
                            self.page_dirty = true;
                            let new_url = self.active_page().url.clone();
                            if new_url != prev_url {
                                let title = self.active_page().title();
                                self.profile.record_visit(&new_url, &title);
                                self.sync_omnibox();
                            }
                            self.sync_window_title();
                        }
                    }
                    _ => {
                        if let Some(txt) = &event.text {
                            for c in txt.chars() {
                                if self.active_page_mut().on_text_input(c) {
                                    self.page_dirty = true;
                                }
                            }
                        }
                    }
                }
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                self.render();
            }
            _ => {}
        }
    }
}

fn read_clipboard() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let out = Command::new("pbpaste").output().ok()?;
        if out.status.success() {
            return String::from_utf8(out.stdout).ok();
        }
        return None;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let out = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; Get-Clipboard -Raw",
            ])
            .creation_flags(0x08000000)
            .output()
            .ok()?;
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).to_string();
            return Some(s.trim_end_matches(&['\r', '\n'][..]).to_string());
        }
        return None;
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        for (bin, args) in [
            ("wl-paste", &["-n"][..]),
            ("xclip", &["-selection", "clipboard", "-o"][..]),
            ("xsel", &["--clipboard", "--output"][..]),
        ] {
            if let Ok(out) = Command::new(bin).args(args).output() {
                if out.status.success() {
                    if let Ok(s) = String::from_utf8(out.stdout) {
                        return Some(s);
                    }
                }
            }
        }
        None
    }
}

fn write_clipboard(text: &str) {
    #[cfg(target_os = "macos")]
    {
        if let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
        }
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        if let Ok(mut child) = Command::new("clip.exe")
            .stdin(Stdio::piped())
            .creation_flags(0x08000000)
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
        }
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        for (bin, args) in [
            ("wl-copy", &[][..]),
            ("xclip", &["-selection", "clipboard"][..]),
            ("xsel", &["--clipboard", "--input"][..]),
        ] {
            if let Ok(mut child) = Command::new(bin).args(args).stdin(Stdio::piped()).spawn() {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                if child.wait().map_or(false, |s| s.success()) {
                    return;
                }
            }
        }
    }
}
