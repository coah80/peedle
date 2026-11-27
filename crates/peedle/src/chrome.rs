use peedle_css::get_computed_property;
use peedle_engine::Page;
use peedle_render::{FontFaceKey, FontSystem, Framebuffer, RgbaColor};

pub const TAB_BAR_H: f32 = 34.0;
pub const TOOLBAR_H: f32 = 42.0;
pub const BOOKMARK_BAR_H: f32 = 28.0;
pub const DEVTOOLS_H: f32 = 220.0;

#[derive(Clone, Debug)]
pub struct Bookmark {
    pub title: String,
    pub url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevToolsTab {
    Elements,
    Console,
    Network,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromeHit {
    Tab(usize),
    CloseTab(usize),
    NewTab,
    Back,
    Forward,
    Reload,
    Omnibox,
    ToggleBookmark,
    BookmarkItem(usize),
    ToggleDevTools,
    DevToolsTab(DevToolsTab),
    ConsoleInput,
    FindInput,
    Scrollbar,
    Content,
    None,
}

pub struct ChromeState {
    pub omnibox_text: String,
    pub omnibox_focused: bool,
    pub omnibox_selected: bool,
    pub devtools_open: bool,
    pub devtools_tab: DevToolsTab,
    pub console_input: String,
    pub console_focused: bool,
    pub bookmarks_visible: bool,
    pub bookmarks: Vec<Bookmark>,
    pub find_open: bool,
    pub find_query: String,
    pub find_match_count: usize,
    pub find_match_index: usize,
    pub zoom: f32,
}

impl Default for ChromeState {
    fn default() -> Self {
        Self {
            omnibox_text: "peedle://newtab".to_string(),
            omnibox_focused: false,
            omnibox_selected: false,
            devtools_open: false,
            devtools_tab: DevToolsTab::Elements,
            console_input: String::new(),
            console_focused: false,
            bookmarks_visible: true,
            bookmarks: vec![
                Bookmark {
                    title: "Example".to_string(),
                    url: "https://example.com".to_string(),
                },
                Bookmark {
                    title: "Hacker News".to_string(),
                    url: "https://news.ycombinator.com".to_string(),
                },
                Bookmark {
                    title: "Wikipedia".to_string(),
                    url: "https://en.wikipedia.org/wiki/Rust_(programming_language)".to_string(),
                },
                Bookmark {
                    title: "GitHub".to_string(),
                    url: "https://github.com/coah80/peedle".to_string(),
                },
                Bookmark {
                    title: "Demo".to_string(),
                    url: "peedle://demo".to_string(),
                },
                Bookmark {
                    title: "History".to_string(),
                    url: "peedle://history".to_string(),
                },
                Bookmark {
                    title: "About Peedle".to_string(),
                    url: "peedle://about".to_string(),
                },
            ],
            find_open: false,
            find_query: String::new(),
            find_match_count: 0,
            find_match_index: 0,
            zoom: 1.0,
        }
    }
}

pub fn top_chrome_height(chrome: &ChromeState) -> f32 {
    TAB_BAR_H + TOOLBAR_H + if chrome.bookmarks_visible { BOOKMARK_BAR_H } else { 0.0 }
}

pub fn content_viewport_height(window_h: f32, chrome: &ChromeState) -> f32 {
    let dt = if chrome.devtools_open { DEVTOOLS_H } else { 0.0 };
    (window_h - top_chrome_height(chrome) - dt).max(100.0)
}

pub fn hit_test_chrome(
    x: f32,
    y: f32,
    win_w: f32,
    win_h: f32,
    tab_count: usize,
    chrome: &ChromeState,
) -> ChromeHit {
    if y < TAB_BAR_H {
        let tab_w = ((win_w - 60.0) / tab_count.max(1) as f32).clamp(90.0, 200.0);
        let start_x = 10.0;
        for i in 0..tab_count {
            let tx = start_x + i as f32 * (tab_w + 4.0);
            if x >= tx && x < tx + tab_w {
                if x >= tx + tab_w - 22.0 {
                    return ChromeHit::CloseTab(i);
                }
                return ChromeHit::Tab(i);
            }
        }
        let plus_x = start_x + tab_count as f32 * (tab_w + 4.0) + 2.0;
        if x >= plus_x && x < plus_x + 28.0 {
            return ChromeHit::NewTab;
        }
        return ChromeHit::None;
    }

    let toolbar_end = TAB_BAR_H + TOOLBAR_H;
    if y < toolbar_end {
        if x >= 8.0 && x < 36.0 {
            return ChromeHit::Back;
        }
        if x >= 40.0 && x < 68.0 {
            return ChromeHit::Forward;
        }
        if x >= 72.0 && x < 100.0 {
            return ChromeHit::Reload;
        }
        let dt_x = win_w - 46.0;
        if x >= dt_x && x < win_w - 8.0 {
            return ChromeHit::ToggleDevTools;
        }
        let star_x = dt_x - 38.0;
        if x >= star_x && x < dt_x - 8.0 {
            return ChromeHit::ToggleBookmark;
        }
        if x >= 108.0 && x < star_x {
            return ChromeHit::Omnibox;
        }
        return ChromeHit::None;
    }

    let top_h = top_chrome_height(chrome);
    if chrome.bookmarks_visible && y < top_h {
        let mut bx = 12.0;
        for (i, bm) in chrome.bookmarks.iter().enumerate() {
            let bw = (bm.title.chars().count() as f32 * 7.2 + 20.0).clamp(52.0, 150.0);
            if x >= bx && x < bx + bw {
                return ChromeHit::BookmarkItem(i);
            }
            bx += bw + 6.0;
        }
        return ChromeHit::None;
    }

    if chrome.find_open {
        let fw = 280.0;
        let fx = win_w - fw - 16.0;
        let fy = top_h + 8.0;
        if x >= fx && x <= fx + fw && y >= fy && y <= fy + 32.0 {
            return ChromeHit::FindInput;
        }
    }

    if chrome.devtools_open {
        let dt_top = win_h - DEVTOOLS_H;
        if y >= dt_top {
            if y < dt_top + 28.0 {
                if x < 100.0 {
                    return ChromeHit::DevToolsTab(DevToolsTab::Elements);
                }
                if x < 190.0 {
                    return ChromeHit::DevToolsTab(DevToolsTab::Console);
                }
                if x < 280.0 {
                    return ChromeHit::DevToolsTab(DevToolsTab::Network);
                }
            }
            if y >= win_h - 32.0 {
                return ChromeHit::ConsoleInput;
            }
            return ChromeHit::None;
        }
    }

    if x >= win_w - 12.0 && y >= top_h && y < top_h + content_viewport_height(win_h, chrome) {
        return ChromeHit::Scrollbar;
    }

    ChromeHit::Content
}

struct ScaledDraw<'a> {
    fb: &'a mut Framebuffer,
    fonts: &'a FontSystem,
    s: f32,
}

impl<'a> ScaledDraw<'a> {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: RgbaColor) {
        let s = self.s;
        self.fb.fill_rect(x * s, y * s, w * s, h * s, color);
    }

