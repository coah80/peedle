use peedle_css::*;
use peedle_html::{Document, NodeId};

fn build_sample_doc() -> (Document, NodeId, NodeId, NodeId, NodeId, NodeId, NodeId) {
    let mut doc = Document::new();
    doc.url = "https://example.com/page#target-sec".to_string();
    let html = doc.create_element("html");
    doc.set_attribute(html, "lang", "en-US");
    doc.append_child(doc.root, html);

    let head = doc.create_element("head");
    let style_el = doc.create_element("style");
    doc.append_child(head, style_el);
    doc.append_child(html, head);

    let body = doc.create_element("body");
    doc.append_child(html, body);

    let main_div = doc.create_element("div");
    doc.set_attribute(main_div, "id", "target-sec");
    doc.set_attribute(main_div, "class", "card primary");
    doc.set_attribute(main_div, "data-tags", "alpha beta gamma");
    doc.set_attribute(main_div, "data-code", "US-West");
    doc.append_child(body, main_div);

    let h1 = doc.create_element("h1");
    let h1_txt = doc.create_text("Hello");
    doc.append_child(h1, h1_txt);
    doc.append_child(main_div, h1);

    let p = doc.create_element("p");
    doc.set_attribute(p, "class", "lead");
    let p_txt = doc.create_text("Body text");
    doc.append_child(p, p_txt);
    doc.append_child(main_div, p);

    let empty_span = doc.create_element("span");
    doc.append_child(main_div, empty_span);

    (doc, html, head, body, main_div, h1, p)
}

#[test]
fn tokenizer_full_css3_tokens() {
    let css = "/* skipped */ @media screen { #hero .btn\\31 :hover { width: 12.5rem; height: 50%; content: \"hi\\41 \"; background: url(https://x.test/a.png); unicode-range: U+0041-005A, U+04??; } } <!-- -->";
    let tokens = tokenize(css);

    assert!(tokens.iter().any(|t| *t == Token::AtKeyword("media".into())));
    assert!(tokens.iter().any(|t| *t == Token::Hash {
        value: "hero".into(),
        kind: HashKind::Id
    }));
    assert!(tokens.iter().any(|t| *t == Token::Ident("btn1".into())));
    assert!(tokens.iter().any(|t| matches!(
        t,
        Token::Dimension { value, unit, .. } if (*value - 12.5).abs() < 1e-4 && unit == "rem"
    )));
    assert!(tokens.iter().any(|t| matches!(
        t,
        Token::Percentage { value, .. } if (*value - 50.0).abs() < 1e-4
    )));
    assert!(tokens.iter().any(|t| *t == Token::String("hiA".into())));
    assert!(tokens
        .iter()
        .any(|t| *t == Token::Url("https://x.test/a.png".into())));
    assert!(tokens.iter().any(|t| *t == Token::UnicodeRange {
        start: 0x0041,
        end: 0x005A
    }));
    assert!(tokens.iter().any(|t| *t == Token::UnicodeRange {
        start: 0x0400,
        end: 0x04FF
    }));
    assert!(tokens.iter().any(|t| *t == Token::Cdo));
    assert!(tokens.iter().any(|t| *t == Token::Cdc));

    let hash_unrestricted = tokenize("#123abc");
    assert_eq!(
        hash_unrestricted[0],
        Token::Hash {
            value: "123abc".into(),
            kind: HashKind::Unrestricted
        }
    );
}

