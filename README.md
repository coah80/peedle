# Peedle

Peedle is a standalone web browser built from scratch in Rust, completely independent of Chromium/Blink/V8, Firefox/Gecko/SpiderMonkey, WebKit, Servo, Ladybird, Boa, QuickJS, html5ever, cssparser, taffy, or regex crates.

## Architecture

10 Rust crates covering the full browser stack from scratch:

- `peedle-url`: WHATWG URL parser, serializer, IDNA/Punycode, and `URLSearchParams`
- `peedle-net`: HTTP/1.1 client with keep-alive connection pooling, `rustls` TLS, gzip/deflate/brotli decompression, cookie jar, RFC 6455 WebSockets, `file:`, and `data:` schemes
- `peedle-html`: WHATWG HTML5 tokenizer, tree builder (adoption agency, active formatting elements, `<template>` contents, SVG/MathML foreign content), and arena-backed DOM tree
- `peedle-css`: CSS Syntax L3 tokenizer/parser, Selectors L4 (`:is()`, `:where()`, `:not()`, `:has()`, `:nth-child(An+B of S)`), indexed `RuleIndex` + copy-on-write custom properties, `@layer`, `@media` (range syntax, `prefers-color-scheme`, `forced-colors`), `@supports`, CSS Nesting, logical properties, `calc()`/`min()`/`max()`/`clamp()`, modern color spaces (`oklab`, `oklch`, `color-mix()`, `light-dark()`), and CSSOM
- `peedle-layout`: Block, inline, float, BFC isolation, positioned (`relative`/`absolute`/`fixed`/`sticky`), Flexbox, CSS Grid (`grid-template-areas`, `minmax()`, `repeat()`, `fr`), tables (`colspan`, `table-caption`, min-word width), and intrinsic sizing
- `peedle-js`: From-scratch ES2025 lexer, parser, bytecode compiler, VM, mark-and-sweep GC with host root tracing, arbitrary-precision `BigInt`, full RegExp engine (lookbehind, named groups, `v`-flag `unicodeSets`, `RegExp.escape`), Promises, `async`/`await`, `AsyncGenerator`, `Proxy`/`Reflect`, ES Modules + Top-Level Await + `import.meta`, `ShadowRealm`, `Atomics` (`wait`/`notify`/`waitAsync`), `SharedArrayBuffer`, `Iterator` helpers, `DisposableStack`/`using`/`await using`, and Proper Tail Calls
- `peedle-render`: Anti-aliased software 2D rasterizer, SDF rounded rects, linear/radial/conic gradients, outer/inset box shadows, CSS `mask-image`, `object-fit`, multi-font TrueType/OpenType text (`fontdue` + CJK/Unicode fallback), inline SVG renderer (`currentColor`, `<use>`/`<symbol>`, arcs, `evenodd`), and Canvas 2D
- `peedle-dom`: DOM Living Standard bindings, Events (capture/target/bubble/`once`), Shadow DOM, Custom Elements, `MutationObserver`, `Range`, `TreeWalker`, `NodeIterator`, `DOMParser`/`XMLSerializer`, `XPathEvaluator`, `Intl`, `Fetch`/`Headers`/`FormData`/`AbortController`, `WebCrypto`, `Storage`, and `CanvasRenderingContext2D`
- `peedle-engine`: Page coordinator, parallel subresource fetching, event loop, hit testing, interactive forms, `<details>`, built-in pages (`peedle://newtab`, `peedle://demo`, `peedle://history`, `peedle://about`), and DevTools state
- `peedle`: Native desktop browser GUI (`winit` + `softbuffer`) with multi-tab bar, omnibox, bookmarks bar, zoom, Find-in-Page match highlighting, DevTools (Elements, Console REPL, Network), and headless screenshot CLI

## Verified Conformance

| Suite | Result |
| --- | --- |
| WPT URL (`urltestdata.json`) | **915 / 915 (100.0%)** |
| WPT URL (`setters_tests.json`) | **284 / 284 (100.0%)** |
| html5lib-tests Tree Construction | **1,765 / 1,765 (100.0%)** |
| html5lib-tests Tokenizer | **6,677 / 6,705 (99.6%)** |
| Acid1 | **100% pixel-perfect** |
| Acid2 | **100% pixel-perfect (0 red pixels)** |
| Acid3 | **100 / 100 (0 failures, pixel-perfect)** |
| TC39 Test262 (234 non-overlapping suites) | **42,559 / 42,632 (99.83%)** in ~14s |
| WPT CSS / Layout / DOM | **8/8 CSS, 19/19 Layout, 8/8 DOM** |

## Real-World Libraries & Frameworks Tested

Verified against real production bundles in `crates/peedle-dom/tests/modern_libs.rs` and `crates/peedle-js/tests/js_core.rs`:

- **React 18** (mount, click handlers, full interactive Todo app)
- **Vue 3** (reactivity, `computed` Todo workflow)
- **Preact**
- **Alpine.js 3.14** (`Proxy`-backed reactive state)
- **D3 v7** (scales & SVG bar chart data join)
- **Chart.js 4** (Canvas 2D bar chart rendering + `Intl.NumberFormat` ticks)
- **htmx 2** (`DOMContentLoaded`, `XPathEvaluator`, `DOMParser`, `<template>` swaps)
- **Redux 4** (store, subscribers, DOM click dispatch)
- **PrismJS** (RegExp syntax highlighting into DOM)
- **jQuery 3.7**, **Marked**, and **Lodash**

## Live Websites Verified Visually

Rendered and inspected end to end: Wikipedia (`Rust`, `Web browser`), GitHub (`coah80/peedle`), MDN Web Docs, Go.dev, Python.org, Rust-lang.org, SQLite.org, Ziglang.org, Docs.rs, Hacker News, Lobsters, DuckDuckGo Lite, and CNN Lite.

## Keyboard Shortcuts

- `Ctrl+T` / `Ctrl+W`: New / Close tab
- `Ctrl+Tab` / `Ctrl+Shift+Tab` / `Ctrl+1..9`: Switch tabs
- `Ctrl+L`: Focus omnibox
- `Ctrl+R` / `F5`: Reload
- `Alt+Left` / `Alt+Right`: Back / Forward
- `Ctrl+F`: Find in page (`Enter` / `Shift+Enter` next/prev match, `Esc` close)
- `Ctrl+D` / `Ctrl+B`: Bookmark page / Toggle bookmarks bar
- `Ctrl+H`: Open history (`peedle://history`)
- `Ctrl++` / `Ctrl+-` / `Ctrl+0`: Zoom in / out / reset
- `F12`: Toggle DevTools (Elements, Console REPL, Network)

## Building and Running

```bash
cargo build --release
cargo run --release -- https://en.wikipedia.org/wiki/Rust_(programming_language)
cargo run --release -- --headless --screenshot https://example.com out.png
cargo test --release --workspace
```
