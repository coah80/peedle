use peedle_html::parse_html;
use peedle_render::{render_svg_node, Canvas2D, FontFaceKey, FontSystem, Framebuffer, RgbaColor};

#[test]
fn framebuffer_and_canvas2d_and_svg_render() {
    let mut fb = Framebuffer::new(320, 200);
    fb.clear(RgbaColor::WHITE);
    fb.fill_rounded_rect(10.0, 10.0, 120.0, 60.0, [12.0; 4], RgbaColor::new(30, 120, 220, 255));
    fb.fill_linear_gradient(
        150.0,
        10.0,
        150.0,
        60.0,
        [8.0; 4],
        90.0,
        &[
            (RgbaColor::new(255, 80, 80, 255), 0.0),
            (RgbaColor::new(80, 80, 255, 255), 1.0),
        ],
        false,
    );

    let fonts = FontSystem::load_system_fonts();
    let end_x = fonts.draw_text(
        &mut fb,
        "Peedle Render",
        16.0,
        110.0,
        FontFaceKey::SansBold,
        20.0,
        RgbaColor::BLACK,
    );
    assert!(end_x > 80.0);

    let mut canvas = Canvas2D::new(80, 80);
    canvas.state.fill_style = RgbaColor::new(40, 200, 100, 255);
    canvas.begin_path();
    canvas.arc(40.0, 40.0, 30.0, 0.0, std::f32::consts::TAU, false);
    canvas.close_path();
    canvas.fill(false);
    let center = canvas.get_image_data(40, 40, 1, 1);
    assert_eq!(center, vec![40, 200, 100, 255]);

    let doc = parse_html(
        r##"<svg viewBox="0 0 100 100"><rect x="10" y="10" width="80" height="80" fill="#ff0000"/></svg>"##,
    );
    let svg_id = doc.get_elements_by_tag_name(doc.root, "svg")[0];
    render_svg_node(&doc, svg_id, &mut fb, &fonts, 200.0, 100.0, 80.0, 80.0);

    let px_idx = ((140 * 320 + 240) * 4) as usize;
    assert_eq!(fb.data[px_idx], 255);
    assert_eq!(fb.data[px_idx + 1], 0);
    assert_eq!(fb.data[px_idx + 2], 0);

    let png = fb.encode_png().unwrap();
    assert!(png.starts_with(b"\x89PNG"));
}

#[test]
fn acid1_full_pipeline_render() {
    let acid1_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/acid/acid1.html");
    if !acid1_path.exists() {
        return;
    }
    let html = std::fs::read_to_string(&acid1_path).unwrap();
    let doc = parse_html(&html);
    let mut sheets = Vec::new();
    for nid in doc.descendants(doc.root) {
        if let Some(el) = doc.element(nid) {
            if el.is_html() && el.tag_is("style") {
                sheets.push(peedle_css::parse_stylesheet(&doc.text_content(nid)));
            }
        }
    }
    let styles = peedle_css::compute_styles(
        &doc,
        &sheets,
        peedle_css::Viewport {
            width: 800.0,
            height: 600.0,
            dark_mode: false,
        },
    );
    let tree = peedle_layout::layout_document(&doc, &styles, &std::collections::HashMap::new(), 800.0, 600.0);
    let fonts = FontSystem::load_system_fonts();
    let images = peedle_render::ImageCache::new();
    let canvases = std::collections::HashMap::new();
    let mut fb = Framebuffer::new(800, 600);
    peedle_render::paint_layout(&doc, &tree, &mut fb, &fonts, &images, &canvases, 0.0, 0.0);
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../screenshots/acid1_v3.png");
    let _ = fb.save_png(&out);

    let bg_px = ((500 * 800 + 700) * 4) as usize;
    assert_eq!((fb.data[bg_px], fb.data[bg_px + 1], fb.data[bg_px + 2]), (0, 0, 255));
    let dt_px = ((100 * 800 + 60) * 4) as usize;
    assert_eq!((fb.data[dt_px], fb.data[dt_px + 1], fb.data[dt_px + 2]), (204, 0, 0));
}

