use std::collections::HashMap;

use peedle_css::{compute_styles, parse_stylesheet, Viewport};
use peedle_html::parse_html;
use peedle_layout::layout_document;

fn layout_html(html: &str, css: &str, w: f32, h: f32) -> (peedle_html::Document, peedle_layout::LayoutTree) {
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
fn block_centering_and_box_sizing() {
    let (doc, tree) = layout_html(
        r#"<div id="a">hello</div><div id="b">world</div>"#,
        r#"
            body { margin: 0; }
            #a { width: 200px; height: 50px; margin: 0 auto; padding: 10px; box-sizing: border-box; }
            #b { width: 100px; height: 40px; padding: 10px; box-sizing: content-box; }
        "#,
        800.0,
        600.0,
    );
    let a = tree.bounding_rect(doc.get_element_by_id("a").unwrap()).unwrap();
    assert!((a.width - 200.0).abs() < 1.0, "got {}", a.width);
    assert!((a.height - 50.0).abs() < 1.0, "got {}", a.height);
    assert!((a.x - 300.0).abs() < 1.0, "got {}", a.x);

    let b = tree.bounding_rect(doc.get_element_by_id("b").unwrap()).unwrap();
    assert!((b.width - 120.0).abs() < 1.0, "got {}", b.width);
    assert!((b.height - 60.0).abs() < 1.0, "got {}", b.height);
    assert!((b.y - 50.0).abs() < 1.0, "got {}", b.y);
}

#[test]
fn flexbox_row_grow_gap_and_justify() {
    let (doc, tree) = layout_html(
        r#"<div id="row"><div id="c1">A</div><div id="c2">B</div></div>"#,
        r#"
            body { margin: 0; }
            #row { display: flex; width: 420px; gap: 20px; }
            #c1 { flex: 1; height: 40px; }
            #c2 { flex: 3; height: 60px; }
        "#,
        800.0,
        600.0,
    );
    let c1 = tree.bounding_rect(doc.get_element_by_id("c1").unwrap()).unwrap();
    let c2 = tree.bounding_rect(doc.get_element_by_id("c2").unwrap()).unwrap();
    assert!((c1.width - 100.0).abs() < 2.0, "c1.width={}", c1.width);
    assert!((c2.width - 300.0).abs() < 2.0, "c2.width={}", c2.width);
    assert!((c2.x - 120.0).abs() < 2.0, "c2.x={}", c2.x);
}

#[test]
fn css_grid_tracks_and_span() {
    let (doc, tree) = layout_html(
        r#"<div id="g"><div id="g1">1</div><div id="g2">2</div><div id="g3">3</div></div>"#,
        r#"
            body { margin: 0; }
            #g { display: grid; width: 320px; grid-template-columns: 100px 1fr 1fr; gap: 10px; }
            #g1 { height: 30px; }
            #g2 { grid-column: span 2; height: 30px; }
            #g3 { height: 50px; }
        "#,
        800.0,
        600.0,
    );
    let g1 = tree.bounding_rect(doc.get_element_by_id("g1").unwrap()).unwrap();
    let g2 = tree.bounding_rect(doc.get_element_by_id("g2").unwrap()).unwrap();
    let g3 = tree.bounding_rect(doc.get_element_by_id("g3").unwrap()).unwrap();
    assert!((g1.width - 100.0).abs() < 1.5, "g1={:?}", g1);
    assert!((g2.width - 210.0).abs() < 1.5, "g2={:?}", g2);
    assert!((g2.x - 110.0).abs() < 1.5, "g2={:?}", g2);
    assert!((g3.y - 40.0).abs() < 1.5, "g3={:?}", g3);
}

#[test]
fn table_and_absolute_positioning() {
    let (doc, tree) = layout_html(
        r#"
        <table id="t" style="width: 300px; border-collapse: collapse;">
            <tr><td id="td1" style="width: 100px; height: 30px;">A</td><td id="td2">B</td></tr>
        </table>
        <div id="rel" style="position: relative; width: 200px; height: 100px;">
            <div id="abs" style="position: absolute; right: 10px; bottom: 15px; width: 40px; height: 20px;"></div>
        </div>
        "#,
        "body { margin: 0; }",
        800.0,
        600.0,
    );
    let td1 = tree.bounding_rect(doc.get_element_by_id("td1").unwrap()).unwrap();
    let td2 = tree.bounding_rect(doc.get_element_by_id("td2").unwrap()).unwrap();
    assert!(td2.x > td1.x);

    let rel = tree.bounding_rect(doc.get_element_by_id("rel").unwrap()).unwrap();
    let abs = tree.bounding_rect(doc.get_element_by_id("abs").unwrap()).unwrap();
    assert!((abs.right() - (rel.right() - 10.0)).abs() < 1.5);
    assert!((abs.bottom() - (rel.bottom() - 15.0)).abs() < 1.5);
}
