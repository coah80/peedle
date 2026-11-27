use std::num::NonZeroU32;
use std::rc::Rc;

use peedle_engine::Page;
use peedle_render::{ClipRect, FontSystem, Framebuffer, RgbaColor};
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Window, WindowAttributes, WindowId};

use crate::chrome::{
    content_viewport_height, draw_chrome, hit_test_chrome, top_chrome_height, Bookmark, ChromeHit,
    ChromeState, DevToolsTab,
};

pub struct BrowserApp {
    initial_url: String,
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
    width: u32,
    height: u32,
}

impl BrowserApp {
    pub fn run(initial_url: String) -> Result<(), String> {
        let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
        let mut app = Self {
            initial_url,
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
            width: 1280,
            height: 800,
        };
        event_loop.run_app(&mut app).map_err(|e| e.to_string())
    }

    fn active_page(&self) -> &Page {
        &self.tabs[self.active_tab]
    }

    fn active_page_mut(&mut self) -> &mut Page {
        &mut self.tabs[self.active_tab]
    }

    fn sync_omnibox(&mut self) {
        self.chrome.omnibox_text = self.active_page().url.clone();
    }

    fn sync_window_title(&self) {
        if let Some(ref win) = self.window {
            let t = self.active_page().title();
            win.set_title(&format!("{} - Peedle", t));
        }
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
        let vw = (self.width as f32 / z).max(200.0);
        let vh = (content_viewport_height(self.height as f32, &self.chrome) / z).max(100.0);
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
            let _ = self.active_page_mut().navigate("peedle://newtab");
            self.sync_omnibox();
            return;
        }
        self.tabs.remove(idx);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        self.sync_omnibox();
        self.sync_window_title();
    }

    fn resize_all_tabs(&mut self) {
        let z = self.chrome.zoom.clamp(0.5, 2.0);
        let w = (self.width as f32 / z).max(200.0);
        let vh = (content_viewport_height(self.height as f32, &self.chrome) / z).max(100.0);
        for tab in &mut self.tabs {
            tab.resize(w, vh);
        }
    }

    fn scroll_to_scrollbar_y(&mut self, y: f32) {
        let top_h = top_chrome_height(&self.chrome);
        let vh = content_viewport_height(self.height as f32, &self.chrome);
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
            self.width as f32,
            self.height as f32,
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

    fn render(&mut self) {
        if self.width == 0 || self.height == 0 || self.tabs.is_empty() {
            return;
        }
        let top_h = top_chrome_height(&self.chrome);
        let vh = content_viewport_height(self.height as f32, &self.chrome);
        let z = self.chrome.zoom.clamp(0.5, 2.0);
        let pw = (self.width as f32 / z).ceil().max(1.0) as u32;
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

        let mut full_fb = Framebuffer::new(self.width, self.height);
        full_fb.push_clip(ClipRect {
            x: 0,
            y: top_h as i32,
            w: self.width as i32,
            h: vh.ceil() as i32,
        });
        full_fb.blit_rgba(
            page_fb.width,
            page_fb.height,
            &page_fb.data,
            0.0,
            top_h,
            self.width as f32,
            vh,
            1.0,
        );
        full_fb.pop_clip();

        draw_chrome(&mut full_fb, &self.fonts, &self.tabs, self.active_tab, &self.chrome);

        if let Some(ref mut surface) = self.surface {
            if let (Some(w), Some(h)) = (NonZeroU32::new(self.width), NonZeroU32::new(self.height)) {
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
        let attrs = WindowAttributes::default()
            .with_title("Peedle")
            .with_inner_size(LogicalSize::new(self.width, self.height));
        let win = Rc::new(event_loop.create_window(attrs).unwrap());
        let ctx = Context::new(win.clone()).unwrap();
        let surface = Surface::new(&ctx, win.clone()).unwrap();

        let sz = win.inner_size();
        if sz.width > 0 && sz.height > 0 {
            self.width = sz.width;
            self.height = sz.height;
        }
        self.window = Some(win);
        self.surface = Some(surface);

        let url = self.initial_url.clone();
        self.open_new_tab(&url);
        self.render();
        self.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(sz) => {
                if sz.width > 0 && sz.height > 0 {
                    self.width = sz.width;
                    self.height = sz.height;
                    self.resize_all_tabs();
                    self.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_x = position.x as f32;
                self.cursor_y = position.y as f32;
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
                    let vh = content_viewport_height(self.height as f32, &self.chrome) / z;
                    if vy < vh && self.active_page_mut().on_mouse_move(cx, vy) {
                        self.request_redraw();
                    }
                }
                self.update_cursor_icon();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (-x * 40.0, -y * 48.0),
                    MouseScrollDelta::PixelDelta(p) => (-p.x as f32, -p.y as f32),
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
                button: MouseButton::Left,
                ..
            } => {
                let hit = hit_test_chrome(
                    self.cursor_x,
                    self.cursor_y,
                    self.width as f32,
                    self.height as f32,
                    self.tabs.len(),
                    &self.chrome,
                );
                self.chrome.omnibox_focused = false;
                self.chrome.console_focused = false;
                match hit {
                    ChromeHit::Tab(i) => {
                        self.active_tab = i;
                        self.sync_omnibox();
                        self.sync_window_title();
                    }
                    ChromeHit::CloseTab(i) => self.close_tab(i),
                    ChromeHit::NewTab => self.open_new_tab("peedle://newtab"),
                    ChromeHit::Back => {
                        let _ = self.active_page_mut().go_back();
                        self.sync_omnibox();
                        self.sync_window_title();
                    }
                    ChromeHit::Forward => {
                        let _ = self.active_page_mut().go_forward();
                        self.sync_omnibox();
                        self.sync_window_title();
                    }
                    ChromeHit::Reload => {
                        let _ = self.active_page_mut().reload();
                        self.sync_window_title();
                    }
                    ChromeHit::Omnibox => {
                        self.chrome.omnibox_focused = true;
                    }
                    ChromeHit::ToggleBookmark => {
                        self.toggle_current_bookmark();
                    }
                    ChromeHit::BookmarkItem(i) => {
                        if let Some(bm) = self.chrome.bookmarks.get(i).cloned() {
                            let _ = self.active_page_mut().navigate(&bm.url);
                            self.sync_omnibox();
                            self.sync_window_title();
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
                        if self.active_page_mut().on_click(cx, vy).is_some() {
                            self.sync_omnibox();
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
                let ctrl = self.modifiers.control_key() || self.modifiers.super_key();
                let alt = self.modifiers.alt_key();
                if alt {
                    match event.logical_key {
                        Key::Named(NamedKey::ArrowLeft) => {
                            let _ = self.active_page_mut().go_back();
                            self.sync_omnibox();
                            self.sync_window_title();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowRight) => {
                            let _ = self.active_page_mut().go_forward();
                            self.sync_omnibox();
                            self.sync_window_title();
                            self.request_redraw();
                            return;
                        }
                        _ => {}
                    }
                }
                if ctrl {
                    if matches!(event.logical_key, Key::Named(NamedKey::Tab)) && self.tabs.len() > 1 {
                        if self.modifiers.shift_key() {
                            self.active_tab = if self.active_tab == 0 {
                                self.tabs.len() - 1
                            } else {
                                self.active_tab - 1
                            };
                        } else {
                            self.active_tab = (self.active_tab + 1) % self.tabs.len();
                        }
                        self.sync_omnibox();
                        self.sync_window_title();
                        self.request_redraw();
                        return;
                    }
                    if let Key::Character(ref ch) = event.logical_key {
                        match ch.to_ascii_lowercase().as_str() {
                            "t" => {
                                self.open_new_tab("peedle://newtab");
                                self.chrome.omnibox_focused = true;
                                self.request_redraw();
                                return;
                            }
                            "w" => {
                                let idx = self.active_tab;
                                self.close_tab(idx);
                                self.request_redraw();
                                return;
                            }
                            "l" => {
                                self.chrome.omnibox_focused = true;
                                self.chrome.find_open = false;
                                self.chrome.console_focused = false;
                                self.chrome.omnibox_text.clear();
                                self.request_redraw();
                                return;
                            }
                            "r" => {
                                let _ = self.active_page_mut().reload();
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
                            "f" => {
                                self.chrome.find_open = !self.chrome.find_open;
                                self.chrome.omnibox_focused = false;
                                self.chrome.console_focused = false;
                                self.request_redraw();
                                return;
                            }
                            "h" => {
                                let _ = self.active_page_mut().navigate("peedle://history");
                                self.sync_omnibox();
                                self.sync_window_title();
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
                                if let Ok(n) = ch.parse::<usize>() {
                                    let idx = n.saturating_sub(1);
                                    if idx < self.tabs.len() {
                                        self.active_tab = idx;
                                        self.sync_omnibox();
                                        self.sync_window_title();
                                        self.request_redraw();
                                        return;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                if matches!(event.logical_key, Key::Named(NamedKey::F12)) {
                    self.chrome.devtools_open = !self.chrome.devtools_open;
                    self.resize_all_tabs();
                    self.request_redraw();
                    return;
                }
                if matches!(event.logical_key, Key::Named(NamedKey::F5)) {
                    let _ = self.active_page_mut().reload();
                    self.request_redraw();
                    return;
                }

                if self.chrome.find_open {
                    match &event.logical_key {
                        Key::Named(NamedKey::Escape) => {
                            self.chrome.find_open = false;
                        }
                        Key::Named(NamedKey::Enter) => {
                            let step = if self.modifiers.shift_key() { -1 } else { 1 };
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
                            let _ = self.active_page_mut().navigate(&target);
                            self.sync_omnibox();
                            self.sync_window_title();
                        }
                        Key::Named(NamedKey::Escape) => {
                            self.chrome.omnibox_focused = false;
                            self.sync_omnibox();
                        }
                        Key::Named(NamedKey::Backspace) => {
                            self.chrome.omnibox_text.pop();
                        }
                        _ => {
                            if let Some(txt) = &event.text {
                                for c in txt.chars() {
                                    if !c.is_control() {
                                        self.chrome.omnibox_text.push(c);
                                    }
                                }
                            }
                        }
                    }
                    self.request_redraw();
                    return;
                }

                match &event.logical_key {
                    Key::Named(NamedKey::PageDown) => self.active_page_mut().scroll_by(0.0, 420.0),
                    Key::Named(NamedKey::PageUp) => self.active_page_mut().scroll_by(0.0, -420.0),
                    Key::Named(NamedKey::ArrowDown) => self.active_page_mut().scroll_by(0.0, 48.0),
                    Key::Named(NamedKey::ArrowUp) => self.active_page_mut().scroll_by(0.0, -48.0),
                    Key::Named(NamedKey::Backspace) => {
                        self.active_page_mut().on_text_input('\u{0008}');
                    }
                    Key::Named(NamedKey::Enter) => {
                        self.active_page_mut().on_text_input('\n');
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