    fn rrect(&mut self, x: f32, y: f32, w: f32, h: f32, radii: [f32; 4], color: RgbaColor) {
        let s = self.s;
        self.fb.fill_rounded_rect(
            x * s,
            y * s,
            w * s,
            h * s,
            [radii[0] * s, radii[1] * s, radii[2] * s, radii[3] * s],
            color,
        );
    }

    fn stroke_rrect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radii: [f32; 4],
        widths: [f32; 4],
        colors: [RgbaColor; 4],
    ) {
        let s = self.s;
        self.fb.stroke_rounded_rect(
            x * s,
            y * s,
            w * s,
            h * s,
            [radii[0] * s, radii[1] * s, radii[2] * s, radii[3] * s],
            [widths[0] * s, widths[1] * s, widths[2] * s, widths[3] * s],
            colors,
        );
    }

    fn text(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        face: FontFaceKey,
        px: f32,
        color: RgbaColor,
    ) -> f32 {
        let s = self.s;
        self.fonts
            .draw_text(self.fb, text, x * s, y * s, face, px * s, color)
            / s
    }

    fn measure(&self, text: &str, face: FontFaceKey, px: f32) -> f32 {
        self.fonts.measure_text(text, face, px * self.s) / self.s
    }
}

pub fn draw_chrome(
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    tabs: &[Page],
    active_tab: usize,
    chrome: &ChromeState,
    scale: f32,
) {
    let s = scale.max(1.0);
    let w = fb.width as f32 / s;
    let h = fb.height as f32 / s;
    let mut d = ScaledDraw { fb, fonts, s };

    let bg_tabs = RgbaColor::new(15, 19, 26, 255);
    let bg_bar = RgbaColor::new(23, 28, 38, 255);
    let bg_tab_active = RgbaColor::new(28, 35, 48, 255);
    let bg_tab_inactive = RgbaColor::new(19, 24, 33, 255);
    let border_c = RgbaColor::new(40, 50, 68, 255);
    let accent = RgbaColor::new(56, 189, 248, 255);
    let gold = RgbaColor::new(250, 204, 21, 255);
    let text_hi = RgbaColor::new(235, 241, 248, 255);
    let text_dim = RgbaColor::new(148, 163, 184, 255);

    d.rect(0.0, 0.0, w, TAB_BAR_H, bg_tabs);

    let tab_w = ((w - 60.0) / tabs.len().max(1) as f32).clamp(90.0, 200.0);
    let start_x = 10.0;
    for (i, page) in tabs.iter().enumerate() {
        let tx = start_x + i as f32 * (tab_w + 4.0);
        let is_active = i == active_tab;
        let fill = if is_active { bg_tab_active } else { bg_tab_inactive };
        d.rrect(tx, 5.0, tab_w, TAB_BAR_H - 5.0, [7.0, 7.0, 0.0, 0.0], fill);
        if is_active {
            d.rect(tx + 8.0, 5.0, tab_w - 16.0, 2.0, accent);
        }
        let title = truncate_chars(&page.title(), ((tab_w - 34.0) / 7.5).max(4.0) as usize);
        d.text(
            &title,
            tx + 10.0,
            22.0,
            if is_active {
                FontFaceKey::SansBold
            } else {
                FontFaceKey::SansRegular
            },
            12.0,
            if is_active { text_hi } else { text_dim },
        );
        d.text(
            "x",
            tx + tab_w - 16.0,
            21.0,
            FontFaceKey::SansRegular,
            11.0,
            text_dim,
        );
    }

    let plus_x = start_x + tabs.len() as f32 * (tab_w + 4.0) + 2.0;
    d.rrect(plus_x, 7.0, 24.0, 22.0, [6.0; 4], bg_tab_inactive);
    d.text("+", plus_x + 7.0, 22.0, FontFaceKey::SansBold, 14.0, text_dim);

    let toolbar_end = TAB_BAR_H + TOOLBAR_H;
    d.rect(0.0, TAB_BAR_H, w, TOOLBAR_H, bg_bar);
    d.rect(0.0, toolbar_end - 1.0, w, 1.0, border_c);

    let page = &tabs[active_tab];
    draw_nav_button(&mut d, 8.0, TAB_BAR_H + 7.0, "<", page.can_go_back());
    draw_nav_button(&mut d, 40.0, TAB_BAR_H + 7.0, ">", page.can_go_forward());
    draw_nav_button(&mut d, 72.0, TAB_BAR_H + 7.0, "R", true);

    let dt_x = w - 46.0;
    let omni_x = 108.0;
    let omni_w = (dt_x - 8.0 - omni_x).max(120.0);
    let omni_y = TAB_BAR_H + 6.0;
    let omni_h = 30.0;

    d.rrect(omni_x, omni_y, omni_w, omni_h, [8.0; 4], RgbaColor::new(13, 17, 23, 255));
    let omni_border = if chrome.omnibox_focused { accent } else { border_c };
    d.stroke_rrect(omni_x, omni_y, omni_w, omni_h, [8.0; 4], [1.0; 4], [omni_border; 4]);

    let is_https = page.url.starts_with("https://");
    let badge_color = if is_https {
        RgbaColor::new(74, 222, 128, 255)
    } else {
        text_dim
    };
    let badge_label = if is_https { "TLS" } else { "WEB" };
    d.text(
        badge_label,
        omni_x + 10.0,
        omni_y + 19.0,
        FontFaceKey::MonoBold,
        10.0,
        badge_color,
    );

    let text_x = omni_x + 42.0;
    let shown = truncate_chars(
        &chrome.omnibox_text,
        ((omni_w - 86.0) / 7.5).max(10.0) as usize,
    );
    if chrome.omnibox_focused && chrome.omnibox_selected && !shown.is_empty() {
        let sel_w = d
            .measure(&shown, FontFaceKey::SansRegular, 13.0)
            .min(omni_w - 80.0);
        d.rrect(
            text_x - 2.0,
            omni_y + 5.0,
            sel_w + 4.0,
            20.0,
            [3.0; 4],
            RgbaColor::new(37, 99, 235, 165),
        );
    }
    let end_x = d.text(
        &shown,
        text_x,
        omni_y + 19.5,
        FontFaceKey::SansRegular,
        13.0,
        text_hi,
    );
    if chrome.omnibox_focused && !chrome.omnibox_selected {
        d.rect(end_x + 1.0, omni_y + 6.0, 1.5, 18.0, accent);
    }

    if (chrome.zoom - 1.0).abs() > 0.01 {
        let ztxt = format!("{}%", (chrome.zoom * 100.0).round() as i32);
        d.rrect(
            dt_x - 84.0,
            omni_y + 5.0,
            44.0,
            20.0,
            [5.0; 4],
            RgbaColor::new(30, 58, 95, 255),
        );
        d.text(
            &ztxt,
            dt_x - 77.0,
            omni_y + 18.5,
            FontFaceKey::MonoBold,
            10.5,
            accent,
        );
    }

    let is_bookmarked = chrome.bookmarks.iter().any(|b| b.url == page.url);
    d.text(
        if is_bookmarked { "*" } else { "+" },
        dt_x - 28.0,
        omni_y + 20.5,
        FontFaceKey::MonoBold,
        14.0,
        if is_bookmarked { gold } else { text_dim },
    );

    let dt_bg = if chrome.devtools_open {
        RgbaColor::new(30, 58, 95, 255)
    } else {
        RgbaColor::new(31, 39, 53, 255)
    };
    d.rrect(dt_x, TAB_BAR_H + 7.0, 38.0, 28.0, [6.0; 4], dt_bg);
    d.text(
        "DT",
        dt_x + 10.0,
        TAB_BAR_H + 25.0,
        FontFaceKey::MonoBold,
        11.0,
        if chrome.devtools_open { accent } else { text_dim },
    );

    let top_h = top_chrome_height(chrome);
    if chrome.bookmarks_visible {
        d.rect(0.0, toolbar_end, w, BOOKMARK_BAR_H, RgbaColor::new(19, 24, 33, 255));
        d.rect(0.0, top_h - 1.0, w, 1.0, border_c);
        let mut bx = 12.0;
        for bm in &chrome.bookmarks {
            let label = truncate_chars(&bm.title, 18);
            let bw = (label.chars().count() as f32 * 7.2 + 20.0).clamp(52.0, 150.0);
            if bx + bw > w - 12.0 {
                break;
            }
            d.rrect(bx, toolbar_end + 4.0, bw, 20.0, [5.0; 4], RgbaColor::new(28, 35, 48, 255));
            d.text(
                &label,
                bx + 10.0,
                toolbar_end + 17.5,
                FontFaceKey::SansRegular,
                11.5,
                text_dim,
            );
            bx += bw + 6.0;
        }
    }

    let vh = content_viewport_height(h, chrome);
    let content_h = page.js_env.dom.borrow().layout.content_height;
    if content_h > vh + 1.0 {
        let track_h = vh - 8.0;
        let thumb_h = ((vh / content_h) * track_h).clamp(24.0, track_h);
        let max_scroll = (content_h - vh).max(1.0);
        let thumb_y = top_h + 4.0 + (page.scroll_y / max_scroll) * (track_h - thumb_h);
        d.rrect(
            w - 9.0,
            thumb_y,
            6.0,
            thumb_h,
            [3.0; 4],
            RgbaColor::new(100, 116, 139, 175),
        );
    }

    if chrome.find_open {
        let fw = 280.0;
        let fx = w - fw - 16.0;
        let fy = top_h + 8.0;
        d.rrect(fx, fy, fw, 32.0, [8.0; 4], RgbaColor::new(19, 24, 33, 245));
        d.stroke_rrect(fx, fy, fw, 32.0, [8.0; 4], [1.0; 4], [accent; 4]);
        let q = truncate_chars(&chrome.find_query, 24);
        let end = d.text(&q, fx + 10.0, fy + 20.0, FontFaceKey::SansRegular, 12.5, text_hi);
        d.rect(end + 1.0, fy + 8.0, 1.5, 16.0, accent);
        let counter = if chrome.find_query.is_empty() {
            String::new()
        } else if chrome.find_match_count == 0 {
            "0/0".to_string()
        } else {
            format!("{}/{}", chrome.find_match_index + 1, chrome.find_match_count)
        };
        d.text(
            &counter,
            fx + fw - 48.0,
            fy + 20.0,
            FontFaceKey::MonoRegular,
            11.0,
            text_dim,
        );
    }

    if let Some(ref href) = page.hovered_href {
        let dt_off = if chrome.devtools_open { DEVTOOLS_H } else { 0.0 };
        let pill_w = (href.len() as f32 * 7.2 + 20.0).min(w * 0.7);
        let pill_y = h - dt_off - 24.0;
        d.rrect(6.0, pill_y, pill_w, 20.0, [4.0; 4], RgbaColor::new(15, 19, 26, 235));
        d.text(
            &truncate_chars(href, 80),
            12.0,
            pill_y + 14.0,
            FontFaceKey::MonoRegular,
            11.0,
            text_dim,
        );
    }

    if chrome.devtools_open {
        draw_devtools(&mut d, page, chrome, h - DEVTOOLS_H, w, DEVTOOLS_H);
    }
}

