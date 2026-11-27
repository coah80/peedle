use crate::page::SearchEngine;

pub struct SettingsView {
    pub dark_mode: bool,
    pub search_engine: SearchEngine,
    pub restore_session: bool,
    pub bookmarks_visible: bool,
    pub cookie_count: usize,
    pub history_count: usize,
    pub bookmark_count: usize,
    pub status_notice: Option<String>,
}

pub struct DownloadViewEntry {
    pub filename: String,
    pub url: String,
    pub path: String,
    pub bytes: usize,
    pub timestamp: String,
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn theme_palette(dark: bool) -> (&'static str, &'static str, &'static str, &'static str, &'static str, &'static str) {
    if dark {
        ("#0f1117", "#171c26", "#263042", "#e6edf3", "#94a3b8", "#38bdf8")
    } else {
        ("#f8fafc", "#ffffff", "#e2e8f0", "#0f172a", "#475569", "#0284c7")
    }
}

pub fn render_internal_page(url: &str) -> Option<(String, String)> {
    render_internal_page_themed(url, true, SearchEngine::DuckDuckGo)
}

pub fn render_internal_page_themed(
    url: &str,
    dark: bool,
    search: SearchEngine,
) -> Option<(String, String)> {
    match url {
        "peedle://newtab" | "about:newtab" => Some((
            "New Tab".to_string(),
            render_newtab_html(dark, search, &[]),
        )),
        "peedle://about" | "about:peedle" => Some((
            "About Peedle".to_string(),
            render_about_html(dark),
        )),
        "peedle://demo" => Some((
            "Interactive Engine Demo".to_string(),
            DEMO_HTML.to_string(),
        )),
        "about:blank" => Some((
            "about:blank".to_string(),
            "<!DOCTYPE html><html><head><title>about:blank</title></head><body></body></html>".to_string(),
        )),
        _ => None,
    }
}

pub fn render_newtab_html(
    dark: bool,
    search: SearchEngine,
    quick_links: &[(String, String)],
) -> String {
    let (bg, card, border, fg, muted, accent) = theme_palette(dark);
    let badge_bg = if dark { "#233044" } else { "#e0f2fe" };
    let tiles_src: Vec<(String, String)> = if quick_links.is_empty() {
        vec![
            ("Example".to_string(), "https://example.com".to_string()),
            ("Hacker News".to_string(), "https://news.ycombinator.com".to_string()),
            ("Wikipedia".to_string(), "https://en.wikipedia.org/wiki/Web_browser".to_string()),
            ("GitHub".to_string(), "https://github.com/coah80/peedle".to_string()),
            ("MDN Docs".to_string(), "https://developer.mozilla.org/en-US/".to_string()),
            ("Docs.rs".to_string(), "https://docs.rs/".to_string()),
            ("Demo".to_string(), "peedle://demo".to_string()),
            ("Settings".to_string(), "peedle://settings".to_string()),
        ]
    } else {
        quick_links.iter().take(8).cloned().collect()
    };
    let mut tiles = String::new();
    for (title, url) in &tiles_src {
        let letter = title
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_ascii_uppercase().to_string())
            .unwrap_or_else(|| "W".to_string());
        tiles.push_str(&format!(
            "<a class=\"tile\" href=\"{}\"><div class=\"badge\">{}</div><span>{}</span></a>",
            escape_html(url),
            escape_html(&letter),
            escape_html(title)
        ));
    }
    let placeholder = format!("Search with {} or enter URL", search.label());
    format!(
        r##"<!DOCTYPE html><html><head><meta charset="utf-8"><title>New Tab</title><style>
* {{ box-sizing: border-box; }}
body {{ margin:0; min-height:100vh; background:{bg}; color:{fg}; font-family:sans-serif; display:flex; flex-direction:column; align-items:center; padding-top:96px; }}
.brand {{ font-size:44px; font-weight:700; color:{fg}; margin-bottom:26px; }}
.brand span {{ color:{accent}; }}
.search-box {{ width:580px; max-width:90vw; display:flex; gap:10px; margin-bottom:38px; }}
.search-box input {{ flex:1; height:44px; padding:0 16px; border-radius:22px; border:1px solid {border}; background:{card}; color:{fg}; font-size:15px; }}
.grid {{ display:grid; grid-template-columns:repeat(4, 134px); gap:14px; }}
.tile {{ display:flex; flex-direction:column; align-items:center; justify-content:center; height:94px; border-radius:12px; background:{card}; border:1px solid {border}; color:{fg}; text-decoration:none; font-size:13px; font-weight:600; }}
.tile:hover {{ border-color:{accent}; }}
.badge {{ width:34px; height:34px; border-radius:9px; background:{badge_bg}; color:{accent}; display:flex; align-items:center; justify-content:center; margin-bottom:8px; font-size:15px; font-weight:700; }}
.foot {{ margin-top:34px; display:flex; gap:18px; font-size:12px; }}
.foot a {{ color:{muted}; text-decoration:none; padding:4px 10px; }}
.foot a:hover {{ color:{accent}; }}
</style></head><body>
<div class="brand">peed<span>le</span></div>
<form class="search-box" action="{action}" method="get">
  <input type="text" name="{qparam}" placeholder="{placeholder}">
</form>
<div class="grid">{tiles}</div>
<div class="foot">
  <a href="peedle://history">History</a>
  <a href="peedle://bookmarks">Bookmarks</a>
  <a href="peedle://downloads">Downloads</a>
  <a href="peedle://settings">Settings</a>
  <a href="peedle://about">About</a>
</div>
</body></html>"##,
        bg = bg,
        card = card,
        border = border,
        fg = fg,
        muted = muted,
        accent = accent,
        badge_bg = badge_bg,
        action = search.form_action(),
        qparam = search.query_param(),
        placeholder = escape_html(&placeholder),
        tiles = tiles,
    )
}

