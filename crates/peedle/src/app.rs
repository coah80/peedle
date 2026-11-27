use std::io::Write;
use std::num::NonZeroU32;
use std::process::{Command, Stdio};
use std::rc::Rc;

use peedle_engine::Page;
use peedle_render::{ClipRect, FontSystem, Framebuffer, RgbaColor};
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowAttributes, WindowId};

use crate::chrome::{
    content_viewport_height, draw_chrome, hit_test_chrome, top_chrome_height, Bookmark, ChromeHit,
    ChromeState, DevToolsTab,
};

pub struct BrowserApp {
    initial_urls: Vec<String>,
    window: Option<Rc<Window>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    tabs: Vec<Page>,
    active_tab: usize,
    chrome: ChromeState,
    fonts: FontSystem,
    cursor_x: f32,
    cursor_y: f32,
    modifiers: ModifiersState,
    scrollbar_dragging: bool,
    phys_w: u32,
    phys_h: u32,
    scale_factor: f32,
}

impl BrowserApp {
    pub fn run(initial_urls: Vec<String>) -> Result<(), String> {
        let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
        let mut app = Self {
            initial_urls,
            window: None,
            surface: None,
            tabs: Vec::new(),
            active_tab: 0,
            chrome: ChromeState::default(),
            fonts: FontSystem::load_system_fonts(),
            cursor_x: 0.0,
            cursor_y: 0.0,
            modifiers: ModifiersState::empty(),
            scrollbar_dragging: false,
            phys_w: 1280,
            phys_h: 800,
            scale_factor: 1.0,
        };
        event_loop.run_app(&mut app).map_err(|e| e.to_string())
    }

    fn logical_w(&self) -> f32 {
        (self.phys_w as f32 / self.scale_factor.max(1.0)).max(200.0)
    }

    fn logical_h(&self) -> f32 {
        (self.phys_h as f32 / self.scale_factor.max(1.0)).max(200.0)
    }

    fn active_page(&self) -> &Page {
        &self.tabs[self.active_tab]
    }

    fn active_page_mut(&mut self) -> &mut Page {
        &mut self.tabs[self.active_tab]
    }

    fn sync_omnibox(&mut self) {
        self.chrome.omnibox_text = self.active_page().url.clone();
        self.chrome.omnibox_selected = false;
    }

    fn set_loading_title(&self) {
        if let Some(ref win) = self.window {
            win.set_title("Loading... - Peedle");
        }
    }

    fn sync_window_title(&self) {
        if let Some(ref win) = self.window {
            let t = self.active_page().title();
            win.set_title(&format!("{} - Peedle", t));
        }
    }

    fn navigate_active(&mut self, target: &str) {
        self.set_loading_title();
        let _ = self.active_page_mut().navigate(target);
        self.sync_omnibox();
        self.sync_window_title();
    }

    fn toggle_current_bookmark(&mut self) {
        let url = self.active_page().url.clone();
        if let Some(pos) = self.chrome.bookmarks.iter().position(|b| b.url == url) {
            self.chrome.bookmarks.remove(pos);
        } else {
            let title = self.active_page().title();
            self.chrome.bookmarks.push(Bookmark { title, url });
        }
    }

