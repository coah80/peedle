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
        "about:blank" => Some((
            "about:blank".to_string(),
            "<!DOCTYPE html><html><head><title>about:blank</title></head><body></body></html>".to_string(),
        )),
        _ => None,
    }
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
  <div class="grid">
    <a class="tile" href="https://example.com"><div class="badge">E</div>Example</a>
    <a class="tile" href="https://news.ycombinator.com"><div class="badge">Y</div>Hacker News</a>
    <a class="tile" href="https://en.wikipedia.org/wiki/Web_browser"><div class="badge">W</div>Wikipedia</a>
    <a class="tile" href="peedle://about"><div class="badge">P</div>About</a>
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
      <tr><td class="k">URL Engine</td><td class="v">peedle-url (WHATWG URL, 100% WPT parser &amp; setters)</td></tr>
      <tr><td class="k">HTML Engine</td><td class="v">peedle-html (WHATWG HTML5, 100% html5lib tree-construction)</td></tr>
      <tr><td class="k">CSS Engine</td><td class="v">peedle-css (CSS3/4 Cascade, Nesting, Selectors L4)</td></tr>
      <tr><td class="k">Layout Engine</td><td class="v">peedle-layout (Block, Inline, Float, Flex, Grid, Table)</td></tr>
      <tr><td class="k">JS Engine</td><td class="v">peedle-js (ES2024 Bytecode VM, GC, RegExp, Promises, Proxy)</td></tr>
      <tr><td class="k">DOM &amp; Web APIs</td><td class="v">peedle-dom (DOM L4, Events, Canvas2D, Fetch, Storage, Crypto)</td></tr>
      <tr><td class="k">Renderer</td><td class="v">peedle-render (Software AA 2D + Fontdue + SVG + Canvas2D)</td></tr>
      <tr><td class="k">Networking</td><td class="v">peedle-net (HTTP/1.1 + rustls TLS + Cookies + WebSockets)</td></tr>
    </table>
  </div>
</body>
</html>"##;