fn draw_nav_button(d: &mut ScaledDraw<'_>, x: f32, y: f32, label: &str, enabled: bool) {
    d.rrect(x, y, 28.0, 28.0, [6.0; 4], RgbaColor::new(31, 39, 53, 255));
    let c = if enabled {
        RgbaColor::new(226, 232, 240, 255)
    } else {
        RgbaColor::new(71, 85, 105, 255)
    };
    d.text(label, x + 9.5, y + 18.5, FontFaceKey::MonoBold, 13.0, c);
}

fn draw_devtools(
    d: &mut ScaledDraw<'_>,
    page: &Page,
    chrome: &ChromeState,
    top: f32,
    w: f32,
    h: f32,
) {
    let tab = chrome.devtools_tab;
    let bg = RgbaColor::new(13, 17, 23, 255);
    let header_bg = RgbaColor::new(22, 27, 36, 255);
    let border = RgbaColor::new(40, 50, 68, 255);
    let accent = RgbaColor::new(56, 189, 248, 255);
    let text_hi = RgbaColor::new(226, 232, 240, 255);
    let text_dim = RgbaColor::new(148, 163, 184, 255);

    d.rect(0.0, top, w, h, bg);
    d.rect(0.0, top, w, 28.0, header_bg);
    d.rect(0.0, top, w, 1.0, border);
    d.rect(0.0, top + 28.0, w, 1.0, border);

    let tabs = [
        (DevToolsTab::Elements, "Elements", 12.0),
        (DevToolsTab::Console, "Console", 108.0),
        (DevToolsTab::Network, "Network", 198.0),
    ];
    for (kind, label, x) in tabs {
        let active = kind == tab;
        d.text(
            label,
            x,
            top + 18.0,
            FontFaceKey::SansBold,
            12.0,
            if active { accent } else { text_dim },
        );
        if active {
            d.rect(x, top + 26.0, 64.0, 2.0, accent);
        }
    }

    let mut y = top + 46.0;
    let max_y = top + h - 36.0;
    match tab {
        DevToolsTab::Elements => {
            let split_x = (w * 0.58).max(260.0);
            let mut lines = Vec::new();
            let st = page.js_env.dom.borrow();
            if let Some(root_el) = st.doc.document_element() {
                collect_dom_lines(&st.doc, root_el, 0, &mut lines, 10);
            }
            for line in lines {
                if y > max_y + 20.0 {
                    break;
                }
                d.text(&truncate_chars(&line, 66), 12.0, y, FontFaceKey::MonoRegular, 11.5, text_hi);
                y += 16.0;
            }
            d.rect(split_x, top + 28.0, 1.0, h - 28.0, border);
            let inspect_node = page.hovered_node.or_else(|| st.doc.body());
            let mut sy = top + 46.0;
            if let Some(nid) = inspect_node {
                if let Some(el) = st.doc.element(nid) {
                    let hdr = format!("<{}> computed", el.tag_name);
                    d.text(&hdr, split_x + 12.0, sy, FontFaceKey::MonoBold, 11.5, accent);
                    sy += 18.0;
                }
                if let Some(r) = st.layout.bounding_rect(nid) {
                    let box_line = format!("box: {:.0}x{:.0} @ ({:.0}, {:.0})", r.width, r.height, r.x, r.y);
                    d.text(&box_line, split_x + 12.0, sy, FontFaceKey::MonoRegular, 11.0, text_dim);
                    sy += 16.0;
                }
                if let Some(cs) = st.styles.get(nid) {
                    for prop in ["display", "position", "color", "background-color", "font-size", "margin", "padding"] {
                        if sy > top + h - 10.0 {
                            break;
                        }
                        let val = get_computed_property(cs, prop);
                        let row = format!("{}: {}", prop, truncate_chars(&val, 34));
                        d.text(&row, split_x + 12.0, sy, FontFaceKey::MonoRegular, 11.0, text_hi);
                        sy += 15.0;
                    }
                }
            }
        }
        DevToolsTab::Console => {
            if page.console_logs.is_empty() {
                d.text("(no console output)", 12.0, y, FontFaceKey::MonoRegular, 11.5, text_dim);
            } else {
                for line in page.console_logs.iter().rev().take(9).rev() {
                    if y > max_y {
                        break;
                    }
                    let c = if line.starts_with("[Error]") {
                        RgbaColor::new(248, 113, 113, 255)
                    } else if line.starts_with("=> ") {
                        RgbaColor::new(125, 211, 252, 255)
                    } else {
                        text_hi
                    };
                    d.text(&truncate_chars(line, 110), 12.0, y, FontFaceKey::MonoRegular, 11.5, c);
                    y += 16.0;
                }
            }
            let input_y = top + h - 30.0;
            d.rect(0.0, input_y - 2.0, w, 1.0, border);
            d.rect(0.0, input_y - 1.0, w, 31.0, header_bg);
            d.text(">", 12.0, input_y + 18.0, FontFaceKey::MonoBold, 12.0, accent);
            let shown_in = truncate_chars(&chrome.console_input, 110);
            let end_x = d.text(&shown_in, 26.0, input_y + 18.0, FontFaceKey::MonoRegular, 12.0, text_hi);
            if chrome.console_focused {
                d.rect(end_x + 1.0, input_y + 6.0, 1.5, 16.0, accent);
            }
        }
        DevToolsTab::Network => {
            for entry in page.network_log.iter().take(10) {
                if y > max_y + 20.0 {
                    break;
                }
                let row = format!(
                    "{} {} {:>6}B {:>4}ms  {}",
                    entry.status, entry.method, entry.bytes, entry.duration_ms, entry.url
                );
                d.text(&truncate_chars(&row, 110), 12.0, y, FontFaceKey::MonoRegular, 11.5, text_hi);
                y += 16.0;
            }
        }
    }
}

fn collect_dom_lines(
    doc: &peedle_html::Document,
    id: peedle_html::NodeId,
    depth: usize,
    out: &mut Vec<String>,
    limit: usize,
) {
    if out.len() >= limit {
        return;
    }
    let indent = "  ".repeat(depth.min(8));
    if let Some(el) = doc.element(id) {
        let mut s = format!("{}<{}", indent, el.tag_name);
        if let Some(id_attr) = el.id() {
            s.push_str(&format!(" id=\"{}\"", id_attr));
        }
        if let Some(cls) = el.get_attr("class") {
            s.push_str(&format!(" class=\"{}\"", truncate_chars(cls, 24)));
        }
        s.push('>');
        out.push(s);
        for c in doc.element_children(id) {
            collect_dom_lines(doc, c, depth + 1, out, limit);
        }
    }
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else if max_chars <= 3 {
        s.chars().take(max_chars).collect()
    } else {
        let mut out: String = s.chars().take(max_chars - 3).collect();
        out.push_str("...");
        out
    }
}
