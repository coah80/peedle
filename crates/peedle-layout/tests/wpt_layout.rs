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

#[test]
fn wpt_text_overflow_ellipsis_and_word_break() {
    let (doc, tree) = run_layout(
        r#"
            <div id="ell">Supercalifragilisticexpialidocious overflowing headline text</div>
            <div id="brk">aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa</div>
        "#,
        r#"
            body { margin: 0; font-size: 14px; }
            #ell { width: 100px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
            #brk { width: 80px; word-break: break-all; }
        "#,
        400.0,
        300.0,
    );
    let ell_id = doc.get_element_by_id("ell").unwrap();
    let ell_box = tree.box_of(ell_id).unwrap();
    let mut joined = String::new();
    for &cid in &ell_box.children {
        for f in &tree.get(cid).text_fragments {
            joined.push_str(&f.text);
        }
    }
    assert!(joined.ends_with("..."), "expected ellipsis, got {:?}", joined);

    let brk_rect = tree.bounding_rect(doc.get_element_by_id("brk").unwrap()).unwrap();
    assert!(brk_rect.height > 28.0, "expected word-break wrapping, got {:?}", brk_rect);
}

#[test]
fn wpt_css_grid_template_areas_and_shorthand() {
    let (doc, tree) = run_layout(
        r#"<div id="page">
            <div id="main">Main content</div>
            <div id="nav">Sidebar</div>
            <div id="foot">Footer</div>
        </div>"#,
        r#"
            body { margin: 0; }
            #page {
                display: grid;
                width: 820px;
                column-gap: 20px;
                grid-template: auto 1fr / 200px minmax(0, 1fr);
                grid-template-areas: 'sidebar content' 'footer footer';
            }
            #nav { grid-area: sidebar; height: 100px; }
            #main { grid-area: content; height: 240px; }
            #foot { grid-area: footer; height: 40px; }
        "#,
        900.0,
        700.0,
    );
    let nav = tree.bounding_rect(doc.get_element_by_id("nav").unwrap()).unwrap();
    let main = tree.bounding_rect(doc.get_element_by_id("main").unwrap()).unwrap();
    let foot = tree.bounding_rect(doc.get_element_by_id("foot").unwrap()).unwrap();
    assert!((nav.x - 0.0).abs() < 1.5 && (nav.width - 200.0).abs() < 1.5, "nav={:?}", nav);
    assert!((main.x - 220.0).abs() < 1.5 && (main.width - 600.0).abs() < 1.5, "main={:?}", main);
    assert!((foot.y - 240.0).abs() < 1.5 && (foot.width - 820.0).abs() < 1.5, "foot={:?}", foot);
}

#[test]
fn wpt_flex_column_bfc_float_isolation_and_indefinite_pct() {
    let (doc, tree) = run_layout(
        r#"<div id="col">
            <div id="hdr"><a id="logo" href="/">Logo</a></div>
            <main id="body">
                <div id="row1"><div class="flt">V</div><div class="txt">First</div></div>
                <div id="row2"><div class="flt">V</div><div class="txt">Second</div></div>
            </main>
        </div>"#,
        r#"
            body { margin: 0; }
            #col { display: flex; flex-direction: column; width: 600px; }
            #hdr { display: flex; align-items: center; }
            #logo { display: block; height: 100%; }
            #row1, #row2 { clear: both; }
            .flt { float: left; width: 40px; height: 30px; }
        "#,
        800.0,
        800.0,
    );
    let logo = tree.bounding_rect(doc.get_element_by_id("logo").unwrap()).unwrap();
    let row1 = tree.bounding_rect(doc.get_element_by_id("row1").unwrap()).unwrap();
    let row2 = tree.bounding_rect(doc.get_element_by_id("row2").unwrap()).unwrap();
    assert!(logo.height < 60.0, "height:100% in auto flex must not explode to viewport: {:?}", logo);
    assert!(row1.y < 80.0, "first cleared row must not be pushed down by pass-1 floats: {:?}", row1);
    assert!(row2.y >= row1.y + 29.0 && row2.y < 140.0, "row2={:?}", row2);
}

#[test]
fn wpt_flex_intrinsic_ignores_absolute_dropdown() {
    let (doc, tree) = run_layout(
        r#"<header id="bar">
            <h1 id="title">Rust (programming language)</h1>
            <div id="menu">
                <span>60 languages</span>
                <div id="popup">aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa</div>
            </div>
        </header>"#,
        r#"
            body { margin: 0; }
            #bar { display: flex; width: 700px; }
            #title { flex-grow: 1; margin: 0; }
            #menu { position: relative; flex-shrink: 0; }
            #popup { position: absolute; width: 650px; }
        "#,
        800.0,
        600.0,
    );
    let title = tree.bounding_rect(doc.get_element_by_id("title").unwrap()).unwrap();
    assert!(title.width > 400.0, "absolute popup must not shrink flex sibling: {:?}", title);
}

#[test]
fn wpt_table_respects_min_word_width() {
    let (doc, tree) = run_layout(
        r#"<table id="t">
            <tr><th id="c1">Implementation</th><td id="c2">A very long description that wraps across multiple lines inside the second column of this infobox table</td></tr>
        </table>"#,
        r#"
            body { margin: 0; font-size: 14px; }
            #t { width: 260px; border-collapse: collapse; }
            th, td { padding: 4px; }
        "#,
        400.0,
        400.0,
    );
    let c1 = tree.bounding_rect(doc.get_element_by_id("c1").unwrap()).unwrap();
    let c2 = tree.bounding_rect(doc.get_element_by_id("c2").unwrap()).unwrap();
    assert!(c1.width >= 95.0, "column 1 must preserve unbroken word width: {:?}", c1);
    assert!(c2.x >= c1.right() - 0.5, "columns must not overlap: c1={:?} c2={:?}", c1, c2);
}

#[test]
fn wpt_ellipsis_fits_and_inline_flex_shrinks_to_fit() {
    let (doc, tree) = run_layout(
        r#"
            <div id="fit">crates</div>
            <button id="btn"><span>Star</span><span>0</span></button>
        "#,
        r#"
            body { margin: 0; font-size: 14px; }
            #fit { display: inline-block; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
            #btn { display: inline-flex; gap: 6px; padding: 4px 10px; }
        "#,
        800.0,
        400.0,
    );
    let fit_box = tree.box_of(doc.get_element_by_id("fit").unwrap()).unwrap();
    let mut joined = String::new();
    for &cid in &fit_box.children {
        for f in &tree.get(cid).text_fragments {
            joined.push_str(&f.text);
        }
    }
    assert_eq!(joined, "crates", "fitting text must not be elided");

    let btn = tree.bounding_rect(doc.get_element_by_id("btn").unwrap()).unwrap();
    assert!(btn.width < 150.0, "inline-flex must shrink-to-fit instead of stretching: {:?}", btn);
}