pub fn render_history_html(entries: &[(String, String)]) -> String {
    render_history_html_themed(entries, true)
}

pub fn render_history_html_themed(entries: &[(String, String)], dark: bool) -> String {
    let (bg, card, border, fg, muted, accent) = theme_palette(dark);
    let mut rows = String::new();
    for (url, title) in entries.iter().rev().take(250) {
        let esc_url = escape_html(url);
        let esc_title = escape_html(if title.is_empty() { url } else { title });
        rows.push_str(&format!(
            "<tr><td><a href=\"{}\">{}</a></td><td class=\"u\">{}</td></tr>",
            esc_url, esc_title, esc_url
        ));
    }
    if rows.is_empty() {
        rows.push_str("<tr><td class=\"u\">No browsing history yet.</td><td></td></tr>");
    }
    format!(
        r##"<!DOCTYPE html><html><head><meta charset="utf-8"><title>History</title><style>
body {{ margin:0; padding:40px; background:{bg}; color:{fg}; font-family:sans-serif; }}
.card {{ max-width:860px; margin:0 auto; background:{card}; border:1px solid {border}; border-radius:14px; padding:26px 30px; }}
.hdr {{ display:flex; justify-content:space-between; align-items:center; margin-bottom:16px; }}
h1 {{ margin:0; font-size:24px; color:{fg}; }}
.btn {{ padding:7px 14px; border-radius:8px; border:1px solid {border}; color:{fg}; text-decoration:none; font-size:13px; }}
.btn:hover {{ border-color:{accent}; color:{accent}; }}
table {{ width:100%; border-collapse:collapse; }}
td {{ padding:10px 8px; border-bottom:1px solid {border}; font-size:14px; }}
a {{ color:{accent}; text-decoration:none; font-weight:600; }}
td.u {{ color:{muted}; font-family:monospace; font-size:12px; }}
</style></head><body><div class="card">
<div class="hdr"><h1>History</h1><a class="btn" href="peedle://settings?action=clear_history">Clear History</a></div>
<table>{rows}</table></div></body></html>"##,
        bg = bg,
        card = card,
        border = border,
        fg = fg,
        muted = muted,
        accent = accent,
        rows = rows,
    )
}