#[test]
fn acid2_full_pipeline_render() {
    let acid2_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/acid/acid2.html");
    if !acid2_path.exists() {
        return;
    }
    let html = std::fs::read_to_string(&acid2_path).unwrap();
    let doc = parse_html(&html);
    let client = peedle_net::HttpClient::new();
    let mut sheets = Vec::new();
    let mut images = peedle_render::ImageCache::new();
    let mut image_sizes = std::collections::HashMap::new();
    for nid in doc.descendants(doc.root) {
        if let Some(el) = doc.element(nid) {
            if el.is_html() && el.tag_is("style") {
                sheets.push(peedle_css::parse_stylesheet(&doc.text_content(nid)));
            } else if el.is_html() && el.tag_is("link") {
                if let (Some(rel), Some(href)) = (el.get_attr("rel"), el.get_attr("href")) {
                    if rel.to_ascii_lowercase().contains("stylesheet") && href.starts_with("data:") {
                        if let Ok(resp) = client.get(href) {
                            sheets.push(peedle_css::parse_stylesheet(&String::from_utf8_lossy(&resp.body)));
                        }
                    }
                }
            } else if el.is_html() && (el.tag_is("img") || el.tag_is("object")) {
                let u = el.get_attr("src").or_else(|| el.get_attr("data"));
                if let Some(u) = u {
                    if u.starts_with("data:image/") {
                        if let Ok(resp) = client.get(u) {
                            if let Ok(decoded) = peedle_render::DecodedImage::decode(&resp.body) {
                                image_sizes.insert(u.to_string(), (decoded.width, decoded.height));
                                images.insert(u.to_string(), decoded);
                            }
                        }
                    }
                }
            }
        }
    }
    let styles = peedle_css::compute_styles(
        &doc,
        &sheets,
        peedle_css::Viewport {
            width: 800.0,
            height: 700.0,
            dark_mode: false,
        },
    );
    for st in styles.styles.values() {
        if let peedle_css::BackgroundImage::Url(ref u) = st.background_image {
            if u.starts_with("data:image/") && images.get(u).is_none() {
                if let Ok(resp) = client.get(u) {
                    if let Ok(decoded) = peedle_render::DecodedImage::decode(&resp.body) {
                        images.insert(u.clone(), decoded);
                    }
                }
            }
        }
    }
    let tree = peedle_layout::layout_document(&doc, &styles, &image_sizes, 800.0, 700.0);
    let scroll_y = doc
        .get_element_by_id("top")
        .and_then(|id| tree.bounding_rect(id))
        .map(|r| r.y)
        .unwrap_or(0.0);

    let box_of_sel = |sel: &str| {
        let nid = peedle_css::query_selector(&doc, doc.root, sel).unwrap();
        tree.box_of(nid).unwrap().dims.border_box()
    };
    let forehead = box_of_sel(".forehead");
    let eyes = box_of_sel(".eyes");
    let nose = box_of_sel(".nose");
    let chin = box_of_sel(".chin");
    let parser = box_of_sel(".parser");
    let ul = box_of_sel("ul");
    assert!((forehead.height - 12.0).abs() < 0.5);
    assert!((eyes.y - forehead.bottom()).abs() < 0.5);
    assert!((nose.y - eyes.bottom()).abs() < 0.5);
    assert!((chin.y - (nose.bottom() + 24.0)).abs() < 0.5);
    assert!((parser.y - chin.bottom()).abs() < 0.5);
    assert!((ul.y - parser.bottom()).abs() < 0.5);
    assert!((ul.width - 48.0).abs() < 0.5);

    let fonts = FontSystem::load_system_fonts();
    let canvases = std::collections::HashMap::new();
    let mut fb = Framebuffer::new(800, 700);
    peedle_render::paint_layout(&doc, &tree, &mut fb, &fonts, &images, &canvases, 0.0, scroll_y);
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../screenshots/acid2_final.png");
    let _ = fb.save_png(&out);

    let mut red_pixels = 0usize;
    let mut yellow_pixels = 0usize;
    let mut black_pixels = 0usize;
    for chunk in fb.data.chunks_exact(4) {
        if chunk[0] > 200 && chunk[1] < 30 && chunk[2] < 30 {
            red_pixels += 1;
        } else if chunk[0] > 200 && chunk[1] > 200 && chunk[2] < 30 {
            yellow_pixels += 1;
        } else if chunk[0] < 30 && chunk[1] < 30 && chunk[2] < 30 {
            black_pixels += 1;
        }
    }
    assert_eq!(red_pixels, 0, "Acid2 must have zero red failure pixels");
    assert!(yellow_pixels > 10000);
    assert!(black_pixels > 5000);
}

