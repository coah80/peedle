use peedle_engine::Page;
use peedle_render::{FontFaceKey, FontSystem, Framebuffer, RgbaColor};

pub const TAB_BAR_H: f32 = 34.0;
pub const TOOLBAR_H: f32 = 42.0;
pub const CHROME_H: f32 = TAB_BAR_H + TOOLBAR_H;
pub const DEVTOOLS_H: f32 = 220.0;

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
    ToggleDevTools,
    DevToolsTab(DevToolsTab),
    Content,
    None,
}

pub struct ChromeState {
    pub omnibox_text: String,
    pub omnibox_focused: bool,
    pub devtools_open: bool,
    pub devtools_tab: DevToolsTab,
}

impl Default for ChromeState {
    fn default() -> Self {
        Self {
            omnibox_text: "peedle://newtab".to_string(),
            omnibox_focused: false,
            devtools_open: false,
            devtools_tab: DevToolsTab::Elements,
        }
    }
}

pub fn content_viewport_height(window_h: f32, devtools_open: bool) -> f32 {
    let dt = if devtools_open { DEVTOOLS_H } else { 0.0 };
    (window_h - CHROME_H - dt).max(100.0)
}

pub fn hit_test_chrome(
    x: f32,
    y: f32,
    win_w: f32,
    win_h: f32,
    tab_count: usize,
    devtools_open: bool,
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

    if y < CHROME_H {
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
        if x >= 108.0 && x < dt_x - 8.0 {
            return ChromeHit::Omnibox;
        }
        return ChromeHit::None;
    }

    if devtools_open {
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
            return ChromeHit::None;
        }
    }

    ChromeHit::Content
}

