use peedle_css::{
    compute_styles, get_computed_property, parse_media_query_list, parse_stylesheet,
    query_selector_all, BorderStyle, Color, Display, Float, LengthPercentageAuto, Position,
    Viewport,
};
use peedle_html::parse_html;

#[test]
fn wpt_selectors_level4_conformance() {
    let doc = parse_html(
        r#"<!DOCTYPE html><html><body>
        <main id="app" data-env="prod">
            <ul class="list primary">
                <li data-idx="1" lang="en-US">One</li>
                <li data-idx="2" class="special">Two</li>
                <li data-idx="3" class="special">Three</li>
                <li data-idx="4" lang="fr">Four</li>
            </ul>
            <section class="box"><h2>Title</h2><p></p><p>Body</p></section>
            <input type="checkbox" checked>
            <input type="text" disabled>
        </main>
        </body></html>"#,
    );

    assert_eq!(query_selector_all(&doc, doc.root, "li:first-child").len(), 1);
    assert_eq!(query_selector_all(&doc, doc.root, "li:last-child").len(), 1);
    assert_eq!(query_selector_all(&doc, doc.root, "li:nth-child(2n)").len(), 2);
    assert_eq!(query_selector_all(&doc, doc.root, "li:nth-child(odd)").len(), 2);
    assert_eq!(query_selector_all(&doc, doc.root, "li:nth-last-child(2)").len(), 1);
    assert_eq!(query_selector_all(&doc, doc.root, "li:not(.special)").len(), 2);
    assert_eq!(query_selector_all(&doc, doc.root, ":is(h2, p)").len(), 3);
    assert_eq!(query_selector_all(&doc, doc.root, ":where(ul > li.special)").len(), 2);
    assert_eq!(query_selector_all(&doc, doc.root, "section:has(> h2)").len(), 1);
    assert_eq!(query_selector_all(&doc, doc.root, "p:empty").len(), 1);
    assert_eq!(query_selector_all(&doc, doc.root, "[data-env='prod']").len(), 1);
    assert_eq!(query_selector_all(&doc, doc.root, "[class~='primary']").len(), 1);
    assert_eq!(query_selector_all(&doc, doc.root, "[lang|='en']").len(), 1);
    assert_eq!(query_selector_all(&doc, doc.root, "[data-idx^='2']").len(), 1);
    assert_eq!(query_selector_all(&doc, doc.root, "[data-idx$='4']").len(), 1);
    assert_eq!(query_selector_all(&doc, doc.root, "input:checked").len(), 1);
    assert_eq!(query_selector_all(&doc, doc.root, "input:disabled").len(), 1);
    assert_eq!(query_selector_all(&doc, doc.root, "input:enabled").len(), 1);
    assert_eq!(query_selector_all(&doc, doc.root, "h2 + p").len(), 1);
    assert_eq!(query_selector_all(&doc, doc.root, "h2 ~ p").len(), 2);
}

