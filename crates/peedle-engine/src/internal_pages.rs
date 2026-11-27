pub fn render_internal_page(url: &str) -> Option<(String, String)> {
    match url {
        "peedle://newtab" | "about:newtab" => Some((
            "New Tab - Peedle".to_string(),
            NEWTAB_HTML.to_string(),
        )),
        "peedle://about" | "about:peedle" => Some((
            "About Peedle".to_string(),
            ABOUT_HTML.to_string(),
        )),
        "peedle://demo" => Some((
            "Interactive Demo - Peedle".to_string(),
            DEMO_HTML.to_string(),
        )),
        "about:blank" => Some((
            "about:blank".to_string(),
            "<!DOCTYPE html><html><head><title>about:blank</title></head><body></body></html>".to_string(),
        )),
        _ => None,
    }
}

pub fn render_history_html(entries: &[(String, String)]) -> String {
    let mut rows = String::new();
    for (url, title) in entries.iter().rev() {
        let esc_url = url
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        let esc_title = title
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        rows.push_str(&format!(
            "<tr><td><a href=\"{}\">{}</a></td><td class=\"u\">{}</td></tr>",
            esc_url, esc_title, esc_url
        ));
    }
    if rows.is_empty() {
        rows.push_str("<tr><td class=\"u\">No history entries yet.</td><td></td></tr>");
    }
    format!(
        r##"<!DOCTYPE html><html><head><meta charset="utf-8"><title>History - Peedle</title><style>
body {{ margin:0; padding:48px; background:#0f1117; color:#e6edf3; font-family:sans-serif; }}
.card {{ max-width:780px; margin:0 auto; background:#171c26; border:1px solid #263042; border-radius:14px; padding:28px 32px; }}
h1 {{ margin:0 0 16px 0; font-size:26px; color:#f8fafc; }}
table {{ width:100%; border-collapse:collapse; }}
td {{ padding:10px 8px; border-bottom:1px solid #263042; font-size:14px; }}
a {{ color:#38bdf8; text-decoration:none; font-weight:600; }}
td.u {{ color:#94a3b8; font-family:monospace; font-size:12px; }}
</style></head><body><div class="card"><h1>Browsing History</h1><table>{}</table></div></body></html>"##,
        rows
    )
}

const NEWTAB_HTML: &str = r##"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>New Tab</title>
<style>
  * { box-sizing: border-box; }
  body {
    margin: 0;
    min-height: 100vh;
    background: #0f1117;
    color: #e6edf3;
    font-family: sans-serif;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding-top: 110px;
  }
  .brand {
    font-size: 46px;
    font-weight: 700;
    letter-spacing: -1px;
    color: #f8fafc;
    margin-bottom: 28px;
  }
  .brand span { color: #38bdf8; }
  .search-box {
    width: 560px;
    max-width: 90vw;
    display: flex;
    gap: 10px;
    margin-bottom: 42px;
  }
  .search-box input {
    flex: 1;
    height: 44px;
    padding: 0 16px;
    border-radius: 10px;
    border: 1px solid #2b3445;
    background: #171c26;
    color: #f1f5f9;
    font-size: 15px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(4, 130px);
    gap: 14px;
  }
  .tile {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 92px;
    border-radius: 12px;
    background: #171c26;
    border: 1px solid #263042;
    color: #e2e8f0;
    text-decoration: none;
    font-size: 14px;
    font-weight: 600;
  }
  .tile:hover {
    background: #1f2735;
    border-color: #38bdf8;
  }
  .badge {
    width: 34px;
    height: 34px;
    border-radius: 9px;
    background: #233044;
    color: #38bdf8;
    display: flex;
    align-items: center;
    justify-content: center;
    margin-bottom: 8px;
    font-size: 16px;
    font-weight: 700;
  }
</style>
</head>
<body>
  <div class="brand">peed<span>le</span></div>
  <form class="search-box" action="https://lite.duckduckgo.com/lite/" method="get">
    <input type="text" name="q" placeholder="Search the web with DuckDuckGo Lite...">
  </form>
  <div class="grid">
    <a class="tile" href="https://example.com"><div class="badge">E</div>Example</a>
    <a class="tile" href="https://news.ycombinator.com"><div class="badge">Y</div>Hacker News</a>
    <a class="tile" href="https://en.wikipedia.org/wiki/Web_browser"><div class="badge">W</div>Wikipedia</a>
    <a class="tile" href="peedle://demo"><div class="badge">D</div>Demo</a>
  </div>
</body>
</html>"##;

const ABOUT_HTML: &str = r##"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>About Peedle</title>
<style>
  body {
    margin: 0;
    padding: 48px;
    background: #0f1117;
    color: #e6edf3;
    font-family: sans-serif;
  }
  .card {
    max-width: 680px;
    margin: 0 auto;
    background: #171c26;
    border: 1px solid #263042;
    border-radius: 14px;
    padding: 28px 32px;
  }
  h1 { margin: 0 0 16px 0; font-size: 28px; color: #f8fafc; }
  table { width: 100%; border-collapse: collapse; margin-top: 12px; }
  td { padding: 10px 8px; border-bottom: 1px solid #263042; font-size: 14px; }
  td.k { color: #94a3b8; width: 180px; }
  td.v { color: #e2e8f0; font-family: monospace; }
</style>
</head>
<body>
  <div class="card">
    <h1>Peedle 0.1.0</h1>
    <table>
      <tr><td class="k">URL Engine</td><td class="v">peedle-url (WHATWG URL, WPT 915/915 parser &amp; 284/284 setters = 100%)</td></tr>
      <tr><td class="k">HTML Engine</td><td class="v">peedle-html (WHATWG HTML5, html5lib 1,765/1,765 tree = 100%)</td></tr>
      <tr><td class="k">CSS Engine</td><td class="v">peedle-css (CSS3/4 Cascade, RuleIndex, Nesting, Selectors L4, Acid1/2)</td></tr>
      <tr><td class="k">Layout Engine</td><td class="v">peedle-layout (Block, Inline, Float, Flex, Grid, Table, WPT 19/19)</td></tr>
      <tr><td class="k">JS Engine</td><td class="v">peedle-js (ES2025 Bytecode VM, GC, BigInt, Modules, ShadowRealm, Atomics, Test262 42,559/42,632 = 99.83%)</td></tr>
      <tr><td class="k">DOM &amp; Web APIs</td><td class="v">peedle-dom (DOM L4, Traversal, Range, Intl, Acid3 100/100, 14 modern libs)</td></tr>
      <tr><td class="k">Renderer</td><td class="v">peedle-render (Software AA 2D, Fontdue, SVG, Gradients, Canvas2D)</td></tr>
      <tr><td class="k">Networking</td><td class="v">peedle-net (HTTP/1.1 Keep-Alive, rustls TLS, Gzip/Brotli, Cookies, WebSockets)</td></tr>
    </table>
  </div>
</body>
</html>"##;

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