pub fn render_bookmarks_html(bookmarks: &[(String, String)], dark: bool) -> String {
    let (bg, card, border, fg, muted, accent) = theme_palette(dark);
    let mut rows = String::new();
    for (idx, (title, url)) in bookmarks.iter().enumerate() {
        let esc_url = escape_html(url);
        let esc_title = escape_html(title);
        rows.push_str(&format!(
            "<tr><td><a href=\"{u}\">{t}</a></td><td class=\"u\">{u}</td><td class=\"act\"><a class=\"del\" href=\"peedle://bookmarks?remove={i}\">Remove</a></td></tr>",
            u = esc_url,
            t = esc_title,
            i = idx
        ));
    }
    if rows.is_empty() {
        rows.push_str("<tr><td class=\"u\">No bookmarks saved.</td><td></td><td></td></tr>");
    }
    format!(
        r##"<!DOCTYPE html><html><head><meta charset="utf-8"><title>Bookmarks</title><style>
body {{ margin:0; padding:40px; background:{bg}; color:{fg}; font-family:sans-serif; }}
.card {{ max-width:860px; margin:0 auto; background:{card}; border:1px solid {border}; border-radius:14px; padding:26px 30px; }}
h1 {{ margin:0 0 16px 0; font-size:24px; color:{fg}; }}
table {{ width:100%; border-collapse:collapse; }}
td {{ padding:10px 8px; border-bottom:1px solid {border}; font-size:14px; }}
a {{ color:{accent}; text-decoration:none; font-weight:600; }}
td.u {{ color:{muted}; font-family:monospace; font-size:12px; }}
td.act {{ text-align:right; }}
a.del {{ color:#f87171; font-size:12px; font-weight:500; }}
</style></head><body><div class="card"><h1>Bookmarks</h1><table>{rows}</table></div></body></html>"##,
        bg = bg,
        card = card,
        border = border,
        fg = fg,
        muted = muted,
        accent = accent,
        rows = rows,
    )
}

pub fn render_downloads_html(items: &[DownloadViewEntry], dark: bool) -> String {
    let (bg, card, border, fg, muted, accent) = theme_palette(dark);
    let mut rows = String::new();
    for (idx, item) in items.iter().rev().enumerate() {
        let real_idx = items.len().saturating_sub(1 + idx);
        let size_str = format_bytes(item.bytes);
        rows.push_str(&format!(
            "<tr><td><strong>{}</strong><div class=\"u\">{}</div></td><td class=\"u\">{}</td><td class=\"u\">{}</td><td class=\"act\"><a href=\"peedle://downloads?open={}\">Show in folder</a></td></tr>",
            escape_html(&item.filename),
            escape_html(&item.path),
            size_str,
            escape_html(&item.timestamp),
            real_idx,
        ));
    }
    if rows.is_empty() {
        rows.push_str("<tr><td class=\"u\">No downloads yet.</td><td></td><td></td><td></td></tr>");
    }
    format!(
        r##"<!DOCTYPE html><html><head><meta charset="utf-8"><title>Downloads</title><style>
body {{ margin:0; padding:40px; background:{bg}; color:{fg}; font-family:sans-serif; }}
.card {{ max-width:900px; margin:0 auto; background:{card}; border:1px solid {border}; border-radius:14px; padding:26px 30px; }}
.hdr {{ display:flex; justify-content:space-between; align-items:center; margin-bottom:16px; }}
h1 {{ margin:0; font-size:24px; color:{fg}; }}
.btn {{ padding:7px 14px; border-radius:8px; border:1px solid {border}; color:{fg}; text-decoration:none; font-size:13px; }}
table {{ width:100%; border-collapse:collapse; }}
td {{ padding:12px 8px; border-bottom:1px solid {border}; font-size:14px; }}
a {{ color:{accent}; text-decoration:none; font-weight:600; }}
.u {{ color:{muted}; font-family:monospace; font-size:12px; margin-top:3px; }}
td.act {{ text-align:right; }}
</style></head><body><div class="card">
<div class="hdr"><h1>Downloads</h1><a class="btn" href="peedle://downloads?clear=1">Clear List</a></div>
<table>{rows}</table></div></body></html>"##,
        bg = bg,
        card = card,
        border = border,
        fg = fg,
        muted = muted,
        accent = accent,
        rows = rows,
    )
}

pub fn render_settings_html(view: &SettingsView) -> String {
    let (bg, card, border, fg, muted, accent) = theme_palette(view.dark_mode);
    let active_bg = if view.dark_mode { "#1e3a5f" } else { "#e0f2fe" };
    let notice = view
        .status_notice
        .as_deref()
        .map(|n| format!("<div class=\"notice\">{}</div>", escape_html(n)))
        .unwrap_or_default();

    let engine_btn = |e: SearchEngine| {
        let cls = if e == view.search_engine { "pill active" } else { "pill" };
        format!(
            "<a class=\"{}\" href=\"peedle://settings?search={}\">{}</a>",
            cls,
            e.as_id(),
            e.label()
        )
    };
    let engines = format!(
        "{} {} {} {}",
        engine_btn(SearchEngine::DuckDuckGo),
        engine_btn(SearchEngine::Google),
        engine_btn(SearchEngine::Brave),
        engine_btn(SearchEngine::Bing),
    );
    let dark_cls = if view.dark_mode { "pill active" } else { "pill" };
    let light_cls = if !view.dark_mode { "pill active" } else { "pill" };
    let restore_cls = if view.restore_session { "pill active" } else { "pill" };
    let newtab_cls = if !view.restore_session { "pill active" } else { "pill" };
    let bm_on = if view.bookmarks_visible { "pill active" } else { "pill" };
    let bm_off = if !view.bookmarks_visible { "pill active" } else { "pill" };

    format!(
        r##"<!DOCTYPE html><html><head><meta charset="utf-8"><title>Settings</title><style>
body {{ margin:0; padding:40px; background:{bg}; color:{fg}; font-family:sans-serif; }}
.wrap {{ max-width:780px; margin:0 auto; }}
h1 {{ margin:0 0 20px 0; font-size:26px; color:{fg}; }}
.card {{ background:{card}; border:1px solid {border}; border-radius:14px; padding:20px 26px; margin-bottom:16px; }}
.row {{ display:flex; justify-content:space-between; align-items:center; padding:12px 0; border-bottom:1px solid {border}; }}
.row:last-child {{ border-bottom:none; }}
.lbl {{ font-size:14px; font-weight:600; color:{fg}; }}
.sub {{ font-size:12px; color:{muted}; margin-top:3px; }}
.pills {{ display:flex; gap:8px; }}
.pill {{ padding:7px 14px; border-radius:8px; border:1px solid {border}; color:{fg}; text-decoration:none; font-size:13px; font-weight:500; }}
.pill.active {{ background:{active_bg}; border-color:{accent}; color:{accent}; }}
.pill.danger {{ color:#f87171; }}
.notice {{ background:{active_bg}; border:1px solid {accent}; color:{accent}; padding:10px 14px; border-radius:10px; margin-bottom:16px; font-size:13px; }}
</style></head><body><div class="wrap">
<h1>Settings</h1>
{notice}
<div class="card">
  <div class="row">
    <div><div class="lbl">Appearance</div><div class="sub">Browser chrome and prefers-color-scheme</div></div>
    <div class="pills">
      <a class="{dark_cls}" href="peedle://settings?theme=dark">Dark</a>
      <a class="{light_cls}" href="peedle://settings?theme=light">Light</a>
    </div>
  </div>
  <div class="row">
    <div><div class="lbl">Search Engine</div><div class="sub">Used in the address bar and New Tab</div></div>
    <div class="pills">{engines}</div>
  </div>
  <div class="row">
    <div><div class="lbl">On Startup</div><div class="sub">Open previous tabs or a fresh New Tab</div></div>
    <div class="pills">
      <a class="{restore_cls}" href="peedle://settings?restore=1">Restore Tabs</a>
      <a class="{newtab_cls}" href="peedle://settings?restore=0">New Tab</a>
    </div>
  </div>
  <div class="row">
    <div><div class="lbl">Bookmarks Bar</div><div class="sub">{bm_count} saved bookmarks</div></div>
    <div class="pills">
      <a class="{bm_on}" href="peedle://settings?bookmarks_bar=1">Shown</a>
      <a class="{bm_off}" href="peedle://settings?bookmarks_bar=0">Hidden</a>
    </div>
  </div>
</div>
<div class="card">
  <div class="row">
    <div><div class="lbl">Browsing History</div><div class="sub">{hist_count} saved entries</div></div>
    <div class="pills"><a class="pill danger" href="peedle://settings?action=clear_history">Clear History</a></div>
  </div>
  <div class="row">
    <div><div class="lbl">Cookies</div><div class="sub">{cookie_count} stored cookies</div></div>
    <div class="pills"><a class="pill danger" href="peedle://settings?action=clear_cookies">Clear Cookies</a></div>
  </div>
  <div class="row">
    <div><div class="lbl">HTTP Cache</div><div class="sub">Cached stylesheets, scripts, and images</div></div>
    <div class="pills"><a class="pill" href="peedle://settings?action=clear_cache">Clear Cache</a></div>
  </div>
</div>
</div></body></html>"##,
        bg = bg,
        card = card,
        border = border,
        fg = fg,
        muted = muted,
        accent = accent,
        active_bg = active_bg,
        notice = notice,
        dark_cls = dark_cls,
        light_cls = light_cls,
        engines = engines,
        restore_cls = restore_cls,
        newtab_cls = newtab_cls,
        bm_count = view.bookmark_count,
        bm_on = bm_on,
        bm_off = bm_off,
        hist_count = view.history_count,
        cookie_count = view.cookie_count,
    )
}

pub fn render_view_source_html(url: &str, source: &str, dark: bool) -> String {
    let (bg, card, border, fg, muted, accent) = theme_palette(dark);
    let mut lines_html = String::new();
    for (idx, line) in source.lines().take(2000).enumerate() {
        lines_html.push_str(&format!(
            "<tr><td class=\"ln\">{}</td><td class=\"code\">{}</td></tr>",
            idx + 1,
            escape_html(line)
        ));
    }
    format!(
        r##"<!DOCTYPE html><html><head><meta charset="utf-8"><title>view-source:{title}</title><style>
body {{ margin:0; padding:24px; background:{bg}; color:{fg}; font-family:monospace; font-size:13px; }}
.hdr {{ margin-bottom:14px; color:{accent}; font-family:sans-serif; font-size:14px; font-weight:600; }}
.box {{ background:{card}; border:1px solid {border}; border-radius:10px; padding:12px; overflow:auto; }}
table {{ border-collapse:collapse; width:100%; }}
td.ln {{ color:{muted}; text-align:right; padding:1px 12px 1px 4px; width:48px; vertical-align:top; }}
td.code {{ white-space:pre-wrap; padding:1px 4px; }}
</style></head><body>
<div class="hdr">Source: {title}</div>
<div class="box"><table>{lines}</table></div>
</body></html>"##,
        bg = bg,
        card = card,
        border = border,
        fg = fg,
        muted = muted,
        accent = accent,
        title = escape_html(url),
        lines = lines_html,
    )
}

fn render_about_html(dark: bool) -> String {
    let (bg, card, border, fg, muted, _accent) = theme_palette(dark);
    format!(
        r##"<!DOCTYPE html><html><head><meta charset="utf-8"><title>About Peedle</title><style>
body {{ margin:0; padding:48px; background:{bg}; color:{fg}; font-family:sans-serif; }}
.card {{ max-width:720px; margin:0 auto; background:{card}; border:1px solid {border}; border-radius:14px; padding:28px 32px; }}
h1 {{ margin:0 0 16px 0; font-size:28px; color:{fg}; }}
table {{ width:100%; border-collapse:collapse; margin-top:12px; }}
td {{ padding:10px 8px; border-bottom:1px solid {border}; font-size:14px; }}
td.k {{ color:{muted}; width:160px; }}
td.v {{ color:{fg}; font-family:monospace; }}
</style></head><body><div class="card">
<h1>Peedle 0.2.0</h1>
<table>
  <tr><td class="k">URL Engine</td><td class="v">peedle-url (WHATWG URL, WPT 915/915 parser, 284/284 setters)</td></tr>
  <tr><td class="k">HTML Engine</td><td class="v">peedle-html (WHATWG HTML5, html5lib 1,765/1,765 tree)</td></tr>
  <tr><td class="k">CSS Engine</td><td class="v">peedle-css (CSS3/4 Cascade, Nesting, @layer, Selectors L4)</td></tr>
  <tr><td class="k">Layout Engine</td><td class="v">peedle-layout (Block, Inline, Float, Flex, Grid, Table)</td></tr>
  <tr><td class="k">JS Engine</td><td class="v">peedle-js (ES2025 Bytecode VM, GC, Modules, Test262 99.83%)</td></tr>
  <tr><td class="k">DOM &amp; APIs</td><td class="v">peedle-dom (DOM L4, Traversal, Range, Intl, Acid3 100/100)</td></tr>
  <tr><td class="k">Renderer</td><td class="v">peedle-render (HiDPI 2D AA, Fontdue, WOFF, SVG, Canvas2D)</td></tr>
  <tr><td class="k">Networking</td><td class="v">peedle-net (HTTP/1.1 Keep-Alive, rustls TLS, Cache, Cookies, WS)</td></tr>
</table>
</div></body></html>"##,
        bg = bg,
        card = card,
        border = border,
        fg = fg,
        muted = muted,
    )
}

fn format_bytes(b: usize) -> String {
    if b >= 1024 * 1024 {
        format!("{:.1} MB", b as f64 / (1024.0 * 1024.0))
    } else if b >= 1024 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else {
        format!("{} B", b)
    }
}

const DEMO_HTML: &str = r##"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>Interactive Engine Demo</title>
<style>
  * { box-sizing: border-box; }
  body { margin: 0; padding: 32px; background: #0f1117; color: #e6edf3; font-family: sans-serif; }
  .wrap { max-width: 960px; margin: 0 auto; }
  .header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px; }
  h1 { margin: 0; font-size: 28px; color: #f8fafc; }
  .pill { background: #1e293b; border: 1px solid #334155; border-radius: 999px; padding: 6px 14px; font-size: 12px; color: #38bdf8; font-family: monospace; }
  .grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 16px; margin-bottom: 20px; }
  .card { background: #171c26; border: 1px solid #263042; border-radius: 12px; padding: 20px; }
  .label { font-size: 12px; color: #94a3b8; text-transform: uppercase; letter-spacing: 0.05em; }
  .metric { font-size: 32px; font-weight: 700; color: #f8fafc; margin-top: 6px; }
  .accent { color: #38bdf8; }
  .green { color: #4ade80; }
  .panels { display: grid; grid-template-columns: 1.4fr 1fr; gap: 16px; }
  .controls { display: flex; gap: 10px; margin-top: 14px; align-items: center; }
  button { background: #2563eb; color: #ffffff; border: 0; border-radius: 8px; padding: 8px 16px; font-size: 13px; font-weight: 600; cursor: pointer; }
  button.sec { background: #1e293b; color: #e2e8f0; border: 1px solid #334155; }
  canvas { display: block; border-radius: 8px; background: #0b0f17; margin-top: 12px; }
</style>
</head>
<body>
<div class="wrap">
  <div class="header">
    <h1>Peedle Engine Demo</h1>
    <div class="pill">100% Pure Rust Stack</div>
  </div>
  <div class="grid">
    <div class="card"><div class="label">Conformance</div><div class="metric green">Acid 1/2/3</div></div>
    <div class="card"><div class="label">TC39 Test262</div><div class="metric accent">99.83%</div></div>
    <div class="card"><div class="label">Interactive Counter</div><div class="metric" id="counter">0</div></div>
  </div>
  <div class="panels">
    <div class="card">
      <div class="label">Canvas2D Live Chart</div>
      <canvas id="chart" width="480" height="190"></canvas>
      <div class="controls">
        <button id="btn-inc" type="button">Increment +1</button>
        <button id="btn-step" class="sec" type="button">Rotate Data</button>
      </div>
    </div>
    <div class="card">
      <div class="label">Inline SVG Vector Gauge</div>
      <svg width="280" height="190" viewBox="0 0 280 190">
        <rect x="10" y="14" width="260" height="162" rx="12" fill="#0b0f17" stroke="#263042" stroke-width="1"/>
        <circle cx="140" cy="95" r="56" fill="none" stroke="#1e293b" stroke-width="14"/>
        <path d="M 84 95 A 56 56 0 1 1 196 95" fill="none" stroke="#38bdf8" stroke-width="14"/>
        <circle cx="140" cy="95" r="10" fill="#4ade80"/>
      </svg>
    </div>
  </div>
</div>
<script>
  let count = 0;
  let bars = [62, 88, 74, 96, 82, 100];
  const labels = ['URL', 'HTML', 'CSS', 'Lay', 'JS', 'DOM'];
  function drawChart() {
    const cv = document.getElementById('chart');
    const ctx = cv.getContext('2d');
    ctx.fillStyle = '#0b0f17';
    ctx.fillRect(0, 0, 480, 190);
    ctx.strokeStyle = '#1e293b';
    for (let y = 30; y <= 150; y += 40) {
      ctx.beginPath();
      ctx.moveTo(20, y);
      ctx.lineTo(460, y);
      ctx.stroke();
    }
    const colors = ['#38bdf8', '#818cf8', '#34d399', '#fbbf24', '#f472b6', '#22d3ee'];
    ctx.font = '11px sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'top';
    for (let i = 0; i < bars.length; i++) {
      const h = Math.round(bars[i] * 1.25);
      const x = 36 + i * 72;
      const y = 160 - h;
      ctx.fillStyle = colors[i % colors.length];
      ctx.fillRect(x, y, 42, h);
      ctx.fillStyle = '#94a3b8';
      ctx.fillText(labels[i], x + 21, 166);
    }
  }
  document.getElementById('btn-inc').addEventListener('click', () => {
    count += 1;
    document.getElementById('counter').textContent = String(count);
    console.log('Counter clicked:', count);
  });
  document.getElementById('btn-step').addEventListener('click', () => {
    bars.push(bars.shift());
    drawChart();
  });
  drawChart();
</script>
</body>
</html>"##;
