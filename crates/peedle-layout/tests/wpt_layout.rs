use std::collections::HashMap;

use peedle_css::{compute_styles, parse_stylesheet, Viewport};
use peedle_html::parse_html;
use peedle_layout::layout_document;

fn run_layout(html: &str, css: &str, w: f32, h: f32) -> (peedle_html::Document, peedle_layout::LayoutTree) {
    let doc = parse_html(html);
    let sheet = parse_stylesheet(css);
    let styles = compute_styles(
        &doc,
        &[sheet],
        Viewport {
            width: w,
            height: h,
            dark_mode: false,
        },
    );
    let tree = layout_document(&doc, &styles, &HashMap::new(), w, h);
    (doc, tree)
}

#[test]
fn wpt_margin_collapsing_and_negative_margins() {
    let (doc, tree) = run_layout(
        r#"<div id="a">A</div><div id="b">B</div><div id="c">C</div>"#,
        r#"
            body { margin: 0; }
            #a { height: 40px; margin-bottom: 30px; }
            #b { height: 40px; margin-top: 20px; margin-bottom: -10px; }
            #c { height: 40px; margin-top: -15px; }
        "#,
        800.0,
        600.0,
    );
    let a = tree.bounding_rect(doc.get_element_by_id("a").unwrap()).unwrap();
    let b = tree.bounding_rect(doc.get_element_by_id("b").unwrap()).unwrap();
    let c = tree.bounding_rect(doc.get_element_by_id("c").unwrap()).unwrap();
    assert!((b.y - (a.bottom() + 30.0)).abs() < 1.0, "a={:?} b={:?}", a, b);
    assert!((c.y - (b.bottom() - 15.0)).abs() < 1.0, "b={:?} c={:?}", b, c);
}

#[test]
fn wpt_min_height_overrides_max_height_and_auto_pct() {
    let (doc, tree) = run_layout(
        r#"<div id="wrap"><div id="pct"></div><div id="minmax"></div></div>"#,
        r#"
            body { margin: 0; }
            #wrap { width: 400px; }
            #pct { height: 50%; }
            #minmax { height: 8px; min-height: 30px; max-height: 10px; }
        "#,
        800.0,
        600.0,
    );
    let pct = tree.bounding_rect(doc.get_element_by_id("pct").unwrap()).unwrap();
    let mm = tree.bounding_rect(doc.get_element_by_id("minmax").unwrap()).unwrap();
    assert!(pct.height < 1.0, "percentage height in auto parent must compute to auto: {:?}", pct);
    assert!((mm.height - 30.0).abs() < 1.0, "min-height must override max-height: {:?}", mm);
}

#[test]
fn wpt_flexbox_wrap_align_and_column() {
    let (doc, tree) = run_layout(
        r#"<div id="f"><div id="i1">1</div><div id="i2">2</div><div id="i3">3</div></div>"#,
        r#"
            body { margin: 0; }
            #f { display: flex; flex-wrap: wrap; width: 220px; gap: 10px; }
            #i1, #i2, #i3 { width: 100px; height: 40px; }
        "#,
        800.0,
        600.0,
    );
    let i1 = tree.bounding_rect(doc.get_element_by_id("i1").unwrap()).unwrap();
    let i2 = tree.bounding_rect(doc.get_element_by_id("i2").unwrap()).unwrap();
    let i3 = tree.bounding_rect(doc.get_element_by_id("i3").unwrap()).unwrap();
    assert!((i1.y - i2.y).abs() < 1.0, "i1 and i2 on row 1");
    assert!(i3.y >= i1.bottom() + 9.0, "i3 wraps to row 2: i1={:?} i3={:?}", i1, i3);
}

#[test]
fn wpt_float_wrapping_and_clear() {
    let (doc, tree) = run_layout(
        r#"<div id="box">
            <div id="f1">1</div>
            <div id="f2">2</div>
            <div id="f3">3</div>
            <div id="clr">cleared</div>
        </div>"#,
        r#"
            body { margin: 0; }
            #box { width: 200px; }
            #f1, #f2, #f3 { float: left; width: 90px; height: 50px; }
            #clr { clear: both; height: 20px; }
        "#,
        800.0,
        600.0,
    );
    let f1 = tree.bounding_rect(doc.get_element_by_id("f1").unwrap()).unwrap();
    let f2 = tree.bounding_rect(doc.get_element_by_id("f2").unwrap()).unwrap();
    let f3 = tree.bounding_rect(doc.get_element_by_id("f3").unwrap()).unwrap();
    let clr = tree.bounding_rect(doc.get_element_by_id("clr").unwrap()).unwrap();
    assert!((f1.x - 0.0).abs() < 1.0 && (f1.y - 0.0).abs() < 1.0, "f1={:?}", f1);
    assert!((f2.x - 90.0).abs() < 1.0);
    assert!((f3.x - 0.0).abs() < 1.0 && (f3.y - 50.0).abs() < 1.0, "f3={:?}", f3);
    assert!(clr.y >= 100.0 - 0.5, "clr={:?}", clr);
}

#[test]
fn wpt_css_grid_repeat_minmax_and_rows() {
    let (doc, tree) = run_layout(
        r#"<div id="grid">
            <div id="a">A</div><div id="b">B</div>
            <div id="c">C</div><div id="d">D</div>
        </div>"#,
        r#"
            body { margin: 0; }
            #grid { display: grid; width: 420px; grid-template-columns: repeat(2, 1fr); gap: 20px; }
            #a, #b, #c, #d { height: 60px; }
        "#,
        800.0,
        600.0,
    );
    let a = tree.bounding_rect(doc.get_element_by_id("a").unwrap()).unwrap();
    let b = tree.bounding_rect(doc.get_element_by_id("b").unwrap()).unwrap();
    let c = tree.bounding_rect(doc.get_element_by_id("c").unwrap()).unwrap();
    let d = tree.bounding_rect(doc.get_element_by_id("d").unwrap()).unwrap();
    assert!((a.width - 200.0).abs() < 1.5);
    assert!((b.x - 220.0).abs() < 1.5);
    assert!((c.y - 80.0).abs() < 1.5);
    assert!((d.x - 220.0).abs() < 1.5 && (d.y - 80.0).abs() < 1.5);
}