pub fn draw_chrome(
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    tabs: &[Page],
    active_tab: usize,
    chrome: &ChromeState,
) {
    let w = fb.width as f32;
    let h = fb.height as f32;

    let bg_tabs = RgbaColor::new(15, 19, 26, 255);
    let bg_bar = RgbaColor::new(23, 28, 38, 255);
    let bg_tab_active = RgbaColor::new(28, 35, 48, 255);
    let bg_tab_inactive = RgbaColor::new(19, 24, 33, 255);
    let border_c = RgbaColor::new(40, 50, 68, 255);
    let accent = RgbaColor::new(56, 189, 248, 255);
    let text_hi = RgbaColor::new(235, 241, 248, 255);
    let text_dim = RgbaColor::new(148, 163, 184, 255);

    fb.fill_rect(0.0, 0.0, w, TAB_BAR_H, bg_tabs);

    let tab_w = ((w - 60.0) / tabs.len().max(1) as f32).clamp(90.0, 200.0);
    let start_x = 10.0;
    for (i, page) in tabs.iter().enumerate() {
        let tx = start_x + i as f32 * (tab_w + 4.0);
        let is_active = i == active_tab;
        let fill = if is_active { bg_tab_active } else { bg_tab_inactive };
        fb.fill_rounded_rect(tx, 5.0, tab_w, TAB_BAR_H - 5.0, [7.0, 7.0, 0.0, 0.0], fill);
        if is_active {
            fb.fill_rect(tx + 8.0, 5.0, tab_w - 16.0, 2.0, accent);
        }
        let title = truncate_chars(&page.title(), ((tab_w - 34.0) / 7.5).max(4.0) as usize);
        fonts.draw_text(
            fb,
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
        fonts.draw_text(
            fb,
            "x",
            tx + tab_w - 16.0,
            21.0,
            FontFaceKey::SansRegular,
            11.0,
            text_dim,
        );
    }

    let plus_x = start_x + tabs.len() as f32 * (tab_w + 4.0) + 2.0;
    fb.fill_rounded_rect(plus_x, 7.0, 24.0, 22.0, [6.0; 4], bg_tab_inactive);
    fonts.draw_text(fb, "+", plus_x + 7.0, 22.0, FontFaceKey::SansBold, 14.0, text_dim);

    fb.fill_rect(0.0, TAB_BAR_H, w, TOOLBAR_H, bg_bar);
    fb.fill_rect(0.0, CHROME_H - 1.0, w, 1.0, border_c);

    let page = &tabs[active_tab];
    draw_nav_button(fb, fonts, 8.0, TAB_BAR_H + 7.0, "<", page.can_go_back());
    draw_nav_button(fb, fonts, 40.0, TAB_BAR_H + 7.0, ">", page.can_go_forward());
    draw_nav_button(fb, fonts, 72.0, TAB_BAR_H + 7.0, "R", true);

    let dt_x = w - 46.0;
    let omni_x = 108.0;
    let omni_w = (dt_x - 8.0 - omni_x).max(120.0);
    let omni_y = TAB_BAR_H + 6.0;
    let omni_h = 30.0;

    fb.fill_rounded_rect(omni_x, omni_y, omni_w, omni_h, [8.0; 4], RgbaColor::new(13, 17, 23, 255));
    let omni_border = if chrome.omnibox_focused { accent } else { border_c };
    fb.stroke_rounded_rect(omni_x, omni_y, omni_w, omni_h, [8.0; 4], [1.0; 4], [omni_border; 4]);

    let is_https = page.url.starts_with("https://");
    let badge_color = if is_https {
        RgbaColor::new(74, 222, 128, 255)
    } else {
        text_dim
    };
    let badge_label = if is_https { "TLS" } else { "WEB" };
    fonts.draw_text(
        fb,
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
        ((omni_w - 56.0) / 7.5).max(10.0) as usize,
    );
    let end_x = fonts.draw_text(
        fb,
        &shown,
        text_x,
        omni_y + 19.5,
        FontFaceKey::SansRegular,
        13.0,
        text_hi,
    );
    if chrome.omnibox_focused {
        fb.fill_rect(end_x + 1.0, omni_y + 6.0, 1.5, 18.0, accent);
    }

    let dt_bg = if chrome.devtools_open {
        RgbaColor::new(30, 58, 95, 255)
    } else {
        RgbaColor::new(31, 39, 53, 255)
    };
    fb.fill_rounded_rect(dt_x, TAB_BAR_H + 7.0, 38.0, 28.0, [6.0; 4], dt_bg);
    fonts.draw_text(
        fb,
        "DT",
        dt_x + 10.0,
        TAB_BAR_H + 25.0,
        FontFaceKey::MonoBold,
        11.0,
        if chrome.devtools_open { accent } else { text_dim },
    );

    if let Some(ref href) = page.hovered_href {
        let dt_off = if chrome.devtools_open { DEVTOOLS_H } else { 0.0 };
        let pill_w = (href.len() as f32 * 7.2 + 20.0).min(w * 0.7);
        let pill_y = h - dt_off - 24.0;
        fb.fill_rounded_rect(6.0, pill_y, pill_w, 20.0, [4.0; 4], RgbaColor::new(15, 19, 26, 235));
        fonts.draw_text(
            fb,
            &truncate_chars(href, 80),
            12.0,
            pill_y + 14.0,
            FontFaceKey::MonoRegular,
            11.0,
            text_dim,
        );
    }

    if chrome.devtools_open {
        draw_devtools(fb, fonts, page, chrome.devtools_tab, h - DEVTOOLS_H, w, DEVTOOLS_H);
    }
}

fn draw_nav_button(fb: &mut Framebuffer, fonts: &FontSystem, x: f32, y: f32, label: &str, enabled: bool) {
    fb.fill_rounded_rect(x, y, 28.0, 28.0, [6.0; 4], RgbaColor::new(31, 39, 53, 255));
    let c = if enabled {
        RgbaColor::new(226, 232, 240, 255)
    } else {
        RgbaColor::new(71, 85, 105, 255)
    };
    fonts.draw_text(fb, label, x + 9.5, y + 18.5, FontFaceKey::MonoBold, 13.0, c);
}

fn draw_devtools(
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    page: &Page,
    tab: DevToolsTab,
    top: f32,
    w: f32,
    h: f32,
) {
    let bg = RgbaColor::new(13, 17, 23, 255);
    let header_bg = RgbaColor::new(22, 27, 36, 255);
    let border = RgbaColor::new(40, 50, 68, 255);
    let accent = RgbaColor::new(56, 189, 248, 255);
    let text_hi = RgbaColor::new(226, 232, 240, 255);
    let text_dim = RgbaColor::new(148, 163, 184, 255);

    fb.fill_rect(0.0, top, w, h, bg);
    fb.fill_rect(0.0, top, w, 28.0, header_bg);
    fb.fill_rect(0.0, top, w, 1.0, border);
    fb.fill_rect(0.0, top + 28.0, w, 1.0, border);

    let tabs = [
        (DevToolsTab::Elements, "Elements", 12.0),
        (DevToolsTab::Console, "Console", 108.0),
        (DevToolsTab::Network, "Network", 198.0),
    ];
    for (kind, label, x) in tabs {
        let active = kind == tab;
        fonts.draw_text(
            fb,
            label,
            x,
            top + 18.0,
            FontFaceKey::SansBold,
            12.0,
            if active { accent } else { text_dim },
        );
        if active {
            fb.fill_rect(x, top + 26.0, 64.0, 2.0, accent);
        }
    }

    let mut y = top + 46.0;
    let max_y = top + h - 8.0;
    match tab {
        DevToolsTab::Elements => {
            let mut lines = Vec::new();
            if let Some(root_el) = page.document.document_element() {
                collect_dom_lines(&page.document, root_el, 0, &mut lines, 12);
            }
            for line in lines {
                if y > max_y {
                    break;
                }
                fonts.draw_text(fb, &truncate_chars(&line, 110), 12.0, y, FontFaceKey::MonoRegular, 11.5, text_hi);
                y += 16.0;
            }
        }
        DevToolsTab::Console => {
            if page.console_logs.is_empty() {
                fonts.draw_text(fb, "(no console output)", 12.0, y, FontFaceKey::MonoRegular, 11.5, text_dim);
            } else {
                for line in page.console_logs.iter().rev().take(10).rev() {
                    if y > max_y {
                        break;
                    }
                    fonts.draw_text(fb, &truncate_chars(line, 110), 12.0, y, FontFaceKey::MonoRegular, 11.5, text_hi);
                    y += 16.0;
                }
            }
        }
        DevToolsTab::Network => {
            for entry in page.network_log.iter().take(10) {
                if y > max_y {
                    break;
                }
                let row = format!(
                    "{} {} {:>6}B {:>4}ms  {}",
                    entry.status, entry.method, entry.bytes, entry.duration_ms, entry.url
                );
                fonts.draw_text(fb, &truncate_chars(&row, 110), 12.0, y, FontFaceKey::MonoRegular, 11.5, text_hi);
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
            s.push_str(&format!(" class=\"{}\"", truncate_chars(cls, 28)));
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
