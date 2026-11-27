use peedle_css::get_computed_property;
use peedle_engine::Page;
use peedle_html::NodeId;
use peedle_render::{ClipRect, FontFaceKey, FontSystem, Framebuffer, RgbaColor};

use crate::profile::SavedBookmark;
use crate::text_edit::TextEdit;

pub const TAB_BAR_H: f32 = 38.0;
pub const TOOLBAR_H: f32 = 44.0;
pub const BOOKMARK_BAR_H: f32 = 30.0;
pub const DEVTOOLS_H: f32 = 230.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevToolsTab {
    Elements,
    Console,
    Network,
}

#[derive(Clone, Debug)]
pub struct OmniboxSuggestion {
    pub title: String,
    pub url: String,
    pub kind: SuggestionKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionKind {
    Bookmark,
    History,
    Search,
}

#[derive(Clone, Debug)]
pub struct ContextMenuItem {
    pub label: String,
    pub action: MenuAction,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    Back,
    Forward,
    Reload,
    NewTab,
    ReopenClosedTab,
    DuplicateTab(usize),
    ReloadTab(usize),
    CloseTab(usize),
    CloseOtherTabs(usize),
    OpenLinkNewTab(String),
    CopyText(String),
    SearchText(String),
    SavePage,
    ViewSource,
    Inspect,
    OpenUrl(String),
    ToggleBookmarksBar,
    OpenFind,
    ToggleDevTools,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleTheme,
    RemoveBookmark(usize),
}

#[derive(Clone, Debug)]
pub struct PopupMenu {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub items: Vec<ContextMenuItem>,
}

#[derive(Clone, Debug)]
pub struct SelectPopup {
    pub node_id: NodeId,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub options: Vec<String>,
    pub selected: usize,
}

#[derive(Clone, Debug)]
pub struct DownloadToast {
    pub filename: String,
    pub path: String,
    pub bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromeHit {
    Tab(usize),
    CloseTab(usize),
    NewTab,
    TabBarEmpty,
    Back,
    Forward,
    Reload,
    Omnibox,
    ToggleBookmark,
    DownloadsBtn,
    ToggleDevTools,
    MenuBtn,
    BookmarkItem(usize),
    SuggestionItem(usize),
    PopupMenuItem(usize),
    SelectOptionItem(usize),
    ToastReveal,
    ToastDownloads,
    ToastClose,
    DevToolsTab(DevToolsTab),
    ConsoleInput,
    FindInput,
    Scrollbar,
    Content,
    None,
}

pub struct TabVisual<'a> {
    pub page: &'a Page,
    pub loading: bool,
    pub load_progress: f32,
}

pub struct ChromeState {
    pub dark_mode: bool,
    pub omnibox: TextEdit,
    pub omnibox_focused: bool,
    pub suggestions: Vec<OmniboxSuggestion>,
    pub suggestion_index: Option<usize>,
    pub devtools_open: bool,
    pub devtools_tab: DevToolsTab,
    pub console: TextEdit,
    pub console_focused: bool,
    pub bookmarks_visible: bool,
    pub bookmarks: Vec<SavedBookmark>,
    pub find_open: bool,
    pub find: TextEdit,
    pub find_match_count: usize,
    pub find_match_index: usize,
    pub zoom: f32,
    pub hovered_hit: ChromeHit,
    pub popup_menu: Option<PopupMenu>,
    pub select_popup: Option<SelectPopup>,
    pub download_toast: Option<DownloadToast>,
    pub has_downloads: bool,
    pub anim_phase: f32,
}

impl Default for ChromeState {
    fn default() -> Self {
        Self {
            dark_mode: true,
            omnibox: TextEdit::new("peedle://newtab"),
            omnibox_focused: false,
            suggestions: Vec::new(),
            suggestion_index: None,
            devtools_open: false,
            devtools_tab: DevToolsTab::Elements,
            console: TextEdit::default(),
            console_focused: false,
            bookmarks_visible: true,
            bookmarks: Vec::new(),
            find_open: false,
            find: TextEdit::default(),
            find_match_count: 0,
            find_match_index: 0,
            zoom: 1.0,
            hovered_hit: ChromeHit::None,
            popup_menu: None,
            select_popup: None,
            download_toast: None,
            has_downloads: false,
            anim_phase: 0.0,
        }
    }
}

struct Palette {
    tab_strip: RgbaColor,
    toolbar: RgbaColor,
    tab_active: RgbaColor,
    tab_hover: RgbaColor,
    omni_bg: RgbaColor,
    omni_hover: RgbaColor,
    border: RgbaColor,
    accent: RgbaColor,
    accent_sel: RgbaColor,
    gold: RgbaColor,
    green: RgbaColor,
    text_hi: RgbaColor,
    text_dim: RgbaColor,
    text_muted: RgbaColor,
    btn_hover: RgbaColor,
    popup_bg: RgbaColor,
    popup_hover: RgbaColor,
    bookmark_bar: RgbaColor,
}

impl Palette {
    fn for_dark(dark: bool) -> Self {
        if dark {
            Self {
                tab_strip: RgbaColor::new(17, 20, 27, 255),
                toolbar: RgbaColor::new(30, 34, 44, 255),
                tab_active: RgbaColor::new(30, 34, 44, 255),
                tab_hover: RgbaColor::new(26, 31, 41, 255),
                omni_bg: RgbaColor::new(20, 23, 31, 255),
                omni_hover: RgbaColor::new(25, 29, 38, 255),
                border: RgbaColor::new(48, 55, 70, 255),
                accent: RgbaColor::new(56, 189, 248, 255),
                accent_sel: RgbaColor::new(37, 99, 235, 165),
                gold: RgbaColor::new(250, 204, 21, 255),
                green: RgbaColor::new(74, 222, 128, 255),
                text_hi: RgbaColor::new(236, 241, 248, 255),
                text_dim: RgbaColor::new(156, 168, 186, 255),
                text_muted: RgbaColor::new(98, 110, 128, 255),
                btn_hover: RgbaColor::new(45, 52, 66, 255),
                popup_bg: RgbaColor::new(24, 28, 37, 252),
                popup_hover: RgbaColor::new(42, 50, 66, 255),
                bookmark_bar: RgbaColor::new(25, 29, 38, 255),
            }
        } else {
            Self {
                tab_strip: RgbaColor::new(222, 226, 232, 255),
                toolbar: RgbaColor::new(248, 250, 252, 255),
                tab_active: RgbaColor::new(248, 250, 252, 255),
                tab_hover: RgbaColor::new(234, 238, 244, 255),
                omni_bg: RgbaColor::new(236, 240, 245, 255),
                omni_hover: RgbaColor::new(228, 234, 241, 255),
                border: RgbaColor::new(203, 213, 225, 255),
                accent: RgbaColor::new(2, 132, 199, 255),
                accent_sel: RgbaColor::new(59, 130, 246, 120),
                gold: RgbaColor::new(217, 119, 6, 255),
                green: RgbaColor::new(22, 163, 74, 255),
                text_hi: RgbaColor::new(15, 23, 42, 255),
                text_dim: RgbaColor::new(71, 85, 105, 255),
                text_muted: RgbaColor::new(148, 163, 184, 255),
                btn_hover: RgbaColor::new(226, 232, 240, 255),
                popup_bg: RgbaColor::new(255, 255, 255, 252),
                popup_hover: RgbaColor::new(241, 245, 249, 255),
                bookmark_bar: RgbaColor::new(241, 245, 249, 255),
            }
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

pub fn omnibox_rect(win_w: f32) -> (f32, f32, f32, f32) {
    let right_controls = 112.0;
    let omni_x = 114.0;
    let omni_w = (win_w - right_controls - omni_x).max(140.0);
    (omni_x, TAB_BAR_H + 6.0, omni_w, 32.0)
}

pub fn hit_test_chrome(
    x: f32,
    y: f32,
    win_w: f32,
    win_h: f32,
    tab_count: usize,
    chrome: &ChromeState,
) -> ChromeHit {
    if let Some(ref menu) = chrome.popup_menu {
        let item_h = 30.0;
        let mh = menu.items.len() as f32 * item_h + 10.0;
        if x >= menu.x && x <= menu.x + menu.w && y >= menu.y && y <= menu.y + mh {
            let rel = y - menu.y - 5.0;
            if rel >= 0.0 {
                let idx = (rel / item_h) as usize;
                if idx < menu.items.len() {
                    return ChromeHit::PopupMenuItem(idx);
                }
            }
            return ChromeHit::None;
        }
    }

    if let Some(ref sel) = chrome.select_popup {
        let item_h = 28.0;
        let vis = sel.options.len().min(10);
        let sh = vis as f32 * item_h + 8.0;
        if x >= sel.x && x <= sel.x + sel.w && y >= sel.y && y <= sel.y + sh {
            let rel = y - sel.y - 4.0;
            if rel >= 0.0 {
                let idx = (rel / item_h) as usize;
                if idx < vis {
                    return ChromeHit::SelectOptionItem(idx);
                }
            }
            return ChromeHit::None;
        }
    }

    if chrome.omnibox_focused && !chrome.suggestions.is_empty() {
        let (ox, oy, ow, oh) = omnibox_rect(win_w);
        let sy = oy + oh + 4.0;
        let row_h = 36.0;
        let sh = chrome.suggestions.len() as f32 * row_h + 10.0;
        if x >= ox && x <= ox + ow && y >= sy && y <= sy + sh {
            let rel = y - sy - 5.0;
            if rel >= 0.0 {
                let idx = (rel / row_h) as usize;
                if idx < chrome.suggestions.len() {
                    return ChromeHit::SuggestionItem(idx);
                }
            }
            return ChromeHit::None;
        }
    }

    if let Some(ref _toast) = chrome.download_toast {
        let dt_off = if chrome.devtools_open { DEVTOOLS_H } else { 0.0 };
        let ty = win_h - dt_off - 42.0;
        if y >= ty && y <= ty + 36.0 && x >= 12.0 && x <= 440.0 {
            if x >= 412.0 {
                return ChromeHit::ToastClose;
            }
            if x >= 332.0 {
                return ChromeHit::ToastDownloads;
            }
            return ChromeHit::ToastReveal;
        }
    }

    if y < TAB_BAR_H {
        let tab_w = ((win_w - 64.0) / tab_count.max(1) as f32).clamp(110.0, 224.0);
        let start_x = 8.0;
        for i in 0..tab_count {
            let tx = start_x + i as f32 * (tab_w + 3.0);
            if x >= tx && x < tx + tab_w {
                if x >= tx + tab_w - 24.0 {
                    return ChromeHit::CloseTab(i);
                }
                return ChromeHit::Tab(i);
            }
        }
        let plus_x = start_x + tab_count as f32 * (tab_w + 3.0) + 4.0;
        if x >= plus_x && x < plus_x + 28.0 {
            return ChromeHit::NewTab;
        }
        return ChromeHit::TabBarEmpty;
    }

    let toolbar_end = TAB_BAR_H + TOOLBAR_H;
    if y < toolbar_end {
        if x >= 8.0 && x < 38.0 {
            return ChromeHit::Back;
        }
        if x >= 42.0 && x < 72.0 {
            return ChromeHit::Forward;
        }
        if x >= 76.0 && x < 106.0 {
            return ChromeHit::Reload;
        }
        let menu_x = win_w - 38.0;
        let dt_x = menu_x - 34.0;
        let dl_x = dt_x - 34.0;
        if x >= menu_x && x < win_w - 6.0 {
            return ChromeHit::MenuBtn;
        }
        if x >= dt_x && x < menu_x - 2.0 {
            return ChromeHit::ToggleDevTools;
        }
        if x >= dl_x && x < dt_x - 2.0 {
            return ChromeHit::DownloadsBtn;
        }
        let (ox, _, ow, _) = omnibox_rect(win_w);
        let star_x = ox + ow - 32.0;
        if x >= star_x && x < ox + ow {
            return ChromeHit::ToggleBookmark;
        }
        if x >= ox && x < star_x {
            return ChromeHit::Omnibox;
        }
        return ChromeHit::None;
    }

    let top_h = top_chrome_height(chrome);
    if chrome.bookmarks_visible && y < top_h {
        let mut bx = 10.0;
        for (i, bm) in chrome.bookmarks.iter().enumerate() {
            let label = truncate_chars(&bm.title, 20);
            let bw = (label.chars().count() as f32 * 6.8 + 22.0).clamp(52.0, 160.0);
            if x >= bx && x < bx + bw {
                return ChromeHit::BookmarkItem(i);
            }
            bx += bw + 4.0;
        }
        return ChromeHit::None;
    }

    if chrome.find_open {
        let fw = 300.0;
        let fx = win_w - fw - 18.0;
        let fy = top_h + 8.0;
        if x >= fx && x <= fx + fw && y >= fy && y <= fy + 34.0 {
            return ChromeHit::FindInput;
        }
    }

    if chrome.devtools_open {
        let dt_top = win_h - DEVTOOLS_H;
        if y >= dt_top {
            if y < dt_top + 30.0 {
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
            if y >= win_h - 34.0 {
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

    fn shadow(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32) {
        let s = self.s;
        self.fb.draw_box_shadow(
            x * s,
            y * s,
            w * s,
            h * s,
            [r * s; 4],
            0.0,
            6.0 * s,
            16.0 * s,
            0.0,
            RgbaColor::new(0, 0, 0, 110),
            false,
        );
    }

    fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, t: f32, color: RgbaColor) {
        let s = self.s;
        self.fb.draw_line_aa(x0 * s, y0 * s, x1 * s, y1 * s, t * s, color);
    }

    fn circle_fill(&mut self, cx: f32, cy: f32, r: f32, color: RgbaColor) {
        let s = self.s;
        self.fb.draw_circle_fill(cx * s, cy * s, r * s, color);
    }

    fn circle_stroke(&mut self, cx: f32, cy: f32, r: f32, t: f32, color: RgbaColor) {
        let s = self.s;
        self.fb.draw_circle_stroke(cx * s, cy * s, r * s, t * s, color);
    }

    fn arc(&mut self, cx: f32, cy: f32, r: f32, start: f32, sweep: f32, t: f32, color: RgbaColor) {
        let s = self.s;
        self.fb.draw_arc_stroke(cx * s, cy * s, r * s, start, sweep, t * s, color);
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

    fn push_clip(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let s = self.s;
        self.fb.push_clip(ClipRect {
            x: (x * s).floor() as i32,
            y: (y * s).floor() as i32,
            w: (w * s).ceil() as i32,
            h: (h * s).ceil() as i32,
        });
    }

    fn pop_clip(&mut self) {
        self.fb.pop_clip();
    }
}

pub fn draw_chrome(
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    tabs: &[TabVisual<'_>],
    active_tab: usize,
    chrome: &ChromeState,
    scale: f32,
) {
    let s = scale.max(1.0);
    let w = fb.width as f32 / s;
    let h = fb.height as f32 / s;
    let pal = Palette::for_dark(chrome.dark_mode);
    let mut d = ScaledDraw { fb, fonts, s };

    d.rect(0.0, 0.0, w, TAB_BAR_H, pal.tab_strip);

    let tab_w = ((w - 64.0) / tabs.len().max(1) as f32).clamp(110.0, 224.0);
    let start_x = 8.0;
    for (i, tab) in tabs.iter().enumerate() {
        let tx = start_x + i as f32 * (tab_w + 3.0);
        let is_active = i == active_tab;
        let is_hover = chrome.hovered_hit == ChromeHit::Tab(i)
            || chrome.hovered_hit == ChromeHit::CloseTab(i);
        if is_active {
            d.rrect(tx, 6.0, tab_w, TAB_BAR_H - 6.0, [8.0, 8.0, 0.0, 0.0], pal.tab_active);
            d.rect(tx + 12.0, 6.0, tab_w - 24.0, 2.0, pal.accent);
        } else if is_hover {
            d.rrect(tx, 7.0, tab_w, TAB_BAR_H - 10.0, [7.0; 4], pal.tab_hover);
        } else if i + 1 < tabs.len() && i + 1 != active_tab {
            d.rect(tx + tab_w + 1.0, 11.0, 1.0, 18.0, pal.border);
        }

        let ico_cx = tx + 16.0;
        let ico_cy = 22.0;
        if tab.loading {
            d.arc(ico_cx, ico_cy, 5.5, chrome.anim_phase, 4.4, 1.7, pal.accent);
        } else if let Some(ref fav) = tab.page.favicon {
            d.fb.blit_rgba(
                fav.width,
                fav.height,
                &fav.rgba,
                (ico_cx - 7.0) * s,
                (ico_cy - 7.0) * s,
                14.0 * s,
                14.0 * s,
                1.0,
            );
        } else {
            d.circle_stroke(ico_cx, ico_cy, 5.5, 1.3, if is_active { pal.accent } else { pal.text_muted });
            d.circle_fill(ico_cx, ico_cy, 1.8, if is_active { pal.accent } else { pal.text_muted });
        }

        let max_title_w = (tab_w - 60.0).max(20.0);
        let title_face = if is_active { FontFaceKey::SansBold } else { FontFaceKey::SansRegular };
        let title = truncate_to_width(&tab.page.title(), max_title_w, d.fonts, title_face, 12.0);
        d.text(
            &title,
            tx + 28.0,
            26.0,
            title_face,
            12.0,
            if is_active { pal.text_hi } else { pal.text_dim },
        );

        let cx = tx + tab_w - 14.0;
        let cy = 22.0;
        if chrome.hovered_hit == ChromeHit::CloseTab(i) {
            d.circle_fill(cx, cy, 8.5, pal.btn_hover);
        }
        let xc = if chrome.hovered_hit == ChromeHit::CloseTab(i) {
            pal.text_hi
        } else {
            pal.text_muted
        };
        d.line(cx - 3.2, cy - 3.2, cx + 3.2, cy + 3.2, 1.35, xc);
        d.line(cx + 3.2, cy - 3.2, cx - 3.2, cy + 3.2, 1.35, xc);
    }

    let plus_x = start_x + tabs.len() as f32 * (tab_w + 3.0) + 4.0;
    let pcx = plus_x + 14.0;
    let pcy = 22.0;
    if chrome.hovered_hit == ChromeHit::NewTab {
        d.circle_fill(pcx, pcy, 12.0, pal.btn_hover);
    }
    let pc = if chrome.hovered_hit == ChromeHit::NewTab { pal.text_hi } else { pal.text_dim };
    d.line(pcx - 5.0, pcy, pcx + 5.0, pcy, 1.6, pc);
    d.line(pcx, pcy - 5.0, pcx, pcy + 5.0, 1.6, pc);

    let toolbar_end = TAB_BAR_H + TOOLBAR_H;
    d.rect(0.0, TAB_BAR_H, w, TOOLBAR_H, pal.toolbar);
    d.rect(0.0, toolbar_end - 1.0, w, 1.0, pal.border);

    let active = &tabs[active_tab];
    let page = active.page;

    draw_icon_btn(&mut d, 8.0, TAB_BAR_H + 7.0, chrome.hovered_hit == ChromeHit::Back, &pal, |d, cx, cy| {
        let c = if page.can_go_back() { pal.text_hi } else { pal.text_muted };
        d.line(cx + 5.0, cy, cx - 5.0, cy, 1.6, c);
        d.line(cx - 5.0, cy, cx - 0.5, cy - 4.5, 1.6, c);
        d.line(cx - 5.0, cy, cx - 0.5, cy + 4.5, 1.6, c);
    });
    draw_icon_btn(&mut d, 42.0, TAB_BAR_H + 7.0, chrome.hovered_hit == ChromeHit::Forward, &pal, |d, cx, cy| {
        let c = if page.can_go_forward() { pal.text_hi } else { pal.text_muted };
        d.line(cx - 5.0, cy, cx + 5.0, cy, 1.6, c);
        d.line(cx + 5.0, cy, cx + 0.5, cy - 4.5, 1.6, c);
        d.line(cx + 5.0, cy, cx + 0.5, cy + 4.5, 1.6, c);
    });
    draw_icon_btn(&mut d, 76.0, TAB_BAR_H + 7.0, chrome.hovered_hit == ChromeHit::Reload, &pal, |d, cx, cy| {
        if active.loading {
            d.line(cx - 4.5, cy - 4.5, cx + 4.5, cy + 4.5, 1.7, pal.text_hi);
            d.line(cx + 4.5, cy - 4.5, cx - 4.5, cy + 4.5, 1.7, pal.text_hi);
        } else {
            d.arc(cx, cy, 5.5, -0.8, 5.1, 1.6, pal.text_hi);
            d.line(cx + 3.8, cy - 4.2, cx + 6.4, cy - 3.6, 1.5, pal.text_hi);
            d.line(cx + 3.8, cy - 4.2, cx + 4.2, cy - 7.0, 1.5, pal.text_hi);
        }
    });

    let (omni_x, omni_y, omni_w, omni_h) = omnibox_rect(w);
    let omni_fill = if chrome.hovered_hit == ChromeHit::Omnibox && !chrome.omnibox_focused {
        pal.omni_hover
    } else {
        pal.omni_bg
    };
    d.rrect(omni_x, omni_y, omni_w, omni_h, [16.0; 4], omni_fill);
    let omni_border = if chrome.omnibox_focused { pal.accent } else { pal.border };
    d.stroke_rrect(
        omni_x,
        omni_y,
        omni_w,
        omni_h,
        [16.0; 4],
        [if chrome.omnibox_focused { 1.6 } else { 1.0 }; 4],
        [omni_border; 4],
    );

    let lock_cx = omni_x + 17.0;
    let lock_cy = omni_y + 16.0;
    if page.url.starts_with("https://") {
        d.rrect(lock_cx - 4.5, lock_cy - 0.5, 9.0, 6.8, [1.8; 4], pal.green);
        d.arc(lock_cx, lock_cy - 1.0, 3.0, std::f32::consts::PI, std::f32::consts::PI, 1.4, pal.green);
    } else if page.url.starts_with("peedle://") {
        d.circle_fill(lock_cx, lock_cy, 4.0, pal.accent);
    } else {
        d.circle_stroke(lock_cx, lock_cy, 4.8, 1.3, pal.text_muted);
    }

    let text_x = omni_x + 32.0;
    let text_w = (omni_w - 76.0).max(60.0);
    let cursor_px = chrome.omnibox.prefix_width(
        chrome.omnibox.cursor,
        d.fonts,
        FontFaceKey::SansRegular,
        13.0,
    );
    let scroll_x = if cursor_px > text_w - 8.0 {
        cursor_px - (text_w - 8.0)
    } else {
        0.0
    };
    d.push_clip(text_x - 2.0, omni_y + 2.0, text_w + 4.0, omni_h - 4.0);
    if chrome.omnibox_focused {
        if let Some((lo, hi)) = chrome.omnibox.selected_range() {
            let x0 = text_x - scroll_x
                + chrome.omnibox.prefix_width(lo, d.fonts, FontFaceKey::SansRegular, 13.0);
            let x1 = text_x - scroll_x
                + chrome.omnibox.prefix_width(hi, d.fonts, FontFaceKey::SansRegular, 13.0);
            d.rrect(x0, omni_y + 6.0, (x1 - x0).max(2.0), 20.0, [3.0; 4], pal.accent_sel);
        }
    }
    if chrome.omnibox.text.is_empty() {
        d.text(
            "Search or enter address",
            text_x,
            omni_y + 20.5,
            FontFaceKey::SansRegular,
            13.0,
            pal.text_muted,
        );
    } else {
        d.text(
            &chrome.omnibox.text,
            text_x - scroll_x,
            omni_y + 20.5,
            FontFaceKey::SansRegular,
            13.0,
            pal.text_hi,
        );
    }
    if chrome.omnibox_focused && chrome.omnibox.selected_range().is_none() {
        d.rect(text_x - scroll_x + cursor_px, omni_y + 7.0, 1.5, 18.0, pal.accent);
    }
    d.pop_clip();

    if (chrome.zoom - 1.0).abs() > 0.01 {
        let ztxt = format!("{}%", (chrome.zoom * 100.0).round() as i32);
        let zx = omni_x + omni_w - 78.0;
        d.rrect(zx, omni_y + 6.0, 42.0, 20.0, [10.0; 4], pal.btn_hover);
        d.text(&ztxt, zx + 7.0, omni_y + 19.5, FontFaceKey::MonoBold, 10.5, pal.accent);
    }

    let is_bookmarked = chrome.bookmarks.iter().any(|b| b.url == page.url);
    let star_cx = omni_x + omni_w - 17.0;
    let star_cy = omni_y + 16.0;
    if chrome.hovered_hit == ChromeHit::ToggleBookmark {
        d.circle_fill(star_cx, star_cy, 11.0, pal.btn_hover);
    }
    draw_star(&mut d, star_cx, star_cy, 6.2, is_bookmarked, if is_bookmarked { pal.gold } else { pal.text_dim });

    let menu_x = w - 38.0;
    let dt_x = menu_x - 34.0;
    let dl_x = dt_x - 34.0;

    draw_icon_btn(&mut d, dl_x, TAB_BAR_H + 7.0, chrome.hovered_hit == ChromeHit::DownloadsBtn, &pal, |d, cx, cy| {
        let c = if chrome.has_downloads { pal.accent } else { pal.text_dim };
        d.line(cx, cy - 5.5, cx, cy + 2.5, 1.6, c);
        d.line(cx - 3.5, cy - 0.8, cx, cy + 2.8, 1.6, c);
        d.line(cx + 3.5, cy - 0.8, cx, cy + 2.8, 1.6, c);
        d.line(cx - 5.5, cy + 5.5, cx + 5.5, cy + 5.5, 1.6, c);
    });

    draw_icon_btn(
        &mut d,
        dt_x,
        TAB_BAR_H + 7.0,
        chrome.hovered_hit == ChromeHit::ToggleDevTools || chrome.devtools_open,
        &pal,
        |d, cx, cy| {
            let c = if chrome.devtools_open { pal.accent } else { pal.text_dim };
            d.line(cx - 2.5, cy - 4.0, cx - 6.0, cy, 1.5, c);
            d.line(cx - 6.0, cy, cx - 2.5, cy + 4.0, 1.5, c);
            d.line(cx + 2.5, cy - 4.0, cx + 6.0, cy, 1.5, c);
            d.line(cx + 6.0, cy, cx + 2.5, cy + 4.0, 1.5, c);
            d.line(cx + 1.0, cy - 4.8, cx - 1.0, cy + 4.8, 1.3, c);
        },
    );

    draw_icon_btn(&mut d, menu_x, TAB_BAR_H + 7.0, chrome.hovered_hit == ChromeHit::MenuBtn, &pal, |d, cx, cy| {
        let c = pal.text_hi;
        d.circle_fill(cx, cy - 4.5, 1.4, c);
        d.circle_fill(cx, cy, 1.4, c);
        d.circle_fill(cx, cy + 4.5, 1.4, c);
    });

    if active.loading {
        let prog_w = (w * active.load_progress.clamp(0.12, 0.94)).max(60.0);
        d.rect(0.0, toolbar_end - 2.0, prog_w, 2.0, pal.accent);
    }

    let top_h = top_chrome_height(chrome);
    if chrome.bookmarks_visible {
        d.rect(0.0, toolbar_end, w, BOOKMARK_BAR_H, pal.bookmark_bar);
        d.rect(0.0, top_h - 1.0, w, 1.0, pal.border);
        let mut bx = 10.0;
        for (i, bm) in chrome.bookmarks.iter().enumerate() {
            let label = truncate_chars(&bm.title, 20);
            let bw = (label.chars().count() as f32 * 6.8 + 22.0).clamp(52.0, 160.0);
            if bx + bw > w - 12.0 {
                break;
            }
            let hovered = chrome.hovered_hit == ChromeHit::BookmarkItem(i);
            if hovered {
                d.rrect(bx, toolbar_end + 4.0, bw, 22.0, [6.0; 4], pal.btn_hover);
            }
            d.text(
                &label,
                bx + 10.0,
                toolbar_end + 19.0,
                FontFaceKey::SansRegular,
                11.5,
                if hovered { pal.text_hi } else { pal.text_dim },
            );
            bx += bw + 4.0;
        }
    }

    let vh = content_viewport_height(h, chrome);
    let content_h = page.js_env.dom.borrow().layout.content_height;
    if content_h > vh + 1.0 {
        let track_h = vh - 8.0;
        let thumb_h = ((vh / content_h) * track_h).clamp(28.0, track_h);
        let max_scroll = (content_h - vh).max(1.0);
        let thumb_y = top_h + 4.0 + (page.scroll_y / max_scroll) * (track_h - thumb_h);
        d.rrect(
            w - 9.0,
            thumb_y,
            6.0,
            thumb_h,
            [3.0; 4],
            RgbaColor::new(120, 134, 154, 165),
        );
    }

    if chrome.find_open {
        let fw = 300.0;
        let fx = w - fw - 18.0;
        let fy = top_h + 8.0;
        d.shadow(fx, fy, fw, 34.0, 10.0);
        d.rrect(fx, fy, fw, 34.0, [10.0; 4], pal.popup_bg);
        d.stroke_rrect(fx, fy, fw, 34.0, [10.0; 4], [1.2; 4], [pal.accent; 4]);
        let q = truncate_chars(&chrome.find.text, 24);
        let end = d.text(&q, fx + 12.0, fy + 21.5, FontFaceKey::SansRegular, 12.5, pal.text_hi);
        d.rect(end + 1.0, fy + 9.0, 1.5, 16.0, pal.accent);
        let counter = if chrome.find.text.is_empty() {
            String::new()
        } else if chrome.find_match_count == 0 {
            "0/0".to_string()
        } else {
            format!("{}/{}", chrome.find_match_index + 1, chrome.find_match_count)
        };
        d.text(
            &counter,
            fx + fw - 52.0,
            fy + 21.0,
            FontFaceKey::MonoRegular,
            11.0,
            pal.text_dim,
        );
    }

    if let Some(ref href) = page.hovered_href {
        let dt_off = if chrome.devtools_open { DEVTOOLS_H } else { 0.0 };
        let shown = truncate_chars(href, 86);
        let pill_w = (shown.chars().count() as f32 * 6.8 + 18.0).min(w * 0.72);
        let pill_y = h - dt_off - 24.0;
        d.rrect(6.0, pill_y, pill_w, 20.0, [5.0; 4], pal.popup_bg);
        d.stroke_rrect(6.0, pill_y, pill_w, 20.0, [5.0; 4], [1.0; 4], [pal.border; 4]);
        d.text(&shown, 12.0, pill_y + 14.0, FontFaceKey::MonoRegular, 11.0, pal.text_dim);
    }

    if let Some(ref toast) = chrome.download_toast {
        let dt_off = if chrome.devtools_open { DEVTOOLS_H } else { 0.0 };
        let ty = h - dt_off - 42.0;
        d.shadow(12.0, ty, 428.0, 36.0, 10.0);
        d.rrect(12.0, ty, 428.0, 36.0, [10.0; 4], pal.popup_bg);
        d.stroke_rrect(12.0, ty, 428.0, 36.0, [10.0; 4], [1.0; 4], [pal.border; 4]);
        d.circle_fill(28.0, ty + 18.0, 5.0, pal.green);
        let label = format!("Saved {} ({})", truncate_chars(&toast.filename, 24), fmt_bytes(toast.bytes));
        d.text(&label, 40.0, ty + 22.0, FontFaceKey::SansBold, 12.0, pal.text_hi);
        d.text("Show", 290.0, ty + 22.0, FontFaceKey::SansBold, 11.5, pal.accent);
        d.text("Downloads", 336.0, ty + 22.0, FontFaceKey::SansRegular, 11.5, pal.text_dim);
        d.line(418.0, ty + 14.0, 426.0, ty + 22.0, 1.4, pal.text_dim);
        d.line(426.0, ty + 14.0, 418.0, ty + 22.0, 1.4, pal.text_dim);
    }

    if chrome.devtools_open {
        draw_devtools(&mut d, page, chrome, h - DEVTOOLS_H, w, DEVTOOLS_H, &pal);
    }

    if chrome.omnibox_focused && !chrome.suggestions.is_empty() {
        let sy = omni_y + omni_h + 4.0;
        let row_h = 36.0;
        let sh = chrome.suggestions.len() as f32 * row_h + 10.0;
        d.shadow(omni_x, sy, omni_w, sh, 12.0);
        d.rrect(omni_x, sy, omni_w, sh, [12.0; 4], pal.popup_bg);
        d.stroke_rrect(omni_x, sy, omni_w, sh, [12.0; 4], [1.0; 4], [pal.border; 4]);
        for (i, sug) in chrome.suggestions.iter().enumerate() {
            let ry = sy + 5.0 + i as f32 * row_h;
            let active_row = chrome.suggestion_index == Some(i)
                || chrome.hovered_hit == ChromeHit::SuggestionItem(i);
            if active_row {
                d.rrect(omni_x + 5.0, ry, omni_w - 10.0, row_h - 2.0, [8.0; 4], pal.popup_hover);
            }
            let tag = match sug.kind {
                SuggestionKind::Bookmark => "BM",
                SuggestionKind::History => "HIS",
                SuggestionKind::Search => "WEB",
            };
            let tag_c = match sug.kind {
                SuggestionKind::Bookmark => pal.gold,
                SuggestionKind::History => pal.accent,
                SuggestionKind::Search => pal.green,
            };
            d.text(tag, omni_x + 14.0, ry + 22.0, FontFaceKey::MonoBold, 10.0, tag_c);
            let max_c = ((omni_w - 60.0) / 7.5).max(16.0) as usize;
            let title = truncate_chars(&sug.title, max_c / 2);
            let url_s = truncate_chars(&sug.url, max_c / 2);
            let tx_end = d.text(&title, omni_x + 44.0, ry + 22.0, FontFaceKey::SansBold, 12.5, pal.text_hi);
            d.text(&url_s, tx_end + 10.0, ry + 22.0, FontFaceKey::MonoRegular, 11.0, pal.text_dim);
        }
    }

    if let Some(ref sel) = chrome.select_popup {
        let item_h = 28.0;
        let vis = sel.options.len().min(10);
        let sh = vis as f32 * item_h + 8.0;
        d.shadow(sel.x, sel.y, sel.w, sh, 8.0);
        d.rrect(sel.x, sel.y, sel.w, sh, [8.0; 4], pal.popup_bg);
        d.stroke_rrect(sel.x, sel.y, sel.w, sh, [8.0; 4], [1.0; 4], [pal.border; 4]);
        for (i, opt) in sel.options.iter().take(vis).enumerate() {
            let ry = sel.y + 4.0 + i as f32 * item_h;
            let hi = chrome.hovered_hit == ChromeHit::SelectOptionItem(i) || i == sel.selected;
            if hi {
                d.rrect(sel.x + 4.0, ry, sel.w - 8.0, item_h - 2.0, [5.0; 4], pal.popup_hover);
            }
            d.text(
                &truncate_chars(opt, 36),
                sel.x + 12.0,
                ry + 18.0,
                if i == sel.selected { FontFaceKey::SansBold } else { FontFaceKey::SansRegular },
                12.5,
                if i == sel.selected { pal.accent } else { pal.text_hi },
            );
        }
    }

    if let Some(ref menu) = chrome.popup_menu {
        let item_h = 30.0;
        let mh = menu.items.len() as f32 * item_h + 10.0;
        d.shadow(menu.x, menu.y, menu.w, mh, 10.0);
        d.rrect(menu.x, menu.y, menu.w, mh, [10.0; 4], pal.popup_bg);
        d.stroke_rrect(menu.x, menu.y, menu.w, mh, [10.0; 4], [1.0; 4], [pal.border; 4]);
        for (i, item) in menu.items.iter().enumerate() {
            let ry = menu.y + 5.0 + i as f32 * item_h;
            let hi = item.enabled && chrome.hovered_hit == ChromeHit::PopupMenuItem(i);
            if hi {
                d.rrect(menu.x + 5.0, ry, menu.w - 10.0, item_h - 2.0, [6.0; 4], pal.popup_hover);
            }
            d.text(
                &item.label,
                menu.x + 14.0,
                ry + 19.5,
                FontFaceKey::SansRegular,
                12.5,
                if item.enabled { pal.text_hi } else { pal.text_muted },
            );
        }
    }
}

fn draw_icon_btn<F>(
    d: &mut ScaledDraw<'_>,
    x: f32,
    y: f32,
    hovered: bool,
    pal: &Palette,
    paint_icon: F,
) where
    F: FnOnce(&mut ScaledDraw<'_>, f32, f32),
{
    if hovered {
        d.rrect(x, y, 30.0, 30.0, [8.0; 4], pal.btn_hover);
    }
    paint_icon(d, x + 15.0, y + 15.0);
}

fn draw_star(d: &mut ScaledDraw<'_>, cx: f32, cy: f32, r: f32, filled: bool, color: RgbaColor) {
    let mut pts = Vec::with_capacity(10);
    for i in 0..10 {
        let ang = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
        let rad = if i % 2 == 0 { r } else { r * 0.44 };
        pts.push(((cx + rad * ang.cos()) * d.s, (cy + rad * ang.sin()) * d.s));
    }
    if filled {
        d.fb.fill_polygon(&[pts], color, false);
    } else {
        for i in 0..10 {
            let (x0, y0) = pts[i];
            let (x1, y1) = pts[(i + 1) % 10];
            d.fb.draw_line_aa(x0, y0, x1, y1, 1.2 * d.s, color);
        }
    }
}

fn draw_devtools(
    d: &mut ScaledDraw<'_>,
    page: &Page,
    chrome: &ChromeState,
    top: f32,
    w: f32,
    h: f32,
    pal: &Palette,
) {
    let tab = chrome.devtools_tab;
    d.rect(0.0, top, w, h, pal.omni_bg);
    d.rect(0.0, top, w, 30.0, pal.toolbar);
    d.rect(0.0, top, w, 1.0, pal.border);
    d.rect(0.0, top + 30.0, w, 1.0, pal.border);

    let tabs = [
        (DevToolsTab::Elements, "Elements", 14.0),
        (DevToolsTab::Console, "Console", 108.0),
        (DevToolsTab::Network, "Network", 198.0),
    ];
    for (kind, label, x) in tabs {
        let active = kind == tab;
        d.text(
            label,
            x,
            top + 19.0,
            FontFaceKey::SansBold,
            12.0,
            if active { pal.accent } else { pal.text_dim },
        );
        if active {
            d.rect(x, top + 28.0, 64.0, 2.0, pal.accent);
        }
    }

    let mut y = top + 48.0;
    let max_y = top + h - 38.0;
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
                d.text(&truncate_chars(&line, 66), 12.0, y, FontFaceKey::MonoRegular, 11.5, pal.text_hi);
                y += 16.0;
            }
            d.rect(split_x, top + 30.0, 1.0, h - 30.0, pal.border);
            let inspect_node = page.hovered_node.or_else(|| st.doc.body());
            let mut sy = top + 48.0;
            if let Some(nid) = inspect_node {
                if let Some(el) = st.doc.element(nid) {
                    let hdr = format!("<{}> computed", el.tag_name);
                    d.text(&hdr, split_x + 12.0, sy, FontFaceKey::MonoBold, 11.5, pal.accent);
                    sy += 18.0;
                }
                if let Some(r) = st.layout.bounding_rect(nid) {
                    let box_line = format!("box: {:.0}x{:.0} @ ({:.0}, {:.0})", r.width, r.height, r.x, r.y);
                    d.text(&box_line, split_x + 12.0, sy, FontFaceKey::MonoRegular, 11.0, pal.text_dim);
                    sy += 16.0;
                }
                if let Some(cs) = st.styles.get(nid) {
                    for prop in ["display", "position", "color", "background-color", "font-size", "margin", "padding"] {
                        if sy > top + h - 10.0 {
                            break;
                        }
                        let val = get_computed_property(cs, prop);
                        let row = format!("{}: {}", prop, truncate_chars(&val, 34));
                        d.text(&row, split_x + 12.0, sy, FontFaceKey::MonoRegular, 11.0, pal.text_hi);
                        sy += 15.0;
                    }
                }
            }
        }
        DevToolsTab::Console => {
            if page.console_logs.is_empty() {
                d.text("(no console output)", 12.0, y, FontFaceKey::MonoRegular, 11.5, pal.text_dim);
            } else {
                for line in page.console_logs.iter().rev().take(9).rev() {
                    if y > max_y {
                        break;
                    }
                    let c = if line.starts_with("[Error]") {
                        RgbaColor::new(248, 113, 113, 255)
                    } else if line.starts_with("=> ") {
                        pal.accent
                    } else {
                        pal.text_hi
                    };
                    d.text(&truncate_chars(line, 110), 12.0, y, FontFaceKey::MonoRegular, 11.5, c);
                    y += 16.0;
                }
            }
            let input_y = top + h - 32.0;
            d.rect(0.0, input_y - 2.0, w, 1.0, pal.border);
            d.rect(0.0, input_y - 1.0, w, 33.0, pal.toolbar);
            d.text(">", 12.0, input_y + 19.0, FontFaceKey::MonoBold, 12.0, pal.accent);
            let shown_in = truncate_chars(&chrome.console.text, 110);
            let end_x = d.text(&shown_in, 26.0, input_y + 19.0, FontFaceKey::MonoRegular, 12.0, pal.text_hi);
            if chrome.console_focused {
                d.rect(end_x + 1.0, input_y + 7.0, 1.5, 16.0, pal.accent);
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
                d.text(&truncate_chars(&row, 110), 12.0, y, FontFaceKey::MonoRegular, 11.5, pal.text_hi);
                y += 16.0;
            }
        }
    }
}

fn collect_dom_lines(
    doc: &peedle_html::Document,
    id: NodeId,
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

fn fmt_bytes(b: usize) -> String {
    if b >= 1024 * 1024 {
        format!("{:.1} MB", b as f64 / (1024.0 * 1024.0))
    } else if b >= 1024 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else {
        format!("{} B", b)
    }
}

fn truncate_to_width(
    s: &str,
    max_w: f32,
    fonts: &FontSystem,
    face: FontFaceKey,
    px: f32,
) -> String {
    if fonts.measure_text(s, face, px) <= max_w {
        return s.to_string();
    }
    let ell_w = fonts.measure_text("...", face, px);
    let budget = (max_w - ell_w).max(0.0);
    let mut out = String::new();
    let mut w = 0.0f32;
    let mut buf = [0u8; 4];
    for c in s.chars() {
        let cw = fonts.measure_text(c.encode_utf8(&mut buf), face, px);
        if w + cw > budget {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push_str("...");
    out
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