#[test]
fn transform_sticky_outline_and_svg_arcs_render() {
    let html = r##"
        <style>
            body { margin: 0; height: 1000px; }
            #centered {
                position: fixed;
                left: 50%;
                top: 50%;
                width: 100px;
                height: 60px;
                transform: translate(-50%, -50%);
                background: #22c55e;
                outline: 4px solid #ef4444;
                outline-offset: 2px;
            }
            #bar {
                position: sticky;
                top: 10px;
                height: 20px;
                background: #2563eb;
            }
        </style>
        <div id="bar"></div>
        <div id="centered"></div>
    "##;
    let doc = parse_html(html);
    let mut sheets = Vec::new();
    for nid in doc.descendants(doc.root) {
        if let Some(el) = doc.element(nid) {
            if el.tag_is("style") {
                sheets.push(peedle_css::parse_stylesheet(&doc.text_content(nid)));
            }
        }
    }
    let styles = peedle_css::compute_styles(
        &doc,
        &sheets,
        peedle_css::Viewport {
            width: 400.0,
            height: 300.0,
            dark_mode: false,
        },
    );
    let tree = peedle_layout::layout_document(&doc, &styles, &std::collections::HashMap::new(), 400.0, 300.0);
    let fonts = FontSystem::load_system_fonts();
    let images = peedle_render::ImageCache::new();
    let canvases = std::collections::HashMap::new();
    let mut fb = Framebuffer::new(400, 300);
    peedle_render::paint_layout(&doc, &tree, &mut fb, &fonts, &images, &canvases, 0.0, 100.0);

    let sticky_px = ((18 * 400 + 20) * 4) as usize;
    assert_eq!((fb.data[sticky_px], fb.data[sticky_px + 1], fb.data[sticky_px + 2]), (37, 99, 235));

    let center_px = ((150 * 400 + 200) * 4) as usize;
    assert_eq!((fb.data[center_px], fb.data[center_px + 1], fb.data[center_px + 2]), (34, 197, 94));

    let outline_px = ((116 * 400 + 200) * 4) as usize;
    assert_eq!((fb.data[outline_px], fb.data[outline_px + 1], fb.data[outline_px + 2]), (239, 68, 68));

    let svg_doc = parse_html(
        r##"<svg viewBox="0 0 100 100">
            <rect x="10" y="10" width="80" height="80" rx="16" style="fill:#10b981"/>
            <path d="M 50 20 A 30 30 0 1 1 49.9 20 Z" fill="#1d4ed8"/>
        </svg>"##,
    );
    let svg_id = svg_doc.get_elements_by_tag_name(svg_doc.root, "svg")[0];
    let mut svg_fb = Framebuffer::new(100, 100);
    svg_fb.clear(RgbaColor::WHITE);
    render_svg_node(&svg_doc, svg_id, &mut svg_fb, &fonts, 0.0, 0.0, 100.0, 100.0);
    let arc_center = ((50 * 100 + 50) * 4) as usize;
    assert_eq!((svg_fb.data[arc_center], svg_fb.data[arc_center + 1], svg_fb.data[arc_center + 2]), (29, 78, 216));
    let rounded_corner = ((11 * 100 + 11) * 4) as usize;
    assert_eq!((svg_fb.data[rounded_corner], svg_fb.data[rounded_corner + 1], svg_fb.data[rounded_corner + 2]), (255, 255, 255));
}