#[test]
fn selectors_level4_matching_and_queries() {
    let (mut doc, html, _head, body, main_div, h1, p) = build_sample_doc();

    let empty_span = doc.element_children(main_div)[2];
    let link = doc.create_element("a");
    doc.set_attribute(link, "href", "https://example.com");
    doc.append_child(body, link);

    let input = doc.create_element("input");
    doc.set_attribute(input, "type", "checkbox");
    doc.set_attribute(input, "checked", "");
    doc.set_attribute(input, "required", "");
    doc.append_child(body, input);

    let text_in = doc.create_element("input");
    doc.set_attribute(text_in, "placeholder", "Enter name");
    doc.set_attribute(text_in, "disabled", "");
    doc.append_child(body, text_in);

    assert!(matches_selector(&doc, html, ":root"));
    assert!(!matches_selector(&doc, body, ":root"));
    assert!(matches_selector(&doc, empty_span, "span:empty"));
    assert!(!matches_selector(&doc, p, "p:empty"));
    assert!(matches_selector(&doc, h1, "div > h1:first-child"));
    assert!(matches_selector(&doc, p, "h1 + p.lead"));
    assert!(matches_selector(&doc, empty_span, "h1 ~ span:last-child"));
    assert!(matches_selector(&doc, main_div, "[data-tags~=\"beta\"]"));
    assert!(matches_selector(&doc, main_div, "[data-code|=\"us\" i]"));
    assert!(!matches_selector(&doc, main_div, "[data-code|=\"us\" s]"));
    assert!(matches_selector(&doc, main_div, "[data-code^=\"US\"]"));
    assert!(matches_selector(&doc, main_div, "[data-code$=\"West\"]"));
    assert!(matches_selector(&doc, main_div, "[data-code*=\"-W\"]"));
    assert!(matches_selector(&doc, main_div, ":target"));
    assert!(matches_selector(&doc, p, ":lang(en)"));
    assert!(matches_selector(&doc, main_div, "div:has(> h1 + p.lead)"));
    assert!(matches_selector(&doc, body, "body:has(p.lead)"));
    assert!(!matches_selector(&doc, main_div, "div:has(> footer)"));
    assert!(matches_selector(&doc, p, ":is(article, div) > :where(p, span).lead"));
    assert!(matches_selector(&doc, h1, ":not(p)"));
    assert!(matches_selector(&doc, link, ":link:any-link"));
    assert!(matches_selector(&doc, input, ":checked:required:enabled:read-write"));
    assert!(matches_selector(&doc, text_in, ":disabled:optional:placeholder-shown:read-only"));

    if let Some(el) = doc.element_mut(p) {
        el.hovered = true;
        el.focused = true;
    }
    assert!(matches_selector(&doc, p, "p:hover:focus:focus-visible"));
    assert!(matches_selector(&doc, main_div, "div:focus-within"));

    let found = query_selector(&doc, doc.root, "div.card > p.lead");
    assert_eq!(found, Some(p));
    let all_inputs = query_selector_all(&doc, body, "input");
    assert_eq!(all_inputs, vec![input, text_in]);
}

#[test]
fn nth_child_of_selector_and_specificity() {
    let mut doc = Document::new();
    let ul = doc.create_element("ul");
    doc.append_child(doc.root, ul);
    let mut items = Vec::new();
    for i in 0..5 {
        let li = doc.create_element("li");
        if i % 2 == 0 {
            doc.set_attribute(li, "class", "special");
        }
        doc.append_child(ul, li);
        items.push(li);
    }

    assert!(matches_selector(&doc, items[0], "li:nth-child(1 of .special)"));
    assert!(!matches_selector(&doc, items[1], "li:nth-child(2 of .special)"));
    assert!(matches_selector(&doc, items[2], "li:nth-child(2 of .special)"));
    assert!(matches_selector(&doc, items[4], "li:nth-last-child(1 of .special)"));
    assert!(matches_selector(&doc, items[2], "li:nth-of-type(3)"));

    let s_where = parse_selector_list("div:where(#id.cls)").unwrap().selectors[0].specificity();
    assert_eq!(s_where, Specificity(0, 0, 1));

    let s_is = parse_selector_list(":is(.a, #b.c, span)").unwrap().selectors[0].specificity();
    assert_eq!(s_is, Specificity(1, 1, 0));

    let s_has = parse_selector_list("div:has(> #x.y)").unwrap().selectors[0].specificity();
    assert_eq!(s_has, Specificity(1, 1, 1));

    let s_nth = parse_selector_list("li:nth-child(2n of #foo.bar)")
        .unwrap()
        .selectors[0]
        .specificity();
    assert_eq!(s_nth, Specificity(1, 2, 1));

    let s_pe = parse_selector_list("p::before").unwrap().selectors[0].specificity();
    assert_eq!(s_pe, Specificity(0, 0, 2));
}

#[test]
fn cascade_layers_and_important_reversal() {
    let (doc, _html, _head, _body, main_div, _h1, p) = build_sample_doc();
    let sheet = parse_stylesheet(
        "
        @layer base, theme;
        @layer theme {
            p { color: blue; background-color: yellow !important; }
        }
        @layer base {
            #target-sec p.lead { color: red; background-color: green !important; }
        }
        div.card { color: purple; }
        @layer theme {
            #target-sec.card.primary { color: orange; }
        }
        ",
    );
    let styles = compute_styles(&doc, &[sheet], Viewport::default());
    let p_style = styles.get(p).unwrap();
    let div_style = styles.get(main_div).unwrap();

    assert_eq!(p_style.color, Color::rgb(0, 0, 255));
    assert_eq!(p_style.background_color, Color::rgb(0, 128, 0));
    assert_eq!(div_style.color, Color::rgb(128, 0, 128));
}

