# Peedle

Peedle is a standalone web browser built from scratch in Rust, completely independent of Chromium, Gecko, and WebKit.

## Architecture

- `peedle-url`: WHATWG URL parser, serializer, IDNA/Punycode, and `URLSearchParams`
- `peedle-net`: HTTP/1.1 client with TLS (`rustls`), compression, cookie jar, WebSockets, `file:`, and `data:` schemes
- `peedle-html`: WHATWG HTML5 tokenizer, tree builder, and arena-backed DOM tree
- `peedle-css`: CSS Syntax L3 parser, Selectors L4 matcher, Cascade L4/5, custom properties, `calc()`, modern colors, and CSSOM
- `peedle-layout`: Block, inline, float, positioned, Flexbox, Grid, and table layout engine
- `peedle-js`: From-scratch ES2024 JavaScript lexer, parser, bytecode compiler, VM, garbage collector, RegExp engine, Promises, Proxy/Reflect, and ES modules
- `peedle-render`: 2D display list, anti-aliased software rasterizer, TrueType text rendering, SVG, Canvas 2D, and image decoding
- `peedle-dom`: DOM Living Standard bindings, Events, Shadow DOM, Custom Elements, MutationObserver, Fetch, Storage, Crypto, and Canvas 2D API
- `peedle-engine`: Page coordinator, event loop, hit testing, interactive forms, and DevTools state
- `peedle`: Native desktop browser window (`winit` + `softbuffer`) and headless runner

## Building and running

```bash
cargo build --release
cargo run --release -- https://example.com
cargo run --release -- --headless --screenshot https://example.com out.png
```
