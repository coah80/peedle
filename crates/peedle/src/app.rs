use std::num::NonZeroU32;
use std::rc::Rc;

use peedle_engine::Page;
use peedle_render::{ClipRect, FontSystem, Framebuffer};
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::chrome::{
    content_viewport_height, draw_chrome, hit_test_chrome, ChromeHit, ChromeState, CHROME_H,
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

    fn open_new_tab(&mut self, url: &str) {
        let vh = content_viewport_height(self.height as f32, self.chrome.devtools_open);
        let mut page = Page::new(self.width as f32, vh);
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
        let w = self.width as f32;
        let vh = content_viewport_height(self.height as f32, self.chrome.devtools_open);
        for tab in &mut self.tabs {
            tab.resize(w, vh);
        }
    }

    fn render(&mut self) {
        if self.width == 0 || self.height == 0 || self.tabs.is_empty() {
            return;
        }
        let vh = content_viewport_height(self.height as f32, self.chrome.devtools_open);
        let mut page_fb = Framebuffer::new(self.width, vh.ceil() as u32);
        self.tabs[self.active_tab].render_to_framebuffer(&mut page_fb);

        let mut full_fb = Framebuffer::new(self.width, self.height);
        full_fb.push_clip(ClipRect {
            x: 0,
            y: CHROME_H as i32,
            w: self.width as i32,
            h: vh.ceil() as i32,
        });
        full_fb.blit_rgba(
            page_fb.width,
            page_fb.height,
            &page_fb.data,
            0.0,
            CHROME_H,
            page_fb.width as f32,
            page_fb.height as f32,
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
                let cx = self.cursor_x;
                if self.cursor_y >= CHROME_H {
                    let vy = self.cursor_y - CHROME_H;
                    let vh = content_viewport_height(self.height as f32, self.chrome.devtools_open);
                    if vy < vh && self.active_page_mut().on_mouse_move(cx, vy) {
                        self.request_redraw();
                    }
                }
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
                    self.chrome.devtools_open,
                );
                self.chrome.omnibox_focused = false;
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
                    ChromeHit::ToggleDevTools => {
                        self.chrome.devtools_open = !self.chrome.devtools_open;
                        self.resize_all_tabs();
                    }
                    ChromeHit::DevToolsTab(t) => {
                        self.chrome.devtools_tab = t;
                    }
                    ChromeHit::Content => {
                        let cx = self.cursor_x;
                        let vy = self.cursor_y - CHROME_H;
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
                if ctrl {
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
                                self.chrome.omnibox_text.clear();
                                self.request_redraw();
                                return;
                            }
                            "r" => {
                                let _ = self.active_page_mut().reload();
                                self.request_redraw();
                                return;
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