#[test]
fn css_nesting_and_inline_styles_and_revert() {
    let (mut doc, _html, _head, _body, main_div, h1, p) = build_sample_doc();
    doc.set_attribute(p, "style", "font-size: 22px; color: crimson;");
    let sheet = parse_stylesheet(
        "
        .card {
            padding: 12px 24px;
            & > h1 {
                color: navy;
                font-size: revert;
            }
            .lead {
                color: teal !important;
                margin-top: 10px;
            }
            @media (min-width: 800px) {
                background-color: #f0f0f0;
            }
        }
        ",
    );
    let styles = compute_styles(&doc, &[sheet], Viewport::default());
    let div_s = styles.get(main_div).unwrap();
    let h1_s = styles.get(h1).unwrap();
    let p_s = styles.get(p).unwrap();

    assert_eq!(div_s.padding_top, LengthPercentageAuto::px(12.0));
    assert_eq!(div_s.padding_right, LengthPercentageAuto::px(24.0));
    assert_eq!(div_s.background_color, Color::rgb(0xf0, 0xf0, 0xf0));
    assert_eq!(h1_s.color, Color::rgb(0, 0, 128));
    assert!((h1_s.font_size - 32.0).abs() < 0.01);
    assert_eq!(p_s.font_size, 22.0);
    assert_eq!(p_s.color, Color::rgb(0, 128, 128));
    assert_eq!(p_s.margin_top, LengthPercentageAuto::px(10.0));
}