    fn run_find(&mut self, delta: i32) {
        let q = self.chrome.find_query.trim().to_ascii_lowercase();
        if q.is_empty() {
            self.chrome.find_match_count = 0;
            self.chrome.find_match_index = 0;
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
    }

    fn open_new_tab(&mut self, url: &str) {
        let z = self.chrome.zoom.clamp(0.5, 2.0);
        let vw = (self.logical_w() / z).max(200.0);
        let vh = (content_viewport_height(self.logical_h(), &self.chrome) / z).max(100.0);
        if url != "peedle://newtab" {
            self.set_loading_title();
        }
        let mut page = Page::new(vw, vh);
        if url != "peedle://newtab" {
            let _ = page.navigate(url);
        }
        self.tabs.push(page);
        self.active_tab = self.tabs.len() - 1;
        self.sync_omnibox();
        self.sync_window_title();
    }

    fn close_tab(&mut self, idx: usize) {
        if self.tabs.len() <= 1 {
            self.navigate_active("peedle://newtab");
            return;
        }
        self.tabs.remove(idx);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        self.sync_omnibox();
        self.sync_window_title();
    }

    fn switch_tab_delta(&mut self, delta: isize) {
        if self.tabs.len() <= 1 {
            return;
        }
        let n = self.tabs.len() as isize;
        self.active_tab = (self.active_tab as isize + delta).rem_euclid(n) as usize;
        self.sync_omnibox();
        self.sync_window_title();
    }

    fn resize_all_tabs(&mut self) {
        let z = self.chrome.zoom.clamp(0.5, 2.0);
        let w = (self.logical_w() / z).max(200.0);
        let vh = (content_viewport_height(self.logical_h(), &self.chrome) / z).max(100.0);
        for tab in &mut self.tabs {
            tab.resize(w, vh);
        }
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
    }

    fn update_cursor_icon(&self) {
        let Some(ref win) = self.window else {
            return;
        };
        let hit = hit_test_chrome(
            self.cursor_x,
            self.cursor_y,
            self.logical_w(),
            self.logical_h(),
            self.tabs.len(),
            &self.chrome,
        );
        let icon = match hit {
            ChromeHit::Tab(_)
            | ChromeHit::CloseTab(_)
            | ChromeHit::NewTab
            | ChromeHit::Back
            | ChromeHit::Forward
            | ChromeHit::Reload
            | ChromeHit::ToggleBookmark
            | ChromeHit::BookmarkItem(_)
            | ChromeHit::ToggleDevTools
            | ChromeHit::DevToolsTab(_) => CursorIcon::Pointer,
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
            ChromeHit::Scrollbar | ChromeHit::None => CursorIcon::Default,
        };
        win.set_cursor(icon);
    }

    fn paste_text(&mut self, raw: &str) {
        let cleaned: String = raw
            .chars()
            .filter(|c| !c.is_control() || *c == '\t')
            .collect();
        if cleaned.is_empty() {
            return;
        }
        if self.chrome.omnibox_focused {
            if self.chrome.omnibox_selected {
                self.chrome.omnibox_text.clear();
                self.chrome.omnibox_selected = false;
            }
            self.chrome.omnibox_text.push_str(cleaned.trim());
        } else if self.chrome.find_open {
            self.chrome.find_query.push_str(&cleaned);
            self.run_find(0);
        } else if self.chrome.console_focused {
            self.chrome.console_input.push_str(&cleaned);
        } else if self.active_page().has_focused_input() {
            for c in cleaned.chars() {
                self.active_page_mut().on_text_input(c);
            }
        }
    }

    fn render(&mut self) {
        if self.phys_w == 0 || self.phys_h == 0 || self.tabs.is_empty() {
            return;
        }
        let s = self.scale_factor.max(1.0);
        let lw = self.logical_w();
        let lh = self.logical_h();
        let top_h = top_chrome_height(&self.chrome);
        let vh = content_viewport_height(lh, &self.chrome);
        let z = self.chrome.zoom.clamp(0.5, 2.0);
        let pw = (lw / z).ceil().max(1.0) as u32;
        let ph = (vh / z).ceil().max(1.0) as u32;
        let mut page_fb = Framebuffer::new(pw, ph);
        self.tabs[self.active_tab].render_to_framebuffer(&mut page_fb);

        if self.chrome.find_open && !self.chrome.find_query.trim().is_empty() {
            let q = self.chrome.find_query.trim().to_ascii_lowercase();
            let page = &self.tabs[self.active_tab];
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
                        page_fb.fill_rounded_rect(
                            frag.x - sx - 1.0,
                            frag.y - sy,
                            frag.width + 2.0,
                            frag.height,
                            [2.0; 4],
                            color,
                        );
                        hit_idx += 1;
                    }
                }
            }
        }

        let mut full_fb = Framebuffer::new(self.phys_w, self.phys_h);
        let top_phys = top_h * s;
        let vh_phys = vh * s;
        full_fb.push_clip(ClipRect {
            x: 0,
            y: top_phys.floor() as i32,
            w: self.phys_w as i32,
            h: vh_phys.ceil() as i32,
        });
        full_fb.blit_rgba_opaque(
            page_fb.width,
            page_fb.height,
            &page_fb.data,
            0.0,
            top_phys,
            self.phys_w as f32,
            vh_phys,
        );
        full_fb.pop_clip();

        draw_chrome(
            &mut full_fb,
            &self.fonts,
            &self.tabs,
            self.active_tab,
            &self.chrome,
            s,
        );

        if let Some(ref mut surface) = self.surface {
            if let (Some(w), Some(h)) = (NonZeroU32::new(self.phys_w), NonZeroU32::new(self.phys_h)) {
                let _ = surface.resize(w, h);
                if let Ok(mut buffer) = surface.buffer_mut() {
                    full_fb.to_xrgb_slice(&mut buffer);
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
}

impl ApplicationHandler for BrowserApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let mut attrs = WindowAttributes::default()
            .with_title("Peedle")
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
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

        let urls = self.initial_urls.clone();
        for url in &urls {
            self.open_new_tab(url);
        }
        if self.tabs.is_empty() {
            self.open_new_tab("peedle://newtab");
        }
        self.render();
        self.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
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
                if self.scrollbar_dragging {
                    self.scroll_to_scrollbar_y(self.cursor_y);
                    self.request_redraw();
                    return;
                }
                let z = self.chrome.zoom.clamp(0.5, 2.0);
                let cx = self.cursor_x / z;
                let top_h = top_chrome_height(&self.chrome);
                if self.cursor_y >= top_h {
                    let vy = (self.cursor_y - top_h) / z;
                    let vh = content_viewport_height(self.logical_h(), &self.chrome) / z;
                    if vy < vh && self.active_page_mut().on_mouse_move(cx, vy) {
                        self.request_redraw();
                    }
                }
                self.update_cursor_icon();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let s = self.scale_factor.max(1.0);
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (-x * 40.0, -y * 48.0),
                    MouseScrollDelta::PixelDelta(p) => (-p.x as f32 / s, -p.y as f32 / s),
                };
                self.active_page_mut().scroll_by(dx, dy);
                self.request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.scrollbar_dragging = false;
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Middle,
                ..
            } => {
                let hit = hit_test_chrome(
                    self.cursor_x,
                    self.cursor_y,
                    self.logical_w(),
                    self.logical_h(),
                    self.tabs.len(),
                    &self.chrome,
                );
                match hit {
                    ChromeHit::Tab(i) => {
                        self.close_tab(i);
                        self.request_redraw();
                    }
                    ChromeHit::Content => {
                        let z = self.chrome.zoom.clamp(0.5, 2.0);
                        let cx = self.cursor_x / z;
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
                let hit = hit_test_chrome(
                    self.cursor_x,
                    self.cursor_y,
                    self.logical_w(),
                    self.logical_h(),
                    self.tabs.len(),
                    &self.chrome,
                );
                let was_omni = self.chrome.omnibox_focused;
                self.chrome.omnibox_focused = false;
                self.chrome.omnibox_selected = false;
                self.chrome.console_focused = false;
                match hit {
                    ChromeHit::Tab(i) => {
                        self.active_tab = i;
                        self.sync_omnibox();
                        self.sync_window_title();
                    }
                    ChromeHit::CloseTab(i) => self.close_tab(i),
                    ChromeHit::NewTab => {
                        self.open_new_tab("peedle://newtab");
                        self.chrome.omnibox_focused = true;
                        self.chrome.omnibox_selected = true;
                    }
                    ChromeHit::Back => {
                        self.set_loading_title();
                        let _ = self.active_page_mut().go_back();
                        self.sync_omnibox();
                        self.sync_window_title();
                    }
                    ChromeHit::Forward => {
                        self.set_loading_title();
                        let _ = self.active_page_mut().go_forward();
                        self.sync_omnibox();
                        self.sync_window_title();
                    }
                    ChromeHit::Reload => {
                        self.set_loading_title();
                        let _ = self.active_page_mut().reload();
                        self.sync_window_title();
                    }
                    ChromeHit::Omnibox => {
                        self.chrome.omnibox_focused = true;
                        self.chrome.omnibox_selected = !was_omni;
                    }
                    ChromeHit::ToggleBookmark => {
                        self.toggle_current_bookmark();
                    }
                    ChromeHit::BookmarkItem(i) => {
                        if let Some(bm) = self.chrome.bookmarks.get(i).cloned() {
                            self.navigate_active(&bm.url);
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
                    ChromeHit::Scrollbar => {
                        self.scrollbar_dragging = true;
                        self.scroll_to_scrollbar_y(self.cursor_y);
                    }
                    ChromeHit::Content => {
                        let z = self.chrome.zoom.clamp(0.5, 2.0);
                        let cx = self.cursor_x / z;
                        let vy = (self.cursor_y - top_chrome_height(&self.chrome)) / z;
                        let cmd = self.modifiers.super_key() || self.modifiers.control_key();
                        if cmd {
                            if let Some(url) = self.active_page().href_at(cx, vy) {
                                self.open_new_tab(&url);
                                self.request_redraw();
                                return;
                            }
                        }
                        if self.active_page().href_at(cx, vy).is_some() {
                            self.set_loading_title();
                        }
                        if self.active_page_mut().on_click(cx, vy).is_some() {
                            self.sync_omnibox();
                            self.sync_window_title();
                        } else {
                            self.sync_window_title();
                        }
                    }
                    ChromeHit::None => {}
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

                if alt && !cmd {
                    match &event.logical_key {
                        Key::Named(NamedKey::ArrowLeft) => {
                            self.set_loading_title();
                            let _ = self.active_page_mut().go_back();
                            self.sync_omnibox();
                            self.sync_window_title();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowRight) => {
                            self.set_loading_title();
                            let _ = self.active_page_mut().go_forward();
                            self.sync_omnibox();
                            self.sync_window_title();
                            self.request_redraw();
                            return;
                        }
                        Key::Character(ch) if ch.eq_ignore_ascii_case("d") => {
                            self.chrome.omnibox_focused = true;
                            self.chrome.omnibox_selected = true;
                            self.chrome.find_open = false;
                            self.chrome.console_focused = false;
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
                    match event.logical_key {
                        Key::Named(NamedKey::Tab) => {
                            self.switch_tab_delta(if shift { -1 } else { 1 });
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowLeft) => {
                            self.set_loading_title();
                            let _ = self.active_page_mut().go_back();
                            self.sync_omnibox();
                            self.sync_window_title();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowRight) => {
                            self.set_loading_title();
                            let _ = self.active_page_mut().go_forward();
                            self.sync_omnibox();
                            self.sync_window_title();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowUp) => {
                            let page = self.active_page_mut();
                            page.scroll_y = 0.0;
                            page.scroll_by(0.0, 0.0);
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowDown) => {
                            let max_y = self.active_page().js_env.dom.borrow().layout.content_height;
                            let page = self.active_page_mut();
                            page.scroll_y = max_y;
                            page.scroll_by(0.0, 0.0);
                            self.request_redraw();
                            return;
                        }
                        _ => {}
                    }
                    if let Key::Character(ref ch) = event.logical_key {
                        let lower = ch.to_ascii_lowercase();
                        match lower.as_str() {
                            "q" => {
                                event_loop.exit();
                                return;
                            }
                            "t" => {
                                self.open_new_tab("peedle://newtab");
                                self.chrome.omnibox_focused = true;
                                self.chrome.omnibox_selected = false;
                                self.chrome.omnibox_text.clear();
                                self.request_redraw();
                                return;
                            }
                            "w" => {
                                if self.tabs.len() <= 1 {
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
                                self.chrome.omnibox_selected = true;
                                self.chrome.find_open = false;
                                self.chrome.console_focused = false;
                                self.request_redraw();
                                return;
                            }
                            "a" => {
                                if self.chrome.omnibox_focused {
                                    self.chrome.omnibox_selected = true;
                                    self.request_redraw();
                                }
                                return;
                            }
                            "c" => {
                                let copied = if self.chrome.omnibox_focused {
                                    self.chrome.omnibox_text.clone()
                                } else if self.chrome.find_open {
                                    self.chrome.find_query.clone()
                                } else if self.chrome.console_focused {
                                    self.chrome.console_input.clone()
                                } else {
                                    self.active_page().url.clone()
                                };
                                write_clipboard(&copied);
                                return;
                            }
                            "x" => {
                                if self.chrome.omnibox_focused {
                                    write_clipboard(&self.chrome.omnibox_text);
                                    self.chrome.omnibox_text.clear();
                                    self.chrome.omnibox_selected = false;
                                    self.request_redraw();
                                } else if self.chrome.find_open {
                                    write_clipboard(&self.chrome.find_query);
                                    self.chrome.find_query.clear();
                                    self.run_find(0);
                                    self.request_redraw();
                                } else if self.chrome.console_focused {
                                    write_clipboard(&self.chrome.console_input);
                                    self.chrome.console_input.clear();
                                    self.request_redraw();
                                }
                                return;
                            }
                            "v" => {
                                if let Some(clip) = read_clipboard() {
                                    self.paste_text(&clip);
                                    self.request_redraw();
                                }
                                return;
                            }
                            "r" => {
                                self.set_loading_title();
                                let _ = self.active_page_mut().reload();
                                self.sync_window_title();
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
                                self.resize_all_tabs();
                                self.request_redraw();
                                return;
                            }
                            "i" if alt => {
                                self.chrome.devtools_open = !self.chrome.devtools_open;
                                self.resize_all_tabs();
                                self.request_redraw();
                                return;
                            }
                            "[" | "{" => {
                                if shift {
                                    self.switch_tab_delta(-1);
                                } else {
                                    self.set_loading_title();
                                    let _ = self.active_page_mut().go_back();
                                    self.sync_omnibox();
                                    self.sync_window_title();
                                }
                                self.request_redraw();
                                return;
                            }
                            "]" | "}" => {
                                if shift {
                                    self.switch_tab_delta(1);
                                } else {
                                    self.set_loading_title();
                                    let _ = self.active_page_mut().go_forward();
                                    self.sync_omnibox();
                                    self.sync_window_title();
                                }
                                self.request_redraw();
                                return;
                            }
                            "f" => {
                                self.chrome.find_open = true;
                                self.chrome.omnibox_focused = false;
                                self.chrome.omnibox_selected = false;
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
                                self.navigate_active("peedle://history");
                                self.request_redraw();
                                return;
                            }
                            "h" if !cfg!(target_os = "macos") => {
                                self.navigate_active("peedle://history");
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

                if matches!(event.logical_key, Key::Named(NamedKey::F12)) {
                    self.chrome.devtools_open = !self.chrome.devtools_open;
                    self.resize_all_tabs();
                    self.request_redraw();
                    return;
                }
                if matches!(event.logical_key, Key::Named(NamedKey::F5)) {
                    self.set_loading_title();
                    let _ = self.active_page_mut().reload();
                    self.sync_window_title();
                    self.request_redraw();
                    return;
                }

                if self.chrome.find_open {
                    match &event.logical_key {
                        Key::Named(NamedKey::Escape) => {
                            self.chrome.find_open = false;
                        }
                        Key::Named(NamedKey::Enter) => {
                            let step = if shift { -1 } else { 1 };
                            self.run_find(step);
                        }
                        Key::Named(NamedKey::Backspace) => {
                            self.chrome.find_query.pop();
                            self.run_find(0);
                        }
                        _ => {
                            if let Some(txt) = &event.text {
                                for c in txt.chars() {
                                    if !c.is_control() {
                                        self.chrome.find_query.push(c);
                                    }
                                }
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
                            let code = std::mem::take(&mut self.chrome.console_input);
                            if !code.trim().is_empty() {
                                let _ = self.active_page_mut().eval_js(&code);
                                self.sync_window_title();
                            }
                        }
                        Key::Named(NamedKey::Escape) => {
                            self.chrome.console_focused = false;
                        }
                        Key::Named(NamedKey::Backspace) => {
                            self.chrome.console_input.pop();
                        }
                        _ => {
                            if let Some(txt) = &event.text {
                                for c in txt.chars() {
                                    if !c.is_control() {
                                        self.chrome.console_input.push(c);
                                    }
                                }
                            }
                        }
                    }
                    self.request_redraw();
                    return;
                }

                if self.chrome.omnibox_focused {
                    match &event.logical_key {
                        Key::Named(NamedKey::Enter) => {
                            let target = self.chrome.omnibox_text.clone();
                            self.chrome.omnibox_focused = false;
                            self.chrome.omnibox_selected = false;
                            self.navigate_active(&target);
                        }
                        Key::Named(NamedKey::Escape) => {
                            self.chrome.omnibox_focused = false;
                            self.chrome.omnibox_selected = false;
                            self.sync_omnibox();
                        }
                        Key::Named(NamedKey::Backspace) => {
                            if self.chrome.omnibox_selected {
                                self.chrome.omnibox_text.clear();
                                self.chrome.omnibox_selected = false;
                            } else {
                                self.chrome.omnibox_text.pop();
                            }
                        }
                        Key::Named(NamedKey::ArrowLeft) | Key::Named(NamedKey::ArrowRight) => {
                            self.chrome.omnibox_selected = false;
                        }
                        _ => {
                            if let Some(txt) = &event.text {
                                let mut pushed = false;
                                for c in txt.chars() {
                                    if !c.is_control() {
                                        if self.chrome.omnibox_selected {
                                            self.chrome.omnibox_text.clear();
                                            self.chrome.omnibox_selected = false;
                                        }
                                        self.chrome.omnibox_text.push(c);
                                        pushed = true;
                                    }
                                }
                                let _ = pushed;
                            }
                        }
                    }
                    self.request_redraw();
                    return;
                }

                let page_input = self.active_page().has_focused_input();
                match &event.logical_key {
                    Key::Named(NamedKey::PageDown) => self.active_page_mut().scroll_by(0.0, 480.0),
                    Key::Named(NamedKey::PageUp) => self.active_page_mut().scroll_by(0.0, -480.0),
                    Key::Named(NamedKey::Home) if !page_input => {
                        let page = self.active_page_mut();
                        page.scroll_y = 0.0;
                        page.scroll_by(0.0, 0.0);
                    }
                    Key::Named(NamedKey::End) if !page_input => {
                        let max_y = self.active_page().js_env.dom.borrow().layout.content_height;
                        let page = self.active_page_mut();
                        page.scroll_y = max_y;
                        page.scroll_by(0.0, 0.0);
                    }
                    Key::Named(NamedKey::Space) if !page_input => {
                        let step = (content_viewport_height(self.logical_h(), &self.chrome) * 0.85)
                            .max(200.0);
                        self.active_page_mut()
                            .scroll_by(0.0, if shift { -step } else { step });
                    }
                    Key::Named(NamedKey::ArrowDown) => self.active_page_mut().scroll_by(0.0, 48.0),
                    Key::Named(NamedKey::ArrowUp) => self.active_page_mut().scroll_by(0.0, -48.0),
                    Key::Named(NamedKey::Backspace) => {
                        self.active_page_mut().on_text_input('\u{0008}');
                    }
                    Key::Named(NamedKey::Enter) => {
                        if self.active_page_mut().on_text_input('\n') {
                            self.sync_omnibox();
                            self.sync_window_title();
                        }
                    }
                    _ => {
                        if let Some(txt) = &event.text {
                            for c in txt.chars() {
                                self.active_page_mut().on_text_input(c);
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
