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