#[test]
fn custom_properties_var_fallback_and_cycles() {
    let (doc, _html, _head, _body, main_div, _h1, p) = build_sample_doc();
    let sheet = parse_stylesheet(
        "
        :root {
            --brand: #336699;
            --space: 16px;
            --double-space: calc(var(--space) * 2);
            --cyc-a: var(--cyc-b);
            --cyc-b: var(--cyc-a);
        }
        .card {
            color: var(--brand);
            padding: var(--double-space);
            border: 2px solid var(--missing, #ff0000);
        }
        p.lead {
            color: var(--cyc-a);
            margin-left: var(--undefined-len, 14px);
        }
        ",
    );
    let styles = compute_styles(&doc, &[sheet], Viewport::default());
    let div_s = styles.get(main_div).unwrap();
    let p_s = styles.get(p).unwrap();

    assert_eq!(div_s.color, Color::rgb(0x33, 0x66, 0x99));
    assert_eq!(div_s.padding_top, LengthPercentageAuto::px(32.0));
    assert_eq!(div_s.border_top_width, 2.0);
    assert_eq!(div_s.border_top_color, Color::rgb(255, 0, 0));
    assert_eq!(p_s.color, Color::rgb(0x33, 0x66, 0x99));
    assert_eq!(p_s.margin_left, LengthPercentageAuto::px(14.0));
}

#[test]
fn calc_min_max_clamp_evaluation() {
    let ctx = CalcContext {
        font_size: 20.0,
        root_font_size: 16.0,
        viewport_width: 1000.0,
        viewport_height: 800.0,
        percentage_base: 400.0,
    };

    let c1 = parse_calc_expr("calc(50% - 2rem + 10px)").unwrap();
    assert!((c1.evaluate(&ctx) - 178.0).abs() < 0.01);

    let c2 = parse_calc_expr("clamp(100px, 25vw, 200px)").unwrap();
    assert!((c2.evaluate(&ctx) - 200.0).abs() < 0.01);

    let c3 = parse_calc_expr("min(10em, 50%, 150px)").unwrap();
    assert!((c3.evaluate(&ctx) - 150.0).abs() < 0.01);

    let c4 = parse_calc_expr("max(1rem, 2em, 30px)").unwrap();
    assert!((c4.evaluate(&ctx) - 40.0).abs() < 0.01);
}

#[test]
fn modern_colors_all_formats() {
    assert_eq!(parse_color("rebeccapurple"), Some(Color::rgb(102, 51, 153)));
    assert_eq!(parse_color("cornflowerblue"), Some(Color::rgb(100, 149, 237)));
    assert_eq!(parse_color("transparent"), Some(Color::TRANSPARENT));
    assert_eq!(parse_color("#f0a"), Some(Color::rgb(255, 0, 170)));
    assert_eq!(parse_color("#0f08"), Some(Color::rgba(0, 255, 0, 136)));
    assert_eq!(parse_color("#11223344"), Some(Color::rgba(0x11, 0x22, 0x33, 0x44)));
    assert_eq!(parse_color("rgb(10 20 30 / 50%)"), Some(Color::rgba(10, 20, 30, 128)));
    assert_eq!(parse_color("rgba(255, 0, 128, 0.5)"), Some(Color::rgba(255, 0, 128, 128)));
    assert_eq!(parse_color("hsl(120deg 100% 50%)"), Some(Color::rgb(0, 255, 0)));
    assert_eq!(parse_color("hwb(0 0% 0%)"), Some(Color::rgb(255, 0, 0)));

    let ok_white = parse_color("oklab(100% 0 0)").unwrap();
    assert_eq!(ok_white, Color::WHITE);
    let ok_lch = parse_color("oklch(0.627955 0.257683 29.23388)").unwrap();
    assert!(ok_lch.r > 240 && ok_lch.g < 20 && ok_lch.b < 20);

    let lab_w = parse_color("lab(100% 0 0)").unwrap();
    assert!(lab_w.r >= 253 && lab_w.g >= 253 && lab_w.b >= 253);
    let lch_b = parse_color("lch(0% 0 0)").unwrap();
    assert_eq!(lch_b, Color::BLACK);

    let mix_srgb = parse_color("color-mix(in srgb, red 50%, blue)").unwrap();
    assert_eq!(mix_srgb, Color::rgb(128, 0, 128));

    let mix_oklch = parse_color("color-mix(in oklch, white 50%, black)").unwrap();
    assert!(mix_oklch.r > 80 && mix_oklch.r < 150);
    assert_eq!(mix_oklch.r, mix_oklch.g);
    assert_eq!(mix_oklch.g, mix_oklch.b);
}

#[test]
fn media_queries_and_supports_rules() {
    let (doc, _html, _head, _body, main_div, _h1, p) = build_sample_doc();
    let sheet = parse_stylesheet(
        "
        @media screen and (width >= 1000px) and (prefers-color-scheme: dark) {
            .card { background-color: #111111; }
        }
        @media (400px <= width <= 800px), print {
            .card { background-color: #eeeeee; }
        }
        @media (orientation: portrait) or (max-height: 500px) {
            p { font-weight: 700; }
        }
        @media (hover: hover) and (pointer: fine) {
            .card { cursor: pointer; }
        }
        @supports (display: grid) and (not (foo-bar: baz)) {
            .card { display: grid; grid-template-columns: repeat(3, minmax(100px, 1fr)); }
        }
        @supports (display: fake-layout) {
            p { color: yellow; }
        }
        @supports selector(div:has(> p)) {
            p { letter-spacing: 1px; }
        }
        ",
    );

    let dark_wide = Viewport {
        width: 1200.0,
        height: 800.0,
        dark_mode: true,
    };
    let styles = compute_styles(&doc, &[sheet.clone()], dark_wide);
    let div_s = styles.get(main_div).unwrap();
    assert_eq!(div_s.background_color, Color::rgb(0x11, 0x11, 0x11));
    assert_eq!(div_s.display, Display::Grid);
    assert_eq!(div_s.cursor, Cursor::Pointer);
    assert_eq!(div_s.grid_template_columns.len(), 1);
    assert_eq!(styles.get(p).unwrap().color, Color::BLACK);
    assert_eq!(styles.get(p).unwrap().font_weight, 400);
    assert_eq!(styles.get(p).unwrap().letter_spacing, 1.0);

    let narrow_light = Viewport {
        width: 600.0,
        height: 800.0,
        dark_mode: false,
    };
    let styles2 = compute_styles(&doc, &[sheet], narrow_light);
    assert_eq!(
        styles2.get(main_div).unwrap().background_color,
        Color::rgb(0xee, 0xee, 0xee)
    );
    assert_eq!(styles2.get(p).unwrap().font_weight, 700);
}

#[test]
fn error_recovery_current_color_css_wide_and_more_gradients() {
    let (doc, _html, _head, _body, main_div, _h1, p) = build_sample_doc();
    let sheet = parse_stylesheet(
        "
        @unknown-at-rule foo { bar: baz; }
        .broken.. { color: red; }
        .card {
            malformed-declaration-without-colon;
            color: #123456;
            border: 3px solid currentColor;
            background-image: radial-gradient(circle, white, black);
            inset: 5px 10px 15px 20px;
            grid-area: 1 / 2 / 3 / 4;
            unknown-prop: 999widgets;
        }
        p.lead {
            border-top-style: solid;
            border-top-width: inherit;
            color: initial;
            padding-top: unset;
            background-image: conic-gradient(from 90deg, red, yellow, green);
        }
        ",
    );
    let map = compute_styles(&doc, &[sheet], Viewport::default());
    let card = map.get(main_div).unwrap();
    assert_eq!(card.color, Color::rgb(0x12, 0x34, 0x56));
    assert_eq!(card.border_top_color, Color::rgb(0x12, 0x34, 0x56));
    assert_eq!(card.top, LengthPercentageAuto::px(5.0));
    assert_eq!(card.right, LengthPercentageAuto::px(10.0));
    assert_eq!(card.bottom, LengthPercentageAuto::px(15.0));
    assert_eq!(card.left, LengthPercentageAuto::px(20.0));
    assert_eq!(card.grid_row_start, GridPlacement::Line(1));
    assert_eq!(card.grid_column_start, GridPlacement::Line(2));
    assert_eq!(card.grid_row_end, GridPlacement::Line(3));
    assert_eq!(card.grid_column_end, GridPlacement::Line(4));
    assert!(matches!(
        card.background_image,
        BackgroundImage::RadialGradient { ref stops, .. } if stops.len() == 2
    ));

    let p_s = map.get(p).unwrap();
    assert_eq!(p_s.border_top_width, 3.0);
    assert_eq!(p_s.color, Color::BLACK);
    assert_eq!(p_s.padding_top, LengthPercentageAuto::px(0.0));
    assert!(matches!(
        p_s.background_image,
        BackgroundImage::ConicGradient { from_deg, ref stops, .. }
            if (from_deg - 90.0).abs() < 0.01 && stops.len() == 3
    ));
}

#[test]
fn shorthands_gradients_shadows_transforms_filters_pseudo_elements() {
    let (mut doc, _html, head, _body, main_div, _h1, _p) = build_sample_doc();
    let hidden_el = doc.create_element("div");
    doc.set_attribute(hidden_el, "hidden", "");
    doc.append_child(main_div, hidden_el);

    let legacy_td = doc.create_element("td");
    doc.set_attribute(legacy_td, "width", "200");
    doc.set_attribute(legacy_td, "bgcolor", "#abcdef");
    doc.set_attribute(legacy_td, "align", "center");
    doc.set_attribute(legacy_td, "nowrap", "");
    doc.append_child(main_div, legacy_td);

    let sheet = parse_stylesheet(
        "
        @import url(\"theme.css\") layer(theme) screen and (min-width: 600px);
        @keyframes fade {
            from { opacity: 0; }
            50% { opacity: 0.5; }
            to { opacity: 1; }
        }
        @font-face {
            font-family: \"PeedleSans\";
            src: url(\"peedle.woff2\") format(\"woff2\");
            font-weight: 700;
            font-style: normal;
        }
        .card {
            display: flex;
            flex-flow: column wrap;
            flex: 2 0 120px;
            gap: 8px 16px;
            place-items: center flex-end;
            border-radius: 8px 16px;
            background: linear-gradient(90deg, red 0%, blue 100%) no-repeat #222;
            box-shadow: inset 0 2px 4px 1px rgba(0, 0, 0, 0.5);
            text-shadow: 1px 2px 3px black;
            transform: translate(10px, 20%) rotate(45deg) scale(1.5);
            filter: blur(4px) brightness(120%);
            font: italic 700 18px/1.5 \"Inter\", sans-serif;
            overflow: hidden auto;
            outline: 2px dashed green;
            list-style: square inside;
            text-decoration: underline dotted red;
            grid-column: 2 / span 3;
        }
        .card::before {
            content: \"[\";
            color: red;
        }
        .card::after {
            content: \"]\";
            display: inline-block;
        }
        ",
    );

    assert!(sheet.rules.iter().any(|r| matches!(
        r,
        Rule::Import(imp) if imp.url == "theme.css" && imp.layer == Some(Some("theme".into()))
    )));

    let map = compute_styles(&doc, &[sheet], Viewport::default());
    assert_eq!(map.get(head).unwrap().display, Display::None);
    assert_eq!(map.get(hidden_el).unwrap().display, Display::None);

    let td_s = map.get(legacy_td).unwrap();
    assert_eq!(td_s.width, LengthPercentageAuto::px(200.0));
    assert_eq!(td_s.background_color, Color::rgb(0xab, 0xcd, 0xef));
    assert_eq!(td_s.text_align, TextAlign::Center);
    assert_eq!(td_s.white_space, WhiteSpace::Nowrap);

    let card = map.get(main_div).unwrap();
    assert_eq!(card.display, Display::Flex);
    assert_eq!(card.flex_direction, FlexDirection::Column);
    assert_eq!(card.flex_wrap, FlexWrap::Wrap);
    assert_eq!(card.flex_grow, 2.0);
    assert_eq!(card.flex_shrink, 0.0);
    assert_eq!(card.flex_basis, LengthPercentageAuto::px(120.0));
    assert_eq!(card.row_gap, LengthPercentageAuto::px(8.0));
    assert_eq!(card.column_gap, LengthPercentageAuto::px(16.0));
    assert_eq!(card.align_items, AlignItems::Center);
    assert_eq!(card.justify_items, AlignItems::FlexEnd);
    assert_eq!(card.border_top_left_radius, LengthPercentageAuto::px(8.0));
    assert_eq!(card.border_top_right_radius, LengthPercentageAuto::px(16.0));
    assert_eq!(card.background_color, Color::rgb(0x22, 0x22, 0x22));
    assert_eq!(card.background_repeat, BackgroundRepeat::NoRepeat);
    assert!(matches!(
        card.background_image,
        BackgroundImage::LinearGradient { angle_deg, ref stops, .. }
            if (angle_deg - 90.0).abs() < 0.01 && stops.len() == 2
    ));
    assert_eq!(card.box_shadow.len(), 1);
    assert!(card.box_shadow[0].inset);
    assert_eq!(card.text_shadow.len(), 1);
    assert_eq!(card.transform.len(), 3);
    assert_eq!(card.filter.len(), 2);
    assert_eq!(card.font_style, FontStyle::Italic);
    assert_eq!(card.font_weight, 700);
    assert_eq!(card.font_size, 18.0);
    assert_eq!(card.line_height, LineHeight::Number(1.5));
    assert_eq!(card.font_family, vec!["Inter".to_string(), "sans-serif".to_string()]);
    assert_eq!(card.overflow_x, Overflow::Hidden);
    assert_eq!(card.overflow_y, Overflow::Auto);
    assert_eq!(card.outline_width, 2.0);
    assert_eq!(card.outline_style, BorderStyle::Dashed);
    assert_eq!(card.list_style_type, ListStyleType::Square);
    assert_eq!(card.list_style_position, ListStylePosition::Inside);
    assert_eq!(card.text_decoration_line, TextDecorationLine::Underline);
    assert_eq!(card.text_decoration_style, TextDecorationStyle::Dotted);
    assert_eq!(card.grid_column_start, GridPlacement::Line(2));
    assert_eq!(card.grid_column_end, GridPlacement::Span(3));

    assert_eq!(get_computed_property(card, "display"), "flex");
    assert_eq!(get_computed_property(card, "font-size"), "18px");
    assert_eq!(get_computed_property(card, "overflow"), "hidden auto");

    let before = map.before(main_div).unwrap();
    assert_eq!(before.content.as_deref(), Some("["));
    assert_eq!(before.color, Color::rgb(255, 0, 0));

    let after = map.after(main_div).unwrap();
    assert_eq!(after.content.as_deref(), Some("]"));
    assert_eq!(after.display, Display::InlineBlock);

    assert!(map.keyframes.contains_key("fade"));
    assert_eq!(map.keyframes["fade"].stops.len(), 3);
    assert_eq!(map.font_faces.len(), 1);
    assert_eq!(map.font_faces[0].font_family.as_deref(), Some("PeedleSans"));
    assert_eq!(map.font_faces[0].src, vec!["peedle.woff2".to_string()]);
}
