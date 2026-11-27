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