#[test]
fn wpt_css_cascade_colors_calc_and_error_recovery() {
    let doc = parse_html(
        r#"<!DOCTYPE html><html><body><div id="a" class="x"><span id="b">hi</span></div></body></html>"#,
    );
    let css = r#"
        @layer base, theme;
        @layer theme { #a { color: #00aa00; } }
        @layer base { #a { color: #ff0000 !important; } }
        #a {
            --gap: 12px;
            width: calc(100px + var(--gap) * 2);
            height: clamp(20px, 50px, 80px);
            border: 4px blue;
            background: yellow;
            background: red pink;
            width: 999;
        };
        #a { height: 500px; }
        #a { border: 10px solid red ! bogus; }
        #b {
            color: color-mix(in srgb, red 50%, blue);
            background-color: light-dark(#eee, #222);
            white-space: pre-wrap;
            float: right;
            position: sticky;
        }
    "#;
    let sheet = parse_stylesheet(css);
    let styles = compute_styles(
        &doc,
        &[sheet],
        Viewport {
            width: 1024.0,
            height: 768.0,
            dark_mode: true,
        },
    );

    let a_id = doc.get_element_by_id("a").unwrap();
    let a = styles.get(a_id).unwrap();
    assert_eq!(a.color, Color::rgb(255, 0, 0));
    assert_eq!(a.border_top_style, BorderStyle::None);
    assert_eq!(a.border_top_width, 0.0);
    assert_eq!(a.background_color, Color::rgb(255, 255, 0));
    assert_eq!(a.height, LengthPercentageAuto::px(50.0));
    assert_eq!(get_computed_property(a, "display"), "block");

    let b_id = doc.get_element_by_id("b").unwrap();
    let b = styles.get(b_id).unwrap();
    assert_eq!(b.background_color, Color::rgb(0x22, 0x22, 0x22));
    assert_eq!(get_computed_property(b, "white-space"), "pre-wrap");
    assert_eq!(b.float, Float::Right);
    assert_eq!(b.position, Position::Sticky);
    assert_eq!(b.display, Display::Block);
}

#[test]
fn wpt_media_queries_range_and_color_scheme() {
    let mq1 = parse_media_query_list("(width >= 800px) and (prefers-color-scheme: dark)");
    let dark_wide = Viewport {
        width: 1024.0,
        height: 768.0,
        dark_mode: true,
    };
    let light_narrow = Viewport {
        width: 480.0,
        height: 800.0,
        dark_mode: false,
    };
    assert!(mq1.matches(&dark_wide));
    assert!(!mq1.matches(&light_narrow));
}

#[test]
fn wpt_css_logical_properties_and_mask_image() {
    let doc = parse_html(r#"<div id="box"></div>"#);
    let sheet = parse_stylesheet(
        r#"
        #box {
            inline-size: 320px;
            block-size: 120px;
            margin-inline: 12px 24px;
            margin-block: 8px;
            padding-inline-start: 16px;
            padding-block-end: 10px;
            inset-inline-start: 5px;
            border-inline-start: 3px solid #2563eb;
            mask-image: url("icon.svg");
        }
        "#,
    );
    let styles = compute_styles(
        &doc,
        &[sheet],
        Viewport {
            width: 800.0,
            height: 600.0,
            dark_mode: false,
        },
    );
    let s = styles.get(doc.get_element_by_id("box").unwrap()).unwrap();
    assert_eq!(s.width, LengthPercentageAuto::px(320.0));
    assert_eq!(s.height, LengthPercentageAuto::px(120.0));
    assert_eq!(s.margin_left, LengthPercentageAuto::px(12.0));
    assert_eq!(s.margin_right, LengthPercentageAuto::px(24.0));
    assert_eq!(s.margin_top, LengthPercentageAuto::px(8.0));
    assert_eq!(s.margin_bottom, LengthPercentageAuto::px(8.0));
    assert_eq!(s.padding_left, LengthPercentageAuto::px(16.0));
    assert_eq!(s.padding_bottom, LengthPercentageAuto::px(10.0));
    assert_eq!(s.left, LengthPercentageAuto::px(5.0));
    assert_eq!(s.border_left_width, 3.0);
    assert_eq!(s.border_left_color, Color::rgb(37, 99, 235));
    assert_eq!(s.mask_image.as_deref(), Some("icon.svg"));
}

#[test]
fn wpt_css_nesting_and_nested_media_and_forced_colors() {
    let doc = parse_html(r#"<div class="card"><span class="icon">x</span></div>"#);
    let sheet = parse_stylesheet(
        r#"
        .card {
            color: #112233;
            & .icon {
                color: #224466;
                @media (forced-colors: active) {
                    color: #ff0000;
                }
                @media (forced-colors: none) and (prefers-reduced-motion: no-preference) {
                    background-color: #ddeeff;
                }
                @media (any-hover: hover) and (any-pointer: fine) {
                    padding-left: 8px;
                }
            }
        }
        "#,
    );
    let styles = compute_styles(
        &doc,
        &[sheet],
        Viewport {
            width: 1280.0,
            height: 800.0,
            dark_mode: false,
        },
    );
    let icon_id = query_selector_all(&doc, doc.root, ".icon")[0];
    let s = styles.get(icon_id).unwrap();
    assert_eq!(s.color, Color::rgb(0x22, 0x44, 0x66));
    assert_eq!(s.background_color, Color::rgb(0xdd, 0xee, 0xff));
    assert_eq!(s.padding_left, LengthPercentageAuto::px(8.0));
}

#[test]
fn wpt_css_rule_index_and_cow_custom_properties_scale() {
    let mut html = String::from("<!DOCTYPE html><html><body><div id='root'>");
    for i in 0..400 {
        html.push_str(&format!("<div class='item c{}' id='n{}'><span>{}</span></div>", i % 20, i, i));
    }
    html.push_str("</div></body></html>");

    let mut css = String::from(":root {\n");
    for i in 0..500 {
        css.push_str(&format!("  --tok-{}: {}px;\n", i, (i % 40) + 1));
    }
    css.push_str("  --accent: #3b82f6;\n  --accent-alias: var(--accent);\n}\n");
    for i in 0..2000 {
        css.push_str(&format!(".unused-rule-{} {{ margin-top: {}px; }}\n", i, i % 10));
    }
    css.push_str(".c5 { color: var(--accent-alias); padding-left: var(--tok-12); }\n");
    css.push_str("#n25 { background-color: #10b981; }\n");

    let doc = parse_html(&html);
    let sheet = parse_stylesheet(&css);
    let start = std::time::Instant::now();
    let styles = compute_styles(&doc, &[sheet], Viewport::default());
    let elapsed = start.elapsed();

    let n25 = styles.get(doc.get_element_by_id("n25").unwrap()).unwrap();
    assert_eq!(n25.color, Color::rgb(0x3b, 0x82, 0xf6));
    assert_eq!(n25.padding_left, LengthPercentageAuto::px(13.0));
    assert_eq!(n25.background_color, Color::rgb(0x10, 0xb9, 0x81));
    assert!(
        elapsed.as_millis() < 500,
        "indexed cascade + CoW custom props took too long: {:?}",
        elapsed
    );
}

#[test]
fn wpt_css_specificity_is_where_not_has_and_layers() {
    let doc = parse_html(
        r#"<div id="wrap"><p class="lead"><span>hi</span></p><p class="plain">yo</p></div>"#,
    );
    let sheet = parse_stylesheet(
        r#"
        @layer reset, components, overrides;
        @layer overrides {
            :where(#wrap p.lead) { color: #111111; }
        }
        @layer components {
            :is(#wrap, .nope) p:has(> span) { color: #22aa44; }
            p:not(.lead) { color: #4455cc; }
        }
        "#,
    );
    let styles = compute_styles(&doc, &[sheet], Viewport::default());
    let lead = query_selector_all(&doc, doc.root, "p.lead")[0];
    let plain = query_selector_all(&doc, doc.root, "p.plain")[0];
    assert_eq!(styles.get(lead).unwrap().color, Color::rgb(0x11, 0x11, 0x11));
    assert_eq!(styles.get(plain).unwrap().color, Color::rgb(0x44, 0x55, 0xcc));
}

#[test]
fn wpt_clip_rect_and_clip_path_visually_hidden() {
    let doc = parse_html(r#"<a id="skip">Skip</a><span id="sr">SR</span><div id="vis">Vis</div>"#);
    let sheet = parse_stylesheet(
        r#"
        #skip { position: absolute; clip: rect(0 0 0 0); }
        #sr { clip-path: inset(50%); }
        #vis { clip: auto; }
        "#,
    );
    let styles = compute_styles(&doc, &[sheet], Viewport::default());
    assert!(styles.get(doc.get_element_by_id("skip").unwrap()).unwrap().clip_hidden);
    assert!(styles.get(doc.get_element_by_id("sr").unwrap()).unwrap().clip_hidden);
    assert!(!styles.get(doc.get_element_by_id("vis").unwrap()).unwrap().clip_hidden);
}