#[test]
fn mask_image_and_rounded_object_fit_render() {
    let html = r##"<!DOCTYPE html>
        <style>
            body { margin: 0; background: #ffffff; }
            #icon {
                width: 40px; height: 40px;
                background-color: #2563eb;
                mask-image: url("mask.svg");
            }
            #avatar {
                display: block;
                width: 40px; height: 40px;
                border-radius: 20px;
                object-fit: cover;
            }
        </style>
        <div id="icon"></div>
        <img id="avatar" src="pic.png" width="40" height="40">
    "##;
    let doc = parse_html(html);
    let mut sheets = Vec::new();
    for nid in doc.descendants(doc.root) {
        if let Some(el) = doc.element(nid) {
            if el.tag_is("style") {
                sheets.push(peedle_css::parse_stylesheet(&doc.text_content(nid)));
            }
        }
    }
    let styles = peedle_css::compute_styles(
        &doc,
        &sheets,
        peedle_css::Viewport {
            width: 200.0,
            height: 200.0,
            dark_mode: false,
        },
    );
    let mut img_sizes = std::collections::HashMap::new();
    img_sizes.insert("pic.png".to_string(), (80, 40));
    let tree = peedle_layout::layout_document(&doc, &styles, &img_sizes, 200.0, 200.0);
    let fonts = FontSystem::load_system_fonts();
    let mut images = peedle_render::ImageCache::new();
    let mask_svg = peedle_render::DecodedImage::decode(
        br##"<svg viewBox="0 0 40 40"><circle cx="20" cy="20" r="12" fill="#000"/></svg>"##,
    )
    .unwrap();
    images.insert("mask.svg".to_string(), mask_svg);
    let mut pic_rgba = vec![0u8; 80 * 40 * 4];
    for px in pic_rgba.chunks_exact_mut(4) {
        px[0] = 220;
        px[1] = 38;
        px[2] = 38;
        px[3] = 255;
    }
    images.insert(
        "pic.png".to_string(),
        peedle_render::DecodedImage {
            width: 80,
            height: 40,
            rgba: pic_rgba,
        },
    );
    let canvases = std::collections::HashMap::new();
    let mut fb = Framebuffer::new(200, 200);
    peedle_render::paint_layout(&doc, &tree, &mut fb, &fonts, &images, &canvases, 0.0, 0.0);

    let icon_center = ((20 * 200 + 20) * 4) as usize;
    assert_eq!((fb.data[icon_center], fb.data[icon_center + 1], fb.data[icon_center + 2]), (37, 99, 235));
    let icon_corner = ((2 * 200 + 2) * 4) as usize;
    assert_eq!((fb.data[icon_corner], fb.data[icon_corner + 1], fb.data[icon_corner + 2]), (255, 255, 255));

    let av_center = ((60 * 200 + 20) * 4) as usize;
    assert_eq!((fb.data[av_center], fb.data[av_center + 1], fb.data[av_center + 2]), (220, 38, 38));
    let av_corner = ((41 * 200 + 1) * 4) as usize;
    assert_eq!((fb.data[av_corner], fb.data[av_corner + 1], fb.data[av_corner + 2]), (255, 255, 255));
}

#[test]
fn svg_current_color_use_symbol_and_evenodd_render() {
    let html = r##"<!DOCTYPE html>
        <style>
            body { margin: 0; background: #000000; color: #38bdf8; }
            svg { display: block; }
        </style>
        <svg width="80" height="80" viewBox="0 0 80 80">
            <defs>
                <symbol id="ring">
                    <path fill="currentColor" fill-rule="evenodd" d="M 10 10 L 70 10 L 70 70 L 10 70 Z M 30 30 L 50 30 L 50 50 L 30 50 Z"/>
                </symbol>
            </defs>
            <use href="#ring"/>
        </svg>
    "##;
    let doc = parse_html(html);
    let mut sheets = Vec::new();
    for nid in doc.descendants(doc.root) {
        if let Some(el) = doc.element(nid) {
            if el.tag_is("style") {
                sheets.push(peedle_css::parse_stylesheet(&doc.text_content(nid)));
            }
        }
    }
    let styles = peedle_css::compute_styles(
        &doc,
        &sheets,
        peedle_css::Viewport {
            width: 100.0,
            height: 100.0,
            dark_mode: false,
        },
    );
    let tree = peedle_layout::layout_document(&doc, &styles, &std::collections::HashMap::new(), 100.0, 100.0);
    let fonts = FontSystem::load_system_fonts();
    let images = peedle_render::ImageCache::new();
    let canvases = std::collections::HashMap::new();
    let mut fb = Framebuffer::new(100, 100);
    peedle_render::paint_layout(&doc, &tree, &mut fb, &fonts, &images, &canvases, 0.0, 0.0);

    let ring_px = ((20 * 100 + 20) * 4) as usize;
    assert_eq!((fb.data[ring_px], fb.data[ring_px + 1], fb.data[ring_px + 2]), (56, 189, 248));
    let hole_px = ((40 * 100 + 40) * 4) as usize;
    assert_eq!((fb.data[hole_px], fb.data[hole_px + 1], fb.data[hole_px + 2]), (0, 0, 0));
}
