use crate::parser::{
    parse_inline_declarations, Declaration, FontFaceRule, KeyframesRule, Rule, StyleRule,
    Stylesheet, Viewport,
};
use crate::selector::{
    matches_complex_selector, ComplexSelector, PseudoElement, SimpleSelector, Specificity,
};
use crate::style::{
    is_inherited_property, ua_stylesheet, AlignContent, AlignItems, AlignSelf, BackgroundClip,
    BackgroundRepeat, BackgroundSize, BorderCollapse, BorderStyle, BoxSizing, Clear, ComputedStyle,
    Cursor, Display, FlexDirection, FlexWrap, Float, FontStyle, GridAutoFlow, JustifyContent,
    LineHeight, ListStylePosition, ListStyleType, ObjectFit, Overflow, OverflowWrap, PointerEvents,
    Position, TableLayout, TextAlign, TextDecorationLine, TextDecorationStyle, TextOverflow,
    TextTransform, UserSelect, VerticalAlign, Visibility, WhiteSpace, WordBreak,
};
use crate::tokenizer::{serialize_tokens, tokenize, Token};
use crate::values::{
    parse_background_image, parse_box_shadows, parse_color_from_tokens, parse_filters,
    parse_grid_placement, parse_grid_tracks, parse_length_percentage_auto, parse_text_shadows,
    parse_transforms, split_commas, split_whitespace_groups, trim_ws, CalcContext, Color, Length,
    LengthPercentageAuto, LengthUnit,
};
use peedle_html::{Document, ElementData, NodeId};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[derive(Clone, Debug, Default)]
pub struct StyleMap {
    pub styles: HashMap<NodeId, ComputedStyle>,
    pub before_styles: HashMap<NodeId, ComputedStyle>,
    pub after_styles: HashMap<NodeId, ComputedStyle>,
    pub keyframes: HashMap<String, KeyframesRule>,
    pub font_faces: Vec<FontFaceRule>,
    pub viewport: Viewport,
}

impl StyleMap {
    pub fn get(&self, id: NodeId) -> Option<&ComputedStyle> {
        self.styles.get(&id)
    }

    pub fn before(&self, id: NodeId) -> Option<&ComputedStyle> {
        self.before_styles.get(&id)
    }

    pub fn after(&self, id: NodeId) -> Option<&ComputedStyle> {
        self.after_styles.get(&id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Origin {
    Ua,
    Presentational,
    Author,
    Inline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CascadeKey {
    bucket: u8,
    layer_sort: usize,
    specificity: Specificity,
    order: usize,
    sub_order: usize,
}

impl CascadeKey {
    fn new(
        origin: Origin,
        important: bool,
        layer_rank: Option<usize>,
        specificity: Specificity,
        order: usize,
        sub_order: usize,
    ) -> Self {
        let (bucket, layer_sort) = match (origin, important) {
            (Origin::Ua, false) => (0, layer_rank.unwrap_or(usize::MAX)),
            (Origin::Presentational, false) => (1, 0),
            (Origin::Author, false) => match layer_rank {
                Some(rank) => (2, rank),
                None => (3, 0),
            },
            (Origin::Inline, false) => (4, 0),
            (Origin::Author, true) => match layer_rank {
                None => (5, 0),
                Some(rank) => (6, usize::MAX - rank),
            },
            (Origin::Presentational, true) => (5, 0),
            (Origin::Inline, true) => (7, 0),
            (Origin::Ua, true) => (8, layer_rank.map_or(0, |r| usize::MAX - r)),
        };
        Self {
            bucket,
            layer_sort,
            specificity,
            order,
            sub_order,
        }
    }
}

struct FlatRule<'a> {
    selector: &'a ComplexSelector,
    declarations: &'a [Declaration],
    origin: Origin,
    layer_rank: Option<usize>,
    order: usize,
    specificity: Specificity,
}

struct RuleIndex<'a> {
    rules: Vec<FlatRule<'a>>,
    by_id: HashMap<&'a str, Vec<usize>>,
    by_class: HashMap<&'a str, Vec<usize>>,
    by_tag: HashMap<String, Vec<usize>>,
    universal: Vec<usize>,
}

impl<'a> RuleIndex<'a> {
    fn build(rules: Vec<FlatRule<'a>>) -> Self {
        let mut by_id: HashMap<&'a str, Vec<usize>> = HashMap::new();
        let mut by_class: HashMap<&'a str, Vec<usize>> = HashMap::new();
        let mut by_tag: HashMap<String, Vec<usize>> = HashMap::new();
        let mut universal: Vec<usize> = Vec::new();

        for (idx, rule) in rules.iter().enumerate() {
            let mut key_id: Option<&'a str> = None;
            let mut key_class: Option<&'a str> = None;
            let mut key_tag: Option<&'a str> = None;
            if let Some(last) = rule.selector.compounds.last() {
                for s in &last.simple {
                    match s {
                        SimpleSelector::Id(id) if key_id.is_none() => key_id = Some(id.as_str()),
                        SimpleSelector::Class(c) if key_class.is_none() => {
                            key_class = Some(c.as_str())
                        }
                        SimpleSelector::Type(t) if key_tag.is_none() => {
                            key_tag = Some(t.as_str())
                        }
                        _ => {}
                    }
                }
            }
            if let Some(id) = key_id {
                by_id.entry(id).or_default().push(idx);
            } else if let Some(c) = key_class {
                by_class.entry(c).or_default().push(idx);
            } else if let Some(t) = key_tag {
                by_tag.entry(t.to_ascii_lowercase()).or_default().push(idx);
            } else {
                universal.push(idx);
            }
        }

        Self {
            rules,
            by_id,
            by_class,
            by_tag,
            universal,
        }
    }

    fn for_each_candidate(&self, el: &ElementData, mut f: impl FnMut(&FlatRule<'a>)) {
        for &idx in &self.universal {
            f(&self.rules[idx]);
        }
        let tag_lower = el.tag_name.to_ascii_lowercase();
        if let Some(bucket) = self.by_tag.get(&tag_lower) {
            for &idx in bucket {
                f(&self.rules[idx]);
            }
        }
        if let Some(id) = el.id() {
            if let Some(bucket) = self.by_id.get(id) {
                for &idx in bucket {
                    f(&self.rules[idx]);
                }
            }
        }
        if let Some(cls_attr) = el.get_attr("class") {
            let mut seen: Vec<&str> = Vec::new();
            for cls in cls_attr.split_ascii_whitespace() {
                if seen.contains(&cls) {
                    continue;
                }
                seen.push(cls);
                if let Some(bucket) = self.by_class.get(cls) {
                    for &idx in bucket {
                        f(&self.rules[idx]);
                    }
                }
            }
        }
    }
}

struct LayerRegistry {
    order: HashMap<String, usize>,
    next_rank: usize,
    anon_counter: usize,
}

impl LayerRegistry {
    fn new() -> Self {
        Self {
            order: HashMap::new(),
            next_rank: 0,
            anon_counter: 0,
        }
    }

    fn register(&mut self, name: &str) -> usize {
        if let Some(&r) = self.order.get(name) {
            return r;
        }
        let r = self.next_rank;
        self.next_rank += 1;
        self.order.insert(name.to_string(), r);
        r
    }

    fn register_anon(&mut self, prefix: Option<&str>) -> String {
        let id = self.anon_counter;
        self.anon_counter += 1;
        let full = match prefix {
            Some(p) => format!("{}.__anon_{}", p, id),
            None => format!("__anon_{}", id),
        };
        self.register(&full);
        full
    }
}

pub fn compute_styles(
    doc: &Document,
    author_sheets: &[Stylesheet],
    viewport: Viewport,
) -> StyleMap {
    let ua = ua_stylesheet();
    compute_styles_with_ua_root(doc, doc.root, &ua, author_sheets, viewport)
}

pub fn compute_styles_for_root(
    doc: &Document,
    root: NodeId,
    author_sheets: &[Stylesheet],
    viewport: Viewport,
) -> StyleMap {
    let ua = ua_stylesheet();
    compute_styles_with_ua_root(doc, root, &ua, author_sheets, viewport)
}

pub fn compute_styles_with_ua(
    doc: &Document,
    ua_sheet: &Stylesheet,
    author_sheets: &[Stylesheet],
    viewport: Viewport,
) -> StyleMap {
    compute_styles_with_ua_root(doc, doc.root, ua_sheet, author_sheets, viewport)
}

pub fn compute_styles_with_ua_root(
    doc: &Document,
    root: NodeId,
    ua_sheet: &Stylesheet,
    author_sheets: &[Stylesheet],
    viewport: Viewport,
) -> StyleMap {
    let mut map = StyleMap {
        viewport,
        ..Default::default()
    };

    let mut flat_rules: Vec<FlatRule<'_>> = Vec::new();
    let mut order_counter = 0usize;
    let mut ua_layers = LayerRegistry::new();
    collect_rules(
        &ua_sheet.rules,
        Origin::Ua,
        None,
        &mut ua_layers,
        &viewport,
        &mut flat_rules,
        &mut order_counter,
        &mut map.keyframes,
        &mut map.font_faces,
    );

    let mut author_layers = LayerRegistry::new();
    for sheet in author_sheets {
        collect_rules(
            &sheet.rules,
            Origin::Author,
            None,
            &mut author_layers,
            &viewport,
            &mut flat_rules,
            &mut order_counter,
            &mut map.keyframes,
            &mut map.font_faces,
        );
    }

    let rule_index = RuleIndex::build(flat_rules);

    let prev_dark = crate::values::set_active_dark_mode(viewport.dark_mode);
    let root_default = ComputedStyle::default();
    let mut root_font_size = 16.0f32;
    if let Some(doc_el) = doc.document_element_of(root) {
        resolve_subtree(
            doc,
            doc_el,
            &root_default,
            &mut root_font_size,
            true,
            &rule_index,
            &viewport,
            &mut map,
        );
    } else {
        for child in doc.element_children(root) {
            resolve_subtree(
                doc,
                child,
                &root_default,
                &mut root_font_size,
                true,
                &rule_index,
                &viewport,
                &mut map,
            );
        }
    }
    crate::values::set_active_dark_mode(prev_dark);

    map
}

#[allow(clippy::too_many_arguments)]
fn collect_rules<'a>(
    rules: &'a [Rule],
    origin: Origin,
    current_layer: Option<&str>,
    layers: &mut LayerRegistry,
    viewport: &Viewport,
    out: &mut Vec<FlatRule<'a>>,
    order: &mut usize,
    keyframes: &mut HashMap<String, KeyframesRule>,
    font_faces: &mut Vec<FontFaceRule>,
) {
    let layer_rank = current_layer.map(|name| layers.register(name));
    for rule in rules {
        match rule {
            Rule::Style(StyleRule {
                selectors,
                declarations,
            }) => {
                for sel in &selectors.selectors {
                    *order += 1;
                    out.push(FlatRule {
                        selector: sel,
                        declarations,
                        origin,
                        layer_rank,
                        order: *order,
                        specificity: sel.specificity(),
                    });
                }
            }
            Rule::Media { query, rules: sub } => {
                if query.matches(viewport) {
                    collect_rules(
                        sub,
                        origin,
                        current_layer,
                        layers,
                        viewport,
                        out,
                        order,
                        keyframes,
                        font_faces,
                    );
                }
            }
            Rule::Supports {
                condition,
                rules: sub,
            } => {
                if condition.evaluate() {
                    collect_rules(
                        sub,
                        origin,
                        current_layer,
                        layers,
                        viewport,
                        out,
                        order,
                        keyframes,
                        font_faces,
                    );
                }
            }
            Rule::LayerStatement(stmt) => {
                for name in &stmt.names {
                    let full = match current_layer {
                        Some(p) => format!("{}.{}", p, name),
                        None => name.clone(),
                    };
                    layers.register(&full);
                }
            }
            Rule::LayerBlock(blk) => {
                let full = match &blk.name {
                    Some(name) => {
                        let joined = match current_layer {
                            Some(p) => format!("{}.{}", p, name),
                            None => name.clone(),
                        };
                        layers.register(&joined);
                        joined
                    }
                    None => layers.register_anon(current_layer),
                };
                collect_rules(
                    &blk.rules,
                    origin,
                    Some(&full),
                    layers,
                    viewport,
                    out,
                    order,
                    keyframes,
                    font_faces,
                );
            }
            Rule::Keyframes(kf) => {
                keyframes.insert(kf.name.clone(), kf.clone());
            }
            Rule::FontFace(ff) => {
                font_faces.push(ff.clone());
            }
            Rule::Import(_) => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_subtree(
    doc: &Document,
    node: NodeId,
    parent_style: &ComputedStyle,
    root_font_size: &mut f32,
    is_root_element: bool,
    rule_index: &RuleIndex<'_>,
    viewport: &Viewport,
    map: &mut StyleMap,
) {
    let Some(el) = doc.element(node) else {
        return;
    };

    let mut matched: Vec<(CascadeKey, Declaration)> = Vec::new();
    let mut before_matched: Vec<(CascadeKey, Declaration)> = Vec::new();
    let mut after_matched: Vec<(CascadeKey, Declaration)> = Vec::new();

    for (idx, decl) in presentational_declarations(el).into_iter().enumerate() {
        let key = CascadeKey::new(
            Origin::Presentational,
            false,
            None,
            Specificity::ZERO,
            0,
            idx,
        );
        matched.push((key, decl));
    }

    rule_index.for_each_candidate(el, |rule| {
        if !matches_complex_selector(doc, node, rule.selector, None) {
            return;
        }
        let spec = rule.specificity;
        let target = match rule.selector.pseudo_element {
            None => &mut matched,
            Some(PseudoElement::Before) => &mut before_matched,
            Some(PseudoElement::After) => &mut after_matched,
            Some(_) => return,
        };
        for (idx, decl) in rule.declarations.iter().enumerate() {
            let key = CascadeKey::new(
                rule.origin,
                decl.important,
                rule.layer_rank,
                spec,
                rule.order,
                idx,
            );
            target.push((key, decl.clone()));
        }
    });

    if let Some(inline_str) = el.get_attr("style") {
        let inline_decls = parse_inline_declarations(inline_str);
        for (idx, decl) in inline_decls.into_iter().enumerate() {
            let key = CascadeKey::new(
                Origin::Inline,
                decl.important,
                None,
                Specificity::ZERO,
                usize::MAX,
                idx,
            );
            matched.push((key, decl));
        }
    }

    let rfs = if is_root_element {
        16.0
    } else {
        *root_font_size
    };
    let computed = compute_element_style(parent_style, rfs, viewport, matched);
    if is_root_element {
        *root_font_size = computed.font_size;
    }

    if !before_matched.is_empty() {
        let mut pseudo = compute_element_style(&computed, *root_font_size, viewport, before_matched);
        if pseudo.content.is_some() {
            if pseudo.display == Display::Inline {
                pseudo.display = Display::Inline;
            }
            map.before_styles.insert(node, pseudo);
        }
    }
    if !after_matched.is_empty() {
        let pseudo = compute_element_style(&computed, *root_font_size, viewport, after_matched);
        if pseudo.content.is_some() {
            map.after_styles.insert(node, pseudo);
        }
    }

    let style_clone = computed.clone();
    map.styles.insert(node, computed);

    for child in doc.element_children(node) {
        resolve_subtree(
            doc,
            child,
            &style_clone,
            root_font_size,
            false,
            rule_index,
            viewport,
            map,
        );
    }
}

fn compute_element_style(
    parent: &ComputedStyle,
    root_font_size: f32,
    viewport: &Viewport,
    matched: Vec<(CascadeKey, Declaration)>,
) -> ComputedStyle {
    let mut style = ComputedStyle::inherit_from(parent);

    let mut custom_winners: HashMap<String, (CascadeKey, Vec<Token>)> = HashMap::new();
    let mut normal_entries: Vec<(CascadeKey, Declaration)> = Vec::new();

    for (key, decl) in matched {
        if decl.name.starts_with("--") {
            let replace = custom_winners
                .get(&decl.name)
                .map_or(true, |(existing, _)| key >= *existing);
            if replace {
                custom_winners.insert(decl.name, (key, decl.value));
            }
        } else {
            normal_entries.push((key, decl));
        }
    }

    if !custom_winners.is_empty() {
        let mut raw_custom = (*parent.custom_properties).clone();
        let mut to_resolve: Vec<String> = Vec::with_capacity(custom_winners.len());
        for (name, (_, val)) in custom_winners {
            if is_css_wide(&val, "initial") {
                raw_custom.remove(&name);
            } else if is_css_wide(&val, "inherit") || is_css_wide(&val, "unset") {
                if let Some(p) = parent.custom_properties.get(&name) {
                    raw_custom.insert(name, p.clone());
                } else {
                    raw_custom.remove(&name);
                }
            } else {
                to_resolve.push(name.clone());
                raw_custom.insert(name, val);
            }
        }

        let mut resolved_custom = raw_custom.clone();
        for name in &to_resolve {
            if let Some(raw_val) = raw_custom.get(name) {
                if contains_var_call(raw_val) {
                    let mut visiting = HashSet::new();
                    visiting.insert(name.clone());
                    match resolve_var_in_tokens(raw_val, &raw_custom, &mut visiting) {
                        Some(tokens) => {
                            resolved_custom.insert(name.clone(), tokens);
                        }
                        None => {
                            resolved_custom.remove(name);
                        }
                    }
                }
            }
        }
        style.custom_properties = Rc::new(resolved_custom);
    }

    let mut expanded_lists: HashMap<String, Vec<(CascadeKey, Vec<Token>)>> = HashMap::new();
    let mut ua_winners: HashMap<String, (CascadeKey, Vec<Token>)> = HashMap::new();

    for (key, decl) in normal_entries {
        let resolved_val = if contains_var_call(&decl.value) {
            let mut visiting = HashSet::new();
            match resolve_var_in_tokens(&decl.value, &style.custom_properties, &mut visiting) {
                Some(v) => v,
                None => continue,
            }
        } else {
            decl.value
        };

        let longhands = expand_shorthand(&decl.name, &resolved_val);
        for (sub_idx, (prop_name, prop_val)) in longhands.into_iter().enumerate() {
            let mut sub_key = key;
            sub_key.sub_order = key.sub_order.saturating_mul(16).saturating_add(sub_idx);

            if key.bucket == 0 || key.bucket == 8 {
                let rep = ua_winners
                    .get(&prop_name)
                    .map_or(true, |(k, _)| sub_key >= *k);
                if rep {
                    ua_winners.insert(prop_name.clone(), (sub_key, prop_val.clone()));
                }
            }

            expanded_lists
                .entry(prop_name)
                .or_default()
                .push((sub_key, prop_val));
        }
    }

    if let Some(mut entries) = expanded_lists.remove("font-size") {
        entries.sort_by_key(|(k, _)| *k);
        for (_, val) in entries {
            if let Some(tokens) = resolve_revert("font-size", val, &ua_winners) {
                apply_font_size(&mut style, parent, root_font_size, viewport, &tokens);
            }
        }
    }

    let ctx = CalcContext {
        font_size: style.font_size,
        root_font_size,
        viewport_width: viewport.width,
        viewport_height: viewport.height,
        percentage_base: 0.0,
    };

    if let Some(mut entries) = expanded_lists.remove("color") {
        entries.sort_by_key(|(k, _)| *k);
        for (_, val) in entries {
            if let Some(tokens) = resolve_revert("color", val, &ua_winners) {
                apply_color_prop(&mut style, parent, &tokens);
            }
        }
    }

    for (prop, mut entries) in expanded_lists {
        entries.sort_by_key(|(k, _)| *k);
        for (_, val) in entries {
            let Some(tokens) = resolve_revert(&prop, val, &ua_winners) else {
                continue;
            };
            apply_longhand(&mut style, parent, &prop, &tokens, &ctx);
        }
    }

    if matches!(style.border_top_style, BorderStyle::None | BorderStyle::Hidden) {
        style.border_top_width = 0.0;
    }
    if matches!(style.border_right_style, BorderStyle::None | BorderStyle::Hidden) {
        style.border_right_width = 0.0;
    }
    if matches!(style.border_bottom_style, BorderStyle::None | BorderStyle::Hidden) {
        style.border_bottom_width = 0.0;
    }
    if matches!(style.border_left_style, BorderStyle::None | BorderStyle::Hidden) {
        style.border_left_width = 0.0;
    }
    if matches!(style.outline_style, BorderStyle::None | BorderStyle::Hidden) {
        style.outline_width = 0.0;
    }

    if style.display != Display::None
        && (style.float != Float::None
            || matches!(style.position, Position::Absolute | Position::Fixed))
    {
        style.display = match style.display {
            Display::Inline | Display::InlineBlock | Display::TableCell | Display::TableRow | Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup | Display::TableCaption => Display::Block,
            Display::InlineFlex => Display::Flex,
            Display::InlineGrid => Display::Grid,
            Display::InlineTable => Display::Table,
            other => other,
        };
    }

    style
}

fn resolve_revert(
    prop: &str,
    tokens: Vec<Token>,
    ua_winners: &HashMap<String, (CascadeKey, Vec<Token>)>,
) -> Option<Vec<Token>> {
    if is_css_wide(&tokens, "revert") {
        if let Some((_, ua_val)) = ua_winners.get(prop) {
            return Some(ua_val.clone());
        }
        return Some(vec![Token::Ident("unset".to_string())]);
    }
    Some(tokens)
}

fn contains_var_call(tokens: &[Token]) -> bool {
    tokens
        .iter()
        .any(|t| matches!(t, Token::Function(name) if name.eq_ignore_ascii_case("var")))
}

fn resolve_var_in_tokens(
    tokens: &[Token],
    custom_props: &HashMap<String, Vec<Token>>,
    visiting: &mut HashSet<String>,
) -> Option<Vec<Token>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if let Token::Function(name) = &tokens[i] {
            if name.eq_ignore_ascii_case("var") {
                let start = i + 1;
                let mut depth = 1usize;
                i += 1;
                while i < tokens.len() && depth > 0 {
                    match tokens[i] {
                        Token::OpenParen | Token::Function(_) => depth += 1,
                        Token::CloseParen => depth -= 1,
                        _ => {}
                    }
                    i += 1;
                }
                let end = if depth == 0 { i - 1 } else { i };
                let inner = &tokens[start..end];
                let (var_name, fallback) = parse_var_args(inner)?;
                if visiting.contains(&var_name) {
                    return None;
                }
                if let Some(raw_val) = custom_props.get(&var_name) {
                    visiting.insert(var_name.clone());
                    let sub = resolve_var_in_tokens(raw_val, custom_props, visiting);
                    visiting.remove(&var_name);
                    if let Some(sub_tokens) = sub {
                        out.extend(sub_tokens);
                        continue;
                    }
                }
                if let Some(fb) = fallback {
                    let sub = resolve_var_in_tokens(fb, custom_props, visiting)?;
                    out.extend(sub);
                    continue;
                }
                return None;
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    Some(out)
}

fn parse_var_args(inner: &[Token]) -> Option<(String, Option<&[Token]>)> {
    let mut comma_pos = None;
    let mut depth = 0usize;
    for (i, t) in inner.iter().enumerate() {
        match t {
            Token::OpenParen | Token::Function(_) | Token::OpenSquare => depth += 1,
            Token::CloseParen | Token::CloseSquare => depth = depth.saturating_sub(1),
            Token::Comma if depth == 0 => {
                comma_pos = Some(i);
                break;
            }
            _ => {}
        }
    }
    let (name_part, fb) = match comma_pos {
        Some(idx) => (&inner[..idx], Some(trim_ws(&inner[idx + 1..]))),
        None => (inner, None),
    };
    let name_toks = trim_ws(name_part);
    if name_toks.len() != 1 {
        return None;
    }
    if let Token::Ident(s) = &name_toks[0] {
        if s.starts_with("--") {
            return Some((s.clone(), fb));
        }
    }
    None
}

fn is_css_wide(tokens: &[Token], kw: &str) -> bool {
    let t = trim_ws(tokens);
    t.len() == 1 && matches!(&t[0], Token::Ident(s) if s.eq_ignore_ascii_case(kw))
}

fn is_any_css_wide(tokens: &[Token]) -> bool {
    is_css_wide(tokens, "inherit")
        || is_css_wide(tokens, "initial")
        || is_css_wide(tokens, "unset")
        || is_css_wide(tokens, "revert")
}

fn expand_shorthand(name: &str, value: &[Token]) -> Vec<(String, Vec<Token>)> {
    let trimmed = trim_ws(value);
    if is_any_css_wide(trimmed) {
        let targets: &[&str] = match name {
            "margin" => &["margin-top", "margin-right", "margin-bottom", "margin-left"],
            "padding" => &[
                "padding-top",
                "padding-right",
                "padding-bottom",
                "padding-left",
            ],
            "inset" => &["top", "right", "bottom", "left"],
            "overflow" => &["overflow-x", "overflow-y"],
            "gap" => &["row-gap", "column-gap"],
            "border-radius" => &[
                "border-top-left-radius",
                "border-top-right-radius",
                "border-bottom-right-radius",
                "border-bottom-left-radius",
            ],
            "border" => &[
                "border-top-width",
                "border-right-width",
                "border-bottom-width",
                "border-left-width",
                "border-top-style",
                "border-right-style",
                "border-bottom-style",
                "border-left-style",
                "border-top-color",
                "border-right-color",
                "border-bottom-color",
                "border-left-color",
            ],
            "flex" => &["flex-grow", "flex-shrink", "flex-basis"],
            "flex-flow" => &["flex-direction", "flex-wrap"],
            "place-items" => &["align-items", "justify-items"],
            "place-content" => &["align-content", "justify-content"],
            "place-self" => &["align-self", "justify-self"],
            "outline" => &["outline-width", "outline-style", "outline-color"],
            "list-style" => &["list-style-type", "list-style-position"],
            _ => return vec![(name.to_string(), trimmed.to_vec())],
        };
        return targets
            .iter()
            .map(|t| (t.to_string(), trimmed.to_vec()))
            .collect();
    }

    match name {
        "margin" => expand_four_sides(
            trimmed,
            ["margin-top", "margin-right", "margin-bottom", "margin-left"],
        ),
        "margin-inline" => expand_two_pair(trimmed, "margin-left", "margin-right"),
        "margin-block" => expand_two_pair(trimmed, "margin-top", "margin-bottom"),
        "margin-inline-start" => vec![("margin-left".to_string(), trimmed.to_vec())],
        "margin-inline-end" => vec![("margin-right".to_string(), trimmed.to_vec())],
        "margin-block-start" => vec![("margin-top".to_string(), trimmed.to_vec())],
        "margin-block-end" => vec![("margin-bottom".to_string(), trimmed.to_vec())],
        "padding" => expand_four_sides(
            trimmed,
            [
                "padding-top",
                "padding-right",
                "padding-bottom",
                "padding-left",
            ],
        ),
        "padding-inline" => expand_two_pair(trimmed, "padding-left", "padding-right"),
        "padding-block" => expand_two_pair(trimmed, "padding-top", "padding-bottom"),
        "padding-inline-start" => vec![("padding-left".to_string(), trimmed.to_vec())],
        "padding-inline-end" => vec![("padding-right".to_string(), trimmed.to_vec())],
        "padding-block-start" => vec![("padding-top".to_string(), trimmed.to_vec())],
        "padding-block-end" => vec![("padding-bottom".to_string(), trimmed.to_vec())],
        "inset" => expand_four_sides(trimmed, ["top", "right", "bottom", "left"]),
        "inset-inline" => expand_two_pair(trimmed, "left", "right"),
        "inset-block" => expand_two_pair(trimmed, "top", "bottom"),
        "inset-inline-start" => vec![("left".to_string(), trimmed.to_vec())],
        "inset-inline-end" => vec![("right".to_string(), trimmed.to_vec())],
        "inset-block-start" => vec![("top".to_string(), trimmed.to_vec())],
        "inset-block-end" => vec![("bottom".to_string(), trimmed.to_vec())],
        "inline-size" => vec![("width".to_string(), trimmed.to_vec())],
        "block-size" => vec![("height".to_string(), trimmed.to_vec())],
        "min-inline-size" => vec![("min-width".to_string(), trimmed.to_vec())],
        "max-inline-size" => vec![("max-width".to_string(), trimmed.to_vec())],
        "min-block-size" => vec![("min-height".to_string(), trimmed.to_vec())],
        "max-block-size" => vec![("max-height".to_string(), trimmed.to_vec())],
        "border-inline-start" => expand_border_side("left", trimmed),
        "border-inline-end" => expand_border_side("right", trimmed),
        "border-block-start" => expand_border_side("top", trimmed),
        "border-block-end" => expand_border_side("bottom", trimmed),
        "border-inline" => {
            let mut out = expand_border_side("left", trimmed);
            out.extend(expand_border_side("right", trimmed));
            out
        }
        "border-block" => {
            let mut out = expand_border_side("top", trimmed);
            out.extend(expand_border_side("bottom", trimmed));
            out
        }
        "border-start-start-radius" => vec![("border-top-left-radius".to_string(), trimmed.to_vec())],
        "border-start-end-radius" => vec![("border-top-right-radius".to_string(), trimmed.to_vec())],
        "border-end-end-radius" => vec![("border-bottom-right-radius".to_string(), trimmed.to_vec())],
        "border-end-start-radius" => vec![("border-bottom-left-radius".to_string(), trimmed.to_vec())],
        "border-inline-start-width" => vec![("border-left-width".to_string(), trimmed.to_vec())],
        "border-inline-end-width" => vec![("border-right-width".to_string(), trimmed.to_vec())],
        "border-block-start-width" => vec![("border-top-width".to_string(), trimmed.to_vec())],
        "border-block-end-width" => vec![("border-bottom-width".to_string(), trimmed.to_vec())],
        "border-inline-start-style" => vec![("border-left-style".to_string(), trimmed.to_vec())],
        "border-inline-end-style" => vec![("border-right-style".to_string(), trimmed.to_vec())],
        "border-block-start-style" => vec![("border-top-style".to_string(), trimmed.to_vec())],
        "border-block-end-style" => vec![("border-bottom-style".to_string(), trimmed.to_vec())],
        "border-inline-start-color" => vec![("border-left-color".to_string(), trimmed.to_vec())],
        "border-inline-end-color" => vec![("border-right-color".to_string(), trimmed.to_vec())],
        "border-block-start-color" => vec![("border-top-color".to_string(), trimmed.to_vec())],
        "border-block-end-color" => vec![("border-bottom-color".to_string(), trimmed.to_vec())],
        "border-width" => expand_four_sides(
            trimmed,
            [
                "border-top-width",
                "border-right-width",
                "border-bottom-width",
                "border-left-width",
            ],
        ),
        "border-style" => expand_four_sides(
            trimmed,
            [
                "border-top-style",
                "border-right-style",
                "border-bottom-style",
                "border-left-style",
            ],
        ),
        "border-color" => expand_four_sides(
            trimmed,
            [
                "border-top-color",
                "border-right-color",
                "border-bottom-color",
                "border-left-color",
            ],
        ),
        "border-radius" => {
            let main_part = match trimmed.iter().position(|t| *t == Token::Delim('/')) {
                Some(idx) => trim_ws(&trimmed[..idx]),
                None => trimmed,
            };
            expand_four_sides(
                main_part,
                [
                    "border-top-left-radius",
                    "border-top-right-radius",
                    "border-bottom-right-radius",
                    "border-bottom-left-radius",
                ],
            )
        }
        "border" => {
            let Some((w, s, c)) = parse_border_parts(trimmed) else {
                return Vec::new();
            };
            let mut out = Vec::new();
            for side in ["top", "right", "bottom", "left"] {
                out.push((format!("border-{}-width", side), w.clone()));
                out.push((format!("border-{}-style", side), s.clone()));
                out.push((format!("border-{}-color", side), c.clone()));
            }
            out
        }
        "border-top" => expand_border_side("top", trimmed),
        "border-right" => expand_border_side("right", trimmed),
        "border-bottom" => expand_border_side("bottom", trimmed),
        "border-left" => expand_border_side("left", trimmed),
        "outline" => {
            let Some((w, s, c)) = parse_border_parts(trimmed) else {
                return Vec::new();
            };
            vec![
                ("outline-width".to_string(), w),
                ("outline-style".to_string(), s),
                ("outline-color".to_string(), c),
            ]
        }
        "overflow" => {
            let g = split_whitespace_groups(trimmed);
            if g.len() == 1 {
                vec![
                    ("overflow-x".to_string(), g[0].to_vec()),
                    ("overflow-y".to_string(), g[0].to_vec()),
                ]
            } else if g.len() >= 2 {
                vec![
                    ("overflow-x".to_string(), g[0].to_vec()),
                    ("overflow-y".to_string(), g[1].to_vec()),
                ]
            } else {
                Vec::new()
            }
        }
        "gap" => {
            let g = split_whitespace_groups(trimmed);
            if g.len() == 1 {
                vec![
                    ("row-gap".to_string(), g[0].to_vec()),
                    ("column-gap".to_string(), g[0].to_vec()),
                ]
            } else if g.len() >= 2 {
                vec![
                    ("row-gap".to_string(), g[0].to_vec()),
                    ("column-gap".to_string(), g[1].to_vec()),
                ]
            } else {
                Vec::new()
            }
        }
        "place-items" => expand_two_pair(trimmed, "align-items", "justify-items"),
        "place-content" => expand_two_pair(trimmed, "align-content", "justify-content"),
        "place-self" => expand_two_pair(trimmed, "align-self", "justify-self"),
        "flex" => expand_flex(trimmed),
        "flex-flow" => expand_flex_flow(trimmed),
        "background" => expand_background(trimmed),
        "font" => expand_font(trimmed),
        "list-style" => expand_list_style(trimmed),
        "text-decoration" => expand_text_decoration(trimmed),
        "grid-column" => expand_grid_span_pair(trimmed, "grid-column-start", "grid-column-end"),
        "grid-row" => expand_grid_span_pair(trimmed, "grid-row-start", "grid-row-end"),
        "grid-area" => expand_grid_area(trimmed),
        "grid-template" => expand_grid_template(trimmed),
        _ => vec![(name.to_string(), trimmed.to_vec())],
    }
}

fn expand_four_sides(tokens: &[Token], names: [&str; 4]) -> Vec<(String, Vec<Token>)> {
    let g = split_whitespace_groups(tokens);
    let vals: [&[Token]; 4] = match g.len() {
        1 => [g[0], g[0], g[0], g[0]],
        2 => [g[0], g[1], g[0], g[1]],
        3 => [g[0], g[1], g[2], g[1]],
        4.. => [g[0], g[1], g[2], g[3]],
        _ => return Vec::new(),
    };
    names
        .iter()
        .zip(vals.iter())
        .map(|(n, v)| ((*n).to_string(), v.to_vec()))
        .collect()
}

fn expand_two_pair(tokens: &[Token], first: &str, second: &str) -> Vec<(String, Vec<Token>)> {
    let g = split_whitespace_groups(tokens);
    match g.len() {
        1 => vec![
            (first.to_string(), g[0].to_vec()),
            (second.to_string(), g[0].to_vec()),
        ],
        2.. => vec![
            (first.to_string(), g[0].to_vec()),
            (second.to_string(), g[1].to_vec()),
        ],
        _ => Vec::new(),
    }
}

fn expand_border_side(side: &str, tokens: &[Token]) -> Vec<(String, Vec<Token>)> {
    let Some((w, s, c)) = parse_border_parts(tokens) else {
        return Vec::new();
    };
    vec![
        (format!("border-{}-width", side), w),
        (format!("border-{}-style", side), s),
        (format!("border-{}-color", side), c),
    ]
}

fn parse_border_parts(tokens: &[Token]) -> Option<(Vec<Token>, Vec<Token>, Vec<Token>)> {
    let mut width = vec![Token::Ident("medium".to_string())];
    let mut style = vec![Token::Ident("none".to_string())];
    let mut color = vec![Token::Ident("currentColor".to_string())];
    let mut has_w = false;
    let mut has_s = false;
    let mut has_c = false;

    for g in split_whitespace_groups(tokens) {
        if g.len() == 1 {
            if let Token::Ident(s) = &g[0] {
                let lower = s.to_ascii_lowercase();
                if parse_border_style_str(&lower).is_some() {
                    if has_s {
                        return None;
                    }
                    has_s = true;
                    style = g.to_vec();
                    continue;
                }
                if matches!(lower.as_str(), "thin" | "medium" | "thick") {
                    if has_w {
                        return None;
                    }
                    has_w = true;
                    width = g.to_vec();
                    continue;
                }
            }
        }
        if parse_length_percentage_auto(g).is_some()
            && !matches!(g.first(), Some(Token::Ident(s)) if s.eq_ignore_ascii_case("auto"))
        {
            if has_w {
                return None;
            }
            has_w = true;
            width = g.to_vec();
            continue;
        }
        if parse_color_from_tokens(g, Color::BLACK).is_some() {
            if has_c {
                return None;
            }
            has_c = true;
            color = g.to_vec();
            continue;
        }
        return None;
    }
    Some((width, style, color))
}

fn expand_flex(tokens: &[Token]) -> Vec<(String, Vec<Token>)> {
    let g = split_whitespace_groups(tokens);
    if g.len() == 1 {
        if let Token::Ident(s) = &g[0][0] {
            match s.to_ascii_lowercase().as_str() {
                "none" => {
                    return vec![
                        ("flex-grow".into(), tokenize("0")),
                        ("flex-shrink".into(), tokenize("0")),
                        ("flex-basis".into(), tokenize("auto")),
                    ];
                }
                "auto" => {
                    return vec![
                        ("flex-grow".into(), tokenize("1")),
                        ("flex-shrink".into(), tokenize("1")),
                        ("flex-basis".into(), tokenize("auto")),
                    ];
                }
                "initial" => {
                    return vec![
                        ("flex-grow".into(), tokenize("0")),
                        ("flex-shrink".into(), tokenize("1")),
                        ("flex-basis".into(), tokenize("auto")),
                    ];
                }
                _ => {}
            }
        }
        if matches!(g[0].first(), Some(Token::Number { .. })) {
            return vec![
                ("flex-grow".into(), g[0].to_vec()),
                ("flex-shrink".into(), tokenize("1")),
                ("flex-basis".into(), tokenize("0%")),
            ];
        }
        return vec![
            ("flex-grow".into(), tokenize("1")),
            ("flex-shrink".into(), tokenize("1")),
            ("flex-basis".into(), g[0].to_vec()),
        ];
    }
    if g.len() == 2 {
        let first_num = matches!(g[0].first(), Some(Token::Number { .. }));
        let second_num = matches!(g[1].first(), Some(Token::Number { .. }));
        if first_num && second_num {
            return vec![
                ("flex-grow".into(), g[0].to_vec()),
                ("flex-shrink".into(), g[1].to_vec()),
                ("flex-basis".into(), tokenize("0%")),
            ];
        }
        if first_num {
            return vec![
                ("flex-grow".into(), g[0].to_vec()),
                ("flex-shrink".into(), tokenize("1")),
                ("flex-basis".into(), g[1].to_vec()),
            ];
        }
    }
    if g.len() >= 3 {
        return vec![
            ("flex-grow".into(), g[0].to_vec()),
            ("flex-shrink".into(), g[1].to_vec()),
            ("flex-basis".into(), g[2].to_vec()),
        ];
    }
    Vec::new()
}

fn expand_flex_flow(tokens: &[Token]) -> Vec<(String, Vec<Token>)> {
    let mut out = Vec::new();
    for g in split_whitespace_groups(tokens) {
        if let Some(Token::Ident(s)) = g.first() {
            let lower = s.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "row" | "row-reverse" | "column" | "column-reverse"
            ) {
                out.push(("flex-direction".into(), g.to_vec()));
            } else if matches!(lower.as_str(), "nowrap" | "wrap" | "wrap-reverse") {
                out.push(("flex-wrap".into(), g.to_vec()));
            }
        }
    }
    out
}

fn expand_background(tokens: &[Token]) -> Vec<(String, Vec<Token>)> {
    let mut out = vec![
        ("background-color".into(), tokenize("transparent")),
        ("background-image".into(), tokenize("none")),
        ("background-repeat".into(), tokenize("repeat")),
        ("background-size".into(), tokenize("auto")),
        ("background-attachment".into(), tokenize("scroll")),
        ("background-position".into(), tokenize("0% 0%")),
    ];
    let layers = split_commas(tokens);
    let last_layer = layers.last().copied().unwrap_or(tokens);
    let first_layer = layers.first().copied().unwrap_or(tokens);

    let mut pos_tokens: Vec<Token> = Vec::new();
    for g in split_whitespace_groups(first_layer) {
        if parse_background_image(g, Color::BLACK).is_some() {
            if !matches!(g.first(), Some(Token::Ident(s)) if s.eq_ignore_ascii_case("none")) {
                out.push(("background-image".into(), g.to_vec()));
                continue;
            }
        }
        if let Some(Token::Ident(s)) = g.first() {
            let lower = s.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "repeat" | "repeat-x" | "repeat-y" | "no-repeat" | "space" | "round"
            ) {
                out.push(("background-repeat".into(), g.to_vec()));
                continue;
            }
            if matches!(lower.as_str(), "cover" | "contain") {
                out.push(("background-size".into(), g.to_vec()));
                continue;
            }
            if matches!(lower.as_str(), "fixed" | "scroll" | "local") {
                out.push(("background-attachment".into(), g.to_vec()));
                continue;
            }
            if matches!(lower.as_str(), "left" | "right" | "top" | "bottom" | "center") {
                if !pos_tokens.is_empty() {
                    pos_tokens.push(Token::Whitespace);
                }
                pos_tokens.extend_from_slice(g);
                continue;
            }
        }
        if matches!(
            g.first(),
            Some(Token::Dimension { .. } | Token::Percentage { .. } | Token::Number { .. })
        ) {
            if !pos_tokens.is_empty() {
                pos_tokens.push(Token::Whitespace);
            }
            pos_tokens.extend_from_slice(g);
            continue;
        }
    }
    if !pos_tokens.is_empty() {
        out.push(("background-position".into(), pos_tokens));
    }

    let mut color_count = 0usize;
    for g in split_whitespace_groups(last_layer) {
        if parse_color_from_tokens(g, Color::BLACK).is_some() {
            color_count += 1;
            if color_count > 1 {
                return Vec::new();
            }
            out.push(("background-color".into(), g.to_vec()));
        }
    }
    out
}

fn expand_font(tokens: &[Token]) -> Vec<(String, Vec<Token>)> {
    let groups = split_whitespace_groups(tokens);
    let mut out = Vec::new();
    let mut size_idx = None;
    for (idx, g) in groups.iter().enumerate() {
        if let Some(Token::Ident(s)) = g.first() {
            let l = s.to_ascii_lowercase();
            if matches!(l.as_str(), "italic" | "oblique") {
                out.push(("font-style".into(), g.to_vec()));
                continue;
            }
            if matches!(l.as_str(), "bold" | "bolder" | "lighter") {
                out.push(("font-weight".into(), g.to_vec()));
                continue;
            }
        }
        if let Some(Token::Number { value, .. }) = g.first() {
            if *value >= 100.0 && *value <= 900.0 && size_idx.is_none() {
                out.push(("font-weight".into(), g.to_vec()));
                continue;
            }
        }
        if is_font_size_group(g) && size_idx.is_none() {
            size_idx = Some(idx);
            if let Some(slash) = g.iter().position(|t| *t == Token::Delim('/')) {
                out.push(("font-size".into(), g[..slash].to_vec()));
                out.push(("line-height".into(), g[slash + 1..].to_vec()));
            } else {
                out.push(("font-size".into(), g.to_vec()));
            }
            break;
        }
    }
    if let Some(sidx) = size_idx {
        let mut fam_start = sidx + 1;
        if let Some(next_g) = groups.get(fam_start) {
            if next_g.len() == 1 && next_g[0] == Token::Delim('/') {
                if let Some(lh) = groups.get(fam_start + 1) {
                    out.push(("line-height".into(), lh.to_vec()));
                    fam_start += 2;
                }
            } else if next_g.first() == Some(&Token::Delim('/')) {
                out.push(("line-height".into(), next_g[1..].to_vec()));
                fam_start += 1;
            }
        }
        if fam_start < groups.len() {
            let mut fam_tokens = Vec::new();
            for (k, g) in groups[fam_start..].iter().enumerate() {
                if k > 0 {
                    fam_tokens.push(Token::Whitespace);
                }
                fam_tokens.extend_from_slice(g);
            }
            out.push(("font-family".into(), fam_tokens));
        }
    }
    out
}

fn is_font_size_group(g: &[Token]) -> bool {
    let head = match g.iter().position(|t| *t == Token::Delim('/')) {
        Some(idx) => &g[..idx],
        None => g,
    };
    if parse_length_percentage_auto(head).is_some()
        && !matches!(head.first(), Some(Token::Ident(s)) if s.eq_ignore_ascii_case("auto"))
    {
        return true;
    }
    if head.len() == 1 {
        if let Token::Ident(s) = &head[0] {
            return matches!(
                s.to_ascii_lowercase().as_str(),
                "xx-small"
                    | "x-small"
                    | "small"
                    | "medium"
                    | "large"
                    | "x-large"
                    | "xx-large"
                    | "smaller"
                    | "larger"
            );
        }
    }
    false
}

fn expand_list_style(tokens: &[Token]) -> Vec<(String, Vec<Token>)> {
    let mut out = Vec::new();
    for g in split_whitespace_groups(tokens) {
        if let Some(Token::Ident(s)) = g.first() {
            let l = s.to_ascii_lowercase();
            if matches!(l.as_str(), "inside" | "outside") {
                out.push(("list-style-position".into(), g.to_vec()));
            } else if parse_list_style_type(&l).is_some() {
                out.push(("list-style-type".into(), g.to_vec()));
            }
        }
    }
    out
}

fn expand_text_decoration(tokens: &[Token]) -> Vec<(String, Vec<Token>)> {
    if let [Token::Ident(s)] = tokens {
        let l = s.to_ascii_lowercase();
        if matches!(l.as_str(), "inherit" | "initial" | "unset" | "revert") {
            return vec![
                ("text-decoration-line".into(), tokens.to_vec()),
                ("text-decoration-style".into(), tokens.to_vec()),
                ("text-decoration-color".into(), tokens.to_vec()),
            ];
        }
    }
    let mut out = Vec::new();
    for g in split_whitespace_groups(tokens) {
        if let Some(Token::Ident(s)) = g.first() {
            let l = s.to_ascii_lowercase();
            if matches!(l.as_str(), "none" | "underline" | "overline" | "line-through") {
                out.push(("text-decoration-line".into(), g.to_vec()));
                continue;
            }
            if matches!(l.as_str(), "solid" | "double" | "dotted" | "dashed" | "wavy") {
                out.push(("text-decoration-style".into(), g.to_vec()));
                continue;
            }
        }
        if parse_color_from_tokens(g, Color::BLACK).is_some() {
            out.push(("text-decoration-color".into(), g.to_vec()));
        }
    }
    out
}

fn expand_grid_span_pair(
    tokens: &[Token],
    start_prop: &str,
    end_prop: &str,
) -> Vec<(String, Vec<Token>)> {
    if let Some(slash) = tokens.iter().position(|t| *t == Token::Delim('/')) {
        vec![
            (start_prop.into(), trim_ws(&tokens[..slash]).to_vec()),
            (end_prop.into(), trim_ws(&tokens[slash + 1..]).to_vec()),
        ]
    } else {
        vec![
            (start_prop.into(), tokens.to_vec()),
            (end_prop.into(), tokenize("auto")),
        ]
    }
}

fn expand_grid_area(tokens: &[Token]) -> Vec<(String, Vec<Token>)> {
    let parts: Vec<&[Token]> = tokens
        .split(|t| *t == Token::Delim('/'))
        .map(trim_ws)
        .collect();
    if parts.len() == 1 {
        if let [Token::Ident(name)] = parts[0] {
            if !name.eq_ignore_ascii_case("auto") {
                return vec![
                    ("grid-area".into(), parts[0].to_vec()),
                    ("grid-row-start".into(), tokenize("auto")),
                    ("grid-column-start".into(), tokenize("auto")),
                    ("grid-row-end".into(), tokenize("auto")),
                    ("grid-column-end".into(), tokenize("auto")),
                ];
            }
        }
    }
    let mut out = vec![("grid-area".into(), tokenize("auto"))];
    if let Some(p0) = parts.first() {
        out.push(("grid-row-start".into(), p0.to_vec()));
    }
    if let Some(p1) = parts.get(1) {
        out.push(("grid-column-start".into(), p1.to_vec()));
    }
    if let Some(p2) = parts.get(2) {
        out.push(("grid-row-end".into(), p2.to_vec()));
    }
    if let Some(p3) = parts.get(3) {
        out.push(("grid-column-end".into(), p3.to_vec()));
    }
    out
}

fn expand_grid_template(tokens: &[Token]) -> Vec<(String, Vec<Token>)> {
    let trimmed = trim_ws(tokens);
    if trimmed.len() == 1 {
        if let Token::Ident(s) = &trimmed[0] {
            if s.eq_ignore_ascii_case("none") {
                return vec![
                    ("grid-template-rows".into(), tokenize("none")),
                    ("grid-template-columns".into(), tokenize("none")),
                    ("grid-template-areas".into(), tokenize("none")),
                ];
            }
        }
    }
    if let Some(slash) = trimmed.iter().position(|t| *t == Token::Delim('/')) {
        let row_part = trim_ws(&trimmed[..slash]);
        let col_part = trim_ws(&trimmed[slash + 1..]);
        let mut out = Vec::new();
        let has_area_strings = row_part.iter().any(|t| matches!(t, Token::String(_)));
        if has_area_strings {
            let mut area_tokens = Vec::new();
            let mut row_tokens = Vec::new();
            for t in row_part {
                if matches!(t, Token::String(_)) {
                    area_tokens.push(t.clone());
                } else {
                    row_tokens.push(t.clone());
                }
            }
            out.push(("grid-template-areas".into(), area_tokens));
            if !trim_ws(&row_tokens).is_empty() {
                out.push(("grid-template-rows".into(), row_tokens));
            }
        } else {
            out.push(("grid-template-rows".into(), row_part.to_vec()));
        }
        out.push(("grid-template-columns".into(), col_part.to_vec()));
        return out;
    }
    Vec::new()
}

fn apply_font_size(
    style: &mut ComputedStyle,
    parent: &ComputedStyle,
    root_font_size: f32,
    viewport: &Viewport,
    tokens: &[Token],
) {
    let trimmed = trim_ws(tokens);
    if is_css_wide(trimmed, "inherit") || is_css_wide(trimmed, "unset") {
        style.font_size = parent.font_size;
        return;
    }
    if is_css_wide(trimmed, "initial") {
        style.font_size = 16.0;
        return;
    }
    if trimmed.len() == 1 {
        if let Token::Ident(s) = &trimmed[0] {
            style.font_size = match s.to_ascii_lowercase().as_str() {
                "xx-small" => 9.0,
                "x-small" => 10.0,
                "small" => 13.0,
                "medium" => 16.0,
                "large" => 18.0,
                "x-large" => 24.0,
                "xx-large" => 32.0,
                "smaller" => parent.font_size / 1.2,
                "larger" => parent.font_size * 1.2,
                _ => return,
            };
            return;
        }
    }
    let ctx = CalcContext {
        font_size: parent.font_size,
        root_font_size,
        viewport_width: viewport.width,
        viewport_height: viewport.height,
        percentage_base: parent.font_size,
    };
    if let Some(lpa) = parse_length_percentage_auto(trimmed) {
        if let Some(px) = lpa.resolve(&ctx) {
            if px >= 0.0 {
                style.font_size = px;
            }
        }
    }
}

fn apply_color_prop(style: &mut ComputedStyle, parent: &ComputedStyle, tokens: &[Token]) {
    let trimmed = trim_ws(tokens);
    if is_css_wide(trimmed, "inherit") || is_css_wide(trimmed, "unset") {
        style.color = parent.color;
    } else if is_css_wide(trimmed, "initial") {
        style.color = Color::BLACK;
    } else if let Some(c) = parse_color_from_tokens(trimmed, parent.color) {
        style.color = c;
    }
    style.border_top_color = style.color;
    style.border_right_color = style.color;
    style.border_bottom_color = style.color;
    style.border_left_color = style.color;
    style.outline_color = style.color;
}

fn resolve_lpa_for_style(
    tokens: &[Token],
    ctx: &CalcContext,
    allow_auto: bool,
) -> Option<LengthPercentageAuto> {
    let lpa = parse_length_percentage_auto(tokens)?;
    match lpa {
        LengthPercentageAuto::Auto => {
            if allow_auto {
                Some(LengthPercentageAuto::Auto)
            } else {
                None
            }
        }
        LengthPercentageAuto::Percent(p) => Some(LengthPercentageAuto::Percent(p)),
        LengthPercentageAuto::Length(l) => {
            if l.unit == LengthUnit::Percent {
                Some(LengthPercentageAuto::Percent(l.value))
            } else {
                Some(LengthPercentageAuto::Length(Length::px(l.to_px(ctx))))
            }
        }
        LengthPercentageAuto::Calc(expr) => {
            if calc_uses_percent(&expr) {
                Some(LengthPercentageAuto::Calc(expr))
            } else {
                Some(LengthPercentageAuto::px(expr.evaluate(ctx)))
            }
        }
    }
}

fn calc_uses_percent(expr: &crate::values::CalcExpr) -> bool {
    use crate::values::CalcExpr;
    match expr {
        CalcExpr::Number(_) => false,
        CalcExpr::Length(l) => l.unit == LengthUnit::Percent,
        CalcExpr::Add(a, b)
        | CalcExpr::Sub(a, b)
        | CalcExpr::Mul(a, b)
        | CalcExpr::Div(a, b) => calc_uses_percent(a) || calc_uses_percent(b),
        CalcExpr::Min(v) | CalcExpr::Max(v) => v.iter().any(calc_uses_percent),
        CalcExpr::Clamp(a, b, c) => {
            calc_uses_percent(a) || calc_uses_percent(b) || calc_uses_percent(c)
        }
    }
}

fn parse_border_width_px(tokens: &[Token], ctx: &CalcContext) -> Option<f32> {
    let t = trim_ws(tokens);
    if t.len() == 1 {
        if let Token::Ident(s) = &t[0] {
            return match s.to_ascii_lowercase().as_str() {
                "thin" => Some(1.0),
                "medium" => Some(3.0),
                "thick" => Some(5.0),
                _ => None,
            };
        }
    }
    parse_length_percentage_auto(t)
        .and_then(|l| l.resolve(ctx))
        .map(|v| v.max(0.0))
}

fn parse_border_style_str(s: &str) -> Option<BorderStyle> {
    Some(match s {
        "none" => BorderStyle::None,
        "hidden" => BorderStyle::Hidden,
        "solid" => BorderStyle::Solid,
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "double" => BorderStyle::Double,
        "groove" => BorderStyle::Groove,
        "ridge" => BorderStyle::Ridge,
        "inset" => BorderStyle::Inset,
        "outset" => BorderStyle::Outset,
        _ => return None,
    })
}

fn parse_list_style_type(s: &str) -> Option<ListStyleType> {
    Some(match s {
        "disc" => ListStyleType::Disc,
        "circle" => ListStyleType::Circle,
        "square" => ListStyleType::Square,
        "decimal" => ListStyleType::Decimal,
        "lower-alpha" | "lower-latin" => ListStyleType::LowerAlpha,
        "upper-alpha" | "upper-latin" => ListStyleType::UpperAlpha,
        "lower-roman" => ListStyleType::LowerRoman,
        "upper-roman" => ListStyleType::UpperRoman,
        "none" => ListStyleType::None,
        _ => return None,
    })
}

fn single_ident_lower(tokens: &[Token]) -> Option<String> {
    let t = trim_ws(tokens);
    if t.len() == 1 {
        if let Token::Ident(s) = &t[0] {
            return Some(s.to_ascii_lowercase());
        }
    }
    None
}

fn apply_longhand(
    style: &mut ComputedStyle,
    parent: &ComputedStyle,
    prop: &str,
    tokens: &[Token],
    ctx: &CalcContext,
) {
    let trimmed = trim_ws(tokens);
    if trimmed.is_empty() {
        return;
    }
    if is_css_wide(trimmed, "inherit") {
        copy_property(style, parent, prop);
        return;
    }
    if is_css_wide(trimmed, "initial") {
        let init = ComputedStyle::default();
        copy_property(style, &init, prop);
        return;
    }
    if is_css_wide(trimmed, "unset") {
        if is_inherited_property(prop) {
            copy_property(style, parent, prop);
        } else {
            let init = ComputedStyle::default();
            copy_property(style, &init, prop);
        }
        return;
    }

    let ident = single_ident_lower(trimmed);
    match prop {
        "display" => {
            if let Some(s) = ident.as_deref() {
                style.display = match s {
                    "inline" => Display::Inline,
                    "block" => Display::Block,
                    "inline-block" => Display::InlineBlock,
                    "flex" => Display::Flex,
                    "inline-flex" => Display::InlineFlex,
                    "grid" => Display::Grid,
                    "inline-grid" => Display::InlineGrid,
                    "table" => Display::Table,
                    "inline-table" => Display::InlineTable,
                    "table-row" => Display::TableRow,
                    "table-cell" => Display::TableCell,
                    "table-row-group" => Display::TableRowGroup,
                    "table-header-group" => Display::TableHeaderGroup,
                    "table-footer-group" => Display::TableFooterGroup,
                    "table-caption" => Display::TableCaption,
                    "list-item" => Display::ListItem,
                    "contents" => Display::Contents,
                    "flow-root" => Display::FlowRoot,
                    "none" => Display::None,
                    _ => return,
                };
            }
        }
        "position" => {
            if let Some(s) = ident.as_deref() {
                style.position = match s {
                    "static" => Position::Static,
                    "relative" => Position::Relative,
                    "absolute" => Position::Absolute,
                    "fixed" => Position::Fixed,
                    "sticky" => Position::Sticky,
                    _ => return,
                };
            }
        }
        "box-sizing" => {
            if let Some(s) = ident.as_deref() {
                style.box_sizing = match s {
                    "content-box" => BoxSizing::ContentBox,
                    "border-box" => BoxSizing::BorderBox,
                    _ => return,
                };
            }
        }
        "float" => {
            if let Some(s) = ident.as_deref() {
                style.float = match s {
                    "none" => Float::None,
                    "left" => Float::Left,
                    "right" => Float::Right,
                    _ => return,
                };
            }
        }
        "clear" => {
            if let Some(s) = ident.as_deref() {
                style.clear = match s {
                    "none" => Clear::None,
                    "left" => Clear::Left,
                    "right" => Clear::Right,
                    "both" => Clear::Both,
                    _ => return,
                };
            }
        }
        "z-index" => {
            if ident.as_deref() == Some("auto") {
                style.z_index = None;
            } else if let Some(Token::Number { value, .. }) = trimmed.first() {
                style.z_index = Some(value.round() as i32);
            }
        }
        "width" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, true) {
                style.width = v;
            }
        }
        "height" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, true) {
                style.height = v;
            }
        }
        "min-width" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, true) {
                style.min_width = v;
            }
        }
        "max-width" => {
            if ident.as_deref() == Some("none") {
                style.max_width = LengthPercentageAuto::Auto;
            } else if let Some(v) = resolve_lpa_for_style(trimmed, ctx, true) {
                style.max_width = v;
            }
        }
        "min-height" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, true) {
                style.min_height = v;
            }
        }
        "max-height" => {
            if ident.as_deref() == Some("none") {
                style.max_height = LengthPercentageAuto::Auto;
            } else if let Some(v) = resolve_lpa_for_style(trimmed, ctx, true) {
                style.max_height = v;
            }
        }
        "aspect-ratio" => {
            if ident.as_deref() == Some("auto") {
                style.aspect_ratio = None;
            } else {
                let non_ws: Vec<&Token> = trimmed
                    .iter()
                    .filter(|t| **t != Token::Whitespace)
                    .collect();
                match non_ws.as_slice() {
                    [Token::Number { value, .. }] if *value > 0.0 => {
                        style.aspect_ratio = Some(*value);
                    }
                    [Token::Number { value: a, .. }, Token::Delim('/'), Token::Number { value: b, .. }]
                        if *a > 0.0 && *b > 0.0 =>
                    {
                        style.aspect_ratio = Some(*a / *b);
                    }
                    _ => {}
                }
            }
        }
        "top" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, true) {
                style.top = v;
            }
        }
        "right" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, true) {
                style.right = v;
            }
        }
        "bottom" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, true) {
                style.bottom = v;
            }
        }
        "left" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, true) {
                style.left = v;
            }
        }
        "margin-top" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, true) {
                style.margin_top = v;
            }
        }
        "margin-right" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, true) {
                style.margin_right = v;
            }
        }
        "margin-bottom" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, true) {
                style.margin_bottom = v;
            }
        }
        "margin-left" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, true) {
                style.margin_left = v;
            }
        }
        "padding-top" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, false) {
                style.padding_top = v;
            }
        }
        "padding-right" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, false) {
                style.padding_right = v;
            }
        }
        "padding-bottom" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, false) {
                style.padding_bottom = v;
            }
        }
        "padding-left" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, false) {
                style.padding_left = v;
            }
        }
        "border-top-width" => {
            if let Some(w) = parse_border_width_px(trimmed, ctx) {
                style.border_top_width = w;
            }
        }
        "border-right-width" => {
            if let Some(w) = parse_border_width_px(trimmed, ctx) {
                style.border_right_width = w;
            }
        }
        "border-bottom-width" => {
            if let Some(w) = parse_border_width_px(trimmed, ctx) {
                style.border_bottom_width = w;
            }
        }
        "border-left-width" => {
            if let Some(w) = parse_border_width_px(trimmed, ctx) {
                style.border_left_width = w;
            }
        }
        "border-top-style" => {
            if let Some(s) = ident.as_deref().and_then(parse_border_style_str) {
                style.border_top_style = s;
            }
        }
        "border-right-style" => {
            if let Some(s) = ident.as_deref().and_then(parse_border_style_str) {
                style.border_right_style = s;
            }
        }
        "border-bottom-style" => {
            if let Some(s) = ident.as_deref().and_then(parse_border_style_str) {
                style.border_bottom_style = s;
            }
        }
        "border-left-style" => {
            if let Some(s) = ident.as_deref().and_then(parse_border_style_str) {
                style.border_left_style = s;
            }
        }
        "border-top-color" => {
            if let Some(c) = parse_color_from_tokens(trimmed, style.color) {
                style.border_top_color = c;
            }
        }
        "border-right-color" => {
            if let Some(c) = parse_color_from_tokens(trimmed, style.color) {
                style.border_right_color = c;
            }
        }
        "border-bottom-color" => {
            if let Some(c) = parse_color_from_tokens(trimmed, style.color) {
                style.border_bottom_color = c;
            }
        }
        "border-left-color" => {
            if let Some(c) = parse_color_from_tokens(trimmed, style.color) {
                style.border_left_color = c;
            }
        }
        "border-top-left-radius" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, false) {
                style.border_top_left_radius = v;
            }
        }
        "border-top-right-radius" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, false) {
                style.border_top_right_radius = v;
            }
        }
        "border-bottom-right-radius" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, false) {
                style.border_bottom_right_radius = v;
            }
        }
        "border-bottom-left-radius" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, false) {
                style.border_bottom_left_radius = v;
            }
        }
        "border-collapse" => {
            if let Some(s) = ident.as_deref() {
                style.border_collapse = match s {
                    "separate" => BorderCollapse::Separate,
                    "collapse" => BorderCollapse::Collapse,
                    _ => return,
                };
            }
        }
        "border-spacing" => {
            let g = split_whitespace_groups(trimmed);
            if let Some(x) = g
                .first()
                .and_then(|t| parse_length_percentage_auto(t))
                .and_then(|l| l.resolve(ctx))
            {
                let y = g
                    .get(1)
                    .and_then(|t| parse_length_percentage_auto(t))
                    .and_then(|l| l.resolve(ctx))
                    .unwrap_or(x);
                style.border_spacing_x = x.max(0.0);
                style.border_spacing_y = y.max(0.0);
            }
        }
        "table-layout" => {
            if let Some(s) = ident.as_deref() {
                style.table_layout = match s {
                    "auto" => TableLayout::Auto,
                    "fixed" => TableLayout::Fixed,
                    _ => return,
                };
            }
        }
        "outline-width" => {
            if let Some(w) = parse_border_width_px(trimmed, ctx) {
                style.outline_width = w;
            }
        }
        "outline-style" => {
            if let Some(s) = ident.as_deref().and_then(parse_border_style_str) {
                style.outline_style = s;
            }
        }
        "outline-color" => {
            if let Some(c) = parse_color_from_tokens(trimmed, style.color) {
                style.outline_color = c;
            }
        }
        "outline-offset" => {
            if let Some(px) = parse_length_percentage_auto(trimmed).and_then(|l| l.resolve(ctx)) {
                style.outline_offset = px;
            }
        }
        "font-family" => {
            let fams: Vec<String> = split_commas(trimmed)
                .into_iter()
                .filter_map(|part| {
                    let s = serialize_tokens(trim_ws(part))
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string();
                    if s.is_empty() {
                        None
                    } else {
                        Some(s)
                    }
                })
                .collect();
            if !fams.is_empty() {
                style.font_family = fams;
            }
        }
        "font-weight" => {
            if let Some(s) = ident.as_deref() {
                style.font_weight = match s {
                    "normal" => 400,
                    "bold" => 700,
                    "bolder" => (parent.font_weight + 300).min(900),
                    "lighter" => parent.font_weight.saturating_sub(300).max(100),
                    _ => return,
                };
            } else if let Some(Token::Number { value, .. }) = trimmed.first() {
                style.font_weight = value.clamp(1.0, 1000.0).round() as u16;
            }
        }
        "font-style" => {
            if let Some(s) = ident.as_deref() {
                style.font_style = match s {
                    "normal" => FontStyle::Normal,
                    "italic" => FontStyle::Italic,
                    "oblique" => FontStyle::Oblique,
                    _ => return,
                };
            }
        }
        "line-height" => {
            if ident.as_deref() == Some("normal") {
                style.line_height = LineHeight::Normal;
            } else if trimmed.len() == 1 {
                match &trimmed[0] {
                    Token::Number { value, .. } if *value >= 0.0 => {
                        style.line_height = LineHeight::Number(*value);
                    }
                    Token::Percentage { value, .. } if *value >= 0.0 => {
                        style.line_height = LineHeight::Px(style.font_size * (*value / 100.0));
                    }
                    _ => {
                        if let Some(px) =
                            parse_length_percentage_auto(trimmed).and_then(|l| l.resolve(ctx))
                        {
                            style.line_height = LineHeight::Px(px.max(0.0));
                        }
                    }
                }
            } else if let Some(px) =
                parse_length_percentage_auto(trimmed).and_then(|l| l.resolve(ctx))
            {
                style.line_height = LineHeight::Px(px.max(0.0));
            }
        }
        "letter-spacing" => {
            if ident.as_deref() == Some("normal") {
                style.letter_spacing = 0.0;
            } else if let Some(px) =
                parse_length_percentage_auto(trimmed).and_then(|l| l.resolve(ctx))
            {
                style.letter_spacing = px;
            }
        }
        "word-spacing" => {
            if ident.as_deref() == Some("normal") {
                style.word_spacing = 0.0;
            } else if let Some(px) =
                parse_length_percentage_auto(trimmed).and_then(|l| l.resolve(ctx))
            {
                style.word_spacing = px;
            }
        }
        "text-align" => {
            if let Some(s) = ident.as_deref() {
                style.text_align = match s {
                    "start" => TextAlign::Start,
                    "end" => TextAlign::End,
                    "left" => TextAlign::Left,
                    "right" => TextAlign::Right,
                    "center" => TextAlign::Center,
                    "justify" => TextAlign::Justify,
                    _ => return,
                };
            }
        }
        "text-decoration-line" => {
            if let Some(s) = ident.as_deref() {
                style.text_decoration_line = match s {
                    "none" => TextDecorationLine::None,
                    "underline" => TextDecorationLine::Underline,
                    "overline" => TextDecorationLine::Overline,
                    "line-through" => TextDecorationLine::LineThrough,
                    _ => return,
                };
            }
        }
        "text-decoration-style" => {
            if let Some(s) = ident.as_deref() {
                style.text_decoration_style = match s {
                    "solid" => TextDecorationStyle::Solid,
                    "double" => TextDecorationStyle::Double,
                    "dotted" => TextDecorationStyle::Dotted,
                    "dashed" => TextDecorationStyle::Dashed,
                    "wavy" => TextDecorationStyle::Wavy,
                    _ => return,
                };
            }
        }
        "text-decoration-color" => {
            if let Some(c) = parse_color_from_tokens(trimmed, style.color) {
                style.text_decoration_color = Some(c);
            }
        }
        "text-transform" => {
            if let Some(s) = ident.as_deref() {
                style.text_transform = match s {
                    "none" => TextTransform::None,
                    "uppercase" => TextTransform::Uppercase,
                    "lowercase" => TextTransform::Lowercase,
                    "capitalize" => TextTransform::Capitalize,
                    _ => return,
                };
            }
        }
        "text-indent" => {
            if let Some(px) = parse_length_percentage_auto(trimmed).and_then(|l| l.resolve(ctx)) {
                style.text_indent = px;
            }
        }
        "text-overflow" => {
            if let Some(s) = ident.as_deref() {
                style.text_overflow = match s {
                    "clip" => TextOverflow::Clip,
                    "ellipsis" => TextOverflow::Ellipsis,
                    _ => return,
                };
            }
        }
        "white-space" => {
            if let Some(s) = ident.as_deref() {
                style.white_space = match s {
                    "normal" => WhiteSpace::Normal,
                    "nowrap" => WhiteSpace::Nowrap,
                    "pre" => WhiteSpace::Pre,
                    "pre-wrap" => WhiteSpace::PreWrap,
                    "pre-line" => WhiteSpace::PreLine,
                    "break-spaces" => WhiteSpace::BreakSpaces,
                    _ => return,
                };
            }
        }
        "word-break" => {
            if let Some(s) = ident.as_deref() {
                style.word_break = match s {
                    "normal" => WordBreak::Normal,
                    "break-all" => WordBreak::BreakAll,
                    "keep-all" => WordBreak::KeepAll,
                    "break-word" => WordBreak::BreakWord,
                    _ => return,
                };
            }
        }
        "overflow-wrap" | "word-wrap" => {
            if let Some(s) = ident.as_deref() {
                style.overflow_wrap = match s {
                    "normal" => OverflowWrap::Normal,
                    "break-word" => OverflowWrap::BreakWord,
                    "anywhere" => OverflowWrap::Anywhere,
                    _ => return,
                };
            }
        }
        "vertical-align" => {
            if let Some(s) = ident.as_deref() {
                style.vertical_align = match s {
                    "baseline" => VerticalAlign::Baseline,
                    "top" => VerticalAlign::Top,
                    "middle" => VerticalAlign::Middle,
                    "bottom" => VerticalAlign::Bottom,
                    "text-top" => VerticalAlign::TextTop,
                    "text-bottom" => VerticalAlign::TextBottom,
                    "sub" => VerticalAlign::Sub,
                    "super" => VerticalAlign::Super,
                    _ => return,
                };
            } else if let Some(lpa) = resolve_lpa_for_style(trimmed, ctx, false) {
                if let Some(px) = lpa.resolve(ctx) {
                    style.vertical_align = VerticalAlign::Length(px);
                }
            }
        }
        "background-color" => {
            if let Some(c) = parse_color_from_tokens(trimmed, style.color) {
                style.background_color = c;
            }
        }
        "background-image" => {
            if let Some(img) = parse_background_image(trimmed, style.color) {
                style.background_image = img;
            }
        }
        "mask-image" | "-webkit-mask-image" | "mask" | "-webkit-mask" => {
            if ident.as_deref() == Some("none") {
                style.mask_image = None;
            } else {
                for g in split_whitespace_groups(trimmed) {
                    if let Some(crate::values::BackgroundImage::Url(u)) = parse_background_image(g, style.color) {
                        style.mask_image = Some(u);
                        break;
                    }
                }
            }
        }
        "background-repeat" => {
            if let Some(s) = ident.as_deref() {
                style.background_repeat = match s {
                    "repeat" => BackgroundRepeat::Repeat,
                    "repeat-x" => BackgroundRepeat::RepeatX,
                    "repeat-y" => BackgroundRepeat::RepeatY,
                    "no-repeat" => BackgroundRepeat::NoRepeat,
                    "space" => BackgroundRepeat::Space,
                    "round" => BackgroundRepeat::Round,
                    _ => return,
                };
            }
        }
        "background-size" => {
            if let Some(s) = ident.as_deref() {
                style.background_size = match s {
                    "auto" => BackgroundSize::Auto,
                    "cover" => BackgroundSize::Cover,
                    "contain" => BackgroundSize::Contain,
                    _ => return,
                };
            } else {
                let g = split_whitespace_groups(trimmed);
                if let Some(w) = g.first().and_then(|t| resolve_lpa_for_style(t, ctx, true)) {
                    let h = g
                        .get(1)
                        .and_then(|t| resolve_lpa_for_style(t, ctx, true))
                        .unwrap_or(LengthPercentageAuto::Auto);
                    style.background_size = BackgroundSize::Explicit(w, h);
                }
            }
        }
        "background-clip" => {
            if let Some(s) = ident.as_deref() {
                style.background_clip = match s {
                    "border-box" => BackgroundClip::BorderBox,
                    "padding-box" => BackgroundClip::PaddingBox,
                    "content-box" => BackgroundClip::ContentBox,
                    "text" => BackgroundClip::Text,
                    _ => return,
                };
            }
        }
        "background-position" => {
            let g = split_whitespace_groups(trimmed);
            if let Some(first) = g.first() {
                let first_kw = single_ident_lower(first);
                let second_kw = g.get(1).and_then(|t| single_ident_lower(t));
                let first_is_y = matches!(first_kw.as_deref(), Some("top" | "bottom"));
                let second_is_x = matches!(second_kw.as_deref(), Some("left" | "right"));
                if g.len() == 1 && first_is_y {
                    style.background_position_x = LengthPercentageAuto::Percent(50.0);
                    style.background_position_y = parse_bg_pos_axis(first, ctx, false);
                } else if g.len() >= 2 && (first_is_y || second_is_x) {
                    style.background_position_x = parse_bg_pos_axis(g[1], ctx, true);
                    style.background_position_y = parse_bg_pos_axis(first, ctx, false);
                } else {
                    style.background_position_x = parse_bg_pos_axis(first, ctx, true);
                    style.background_position_y = g
                        .get(1)
                        .map(|t| parse_bg_pos_axis(t, ctx, false))
                        .unwrap_or(LengthPercentageAuto::Percent(50.0));
                }
            }
        }
        "background-attachment" => {
            if let Some(s) = ident.as_deref() {
                style.background_attachment_fixed = s == "fixed";
            }
        }
        "opacity" => match trimmed.first() {
            Some(Token::Number { value, .. }) => style.opacity = value.clamp(0.0, 1.0),
            Some(Token::Percentage { value, .. }) => style.opacity = (value / 100.0).clamp(0.0, 1.0),
            _ => {}
        },
        "visibility" => {
            if let Some(s) = ident.as_deref() {
                style.visibility = match s {
                    "visible" => Visibility::Visible,
                    "hidden" => Visibility::Hidden,
                    "collapse" => Visibility::Collapse,
                    _ => return,
                };
            }
        }
        "overflow-x" => {
            if let Some(o) = ident.as_deref().and_then(parse_overflow_kw) {
                style.overflow_x = o;
            }
        }
        "overflow-y" => {
            if let Some(o) = ident.as_deref().and_then(parse_overflow_kw) {
                style.overflow_y = o;
            }
        }
        "clip" => {
            if ident.as_deref() == Some("auto") {
                style.clip_hidden = false;
            } else {
                let raw = serialize_tokens(trimmed).to_ascii_lowercase();
                if let Some(inner) = raw.strip_prefix("rect(").and_then(|s| s.strip_suffix(')')) {
                    let vals: Vec<f32> = inner
                        .replace(',', " ")
                        .split_ascii_whitespace()
                        .filter_map(|t| t.trim_end_matches("px").parse::<f32>().ok())
                        .collect();
                    if vals.len() == 4 {
                        let w = vals[1] - vals[3];
                        let h = vals[2] - vals[0];
                        style.clip_hidden = w <= 1.0 && h <= 1.0;
                    }
                }
            }
        }
        "clip-path" => {
            if ident.as_deref() == Some("none") {
                style.clip_hidden = false;
            } else {
                let raw = serialize_tokens(trimmed).to_ascii_lowercase();
                if let Some(inner) = raw.strip_prefix("inset(").and_then(|s| s.strip_suffix(')')) {
                    let first = inner.split_ascii_whitespace().next().unwrap_or("");
                    if let Some(pct) = first.strip_suffix('%').and_then(|p| p.parse::<f32>().ok()) {
                        if pct >= 50.0 {
                            style.clip_hidden = true;
                        }
                    }
                }
            }
        }
        "box-shadow" => {
            style.box_shadow = parse_box_shadows(trimmed, ctx, style.color);
        }
        "text-shadow" => {
            style.text_shadow = parse_text_shadows(trimmed, ctx, style.color);
        }
        "transform" => {
            style.transform = parse_transforms(trimmed);
        }
        "transform-origin" => {
            let g = split_whitespace_groups(trimmed);
            if let Some(first) = g.first() {
                style.transform_origin_x = parse_bg_pos_axis(first, ctx, true);
                style.transform_origin_y = g
                    .get(1)
                    .map(|t| parse_bg_pos_axis(t, ctx, false))
                    .unwrap_or(LengthPercentageAuto::Percent(50.0));
            }
        }
        "filter" => {
            style.filter = parse_filters(trimmed, ctx, style.color);
        }
        "backdrop-filter" => {
            style.backdrop_filter = parse_filters(trimmed, ctx, style.color);
        }
        "object-fit" => {
            if let Some(s) = ident.as_deref() {
                style.object_fit = match s {
                    "fill" => ObjectFit::Fill,
                    "contain" => ObjectFit::Contain,
                    "cover" => ObjectFit::Cover,
                    "none" => ObjectFit::None,
                    "scale-down" => ObjectFit::ScaleDown,
                    _ => return,
                };
            }
        }
        "flex-direction" => {
            if let Some(s) = ident.as_deref() {
                style.flex_direction = match s {
                    "row" => FlexDirection::Row,
                    "row-reverse" => FlexDirection::RowReverse,
                    "column" => FlexDirection::Column,
                    "column-reverse" => FlexDirection::ColumnReverse,
                    _ => return,
                };
            }
        }
        "flex-wrap" => {
            if let Some(s) = ident.as_deref() {
                style.flex_wrap = match s {
                    "nowrap" => FlexWrap::Nowrap,
                    "wrap" => FlexWrap::Wrap,
                    "wrap-reverse" => FlexWrap::WrapReverse,
                    _ => return,
                };
            }
        }
        "flex-grow" => {
            if let Some(Token::Number { value, .. }) = trimmed.first() {
                if *value >= 0.0 {
                    style.flex_grow = *value;
                }
            }
        }
        "flex-shrink" => {
            if let Some(Token::Number { value, .. }) = trimmed.first() {
                if *value >= 0.0 {
                    style.flex_shrink = *value;
                }
            }
        }
        "flex-basis" => {
            if let Some(v) = resolve_lpa_for_style(trimmed, ctx, true) {
                style.flex_basis = v;
            }
        }
        "justify-content" => {
            if let Some(s) = ident.as_deref() {
                style.justify_content = match s {
                    "flex-start" => JustifyContent::FlexStart,
                    "flex-end" => JustifyContent::FlexEnd,
                    "center" => JustifyContent::Center,
                    "space-between" => JustifyContent::SpaceBetween,
                    "space-around" => JustifyContent::SpaceAround,
                    "space-evenly" => JustifyContent::SpaceEvenly,
                    "start" => JustifyContent::Start,
                    "end" => JustifyContent::End,
                    "stretch" => JustifyContent::Stretch,
                    _ => return,
                };
            }
        }
        "align-items" => {
            if let Some(a) = ident.as_deref().and_then(parse_align_items_kw) {
                style.align_items = a;
            }
        }
        "justify-items" => {
            if let Some(a) = ident.as_deref().and_then(parse_align_items_kw) {
                style.justify_items = a;
            }
        }
        "align-self" => {
            if let Some(a) = ident.as_deref().and_then(parse_align_self_kw) {
                style.align_self = a;
            }
        }
        "justify-self" => {
            if let Some(a) = ident.as_deref().and_then(parse_align_self_kw) {
                style.justify_self = a;
            }
        }
        "align-content" => {
            if let Some(s) = ident.as_deref() {
                style.align_content = match s {
                    "stretch" => AlignContent::Stretch,
                    "flex-start" => AlignContent::FlexStart,
                    "flex-end" => AlignContent::FlexEnd,
                    "center" => AlignContent::Center,
                    "space-between" => AlignContent::SpaceBetween,
                    "space-around" => AlignContent::SpaceAround,
                    "space-evenly" => AlignContent::SpaceEvenly,
                    "start" => AlignContent::Start,
                    "end" => AlignContent::End,
                    _ => return,
                };
            }
        }
        "order" => {
            if let Some(Token::Number { value, .. }) = trimmed.first() {
                style.order = value.round() as i32;
            }
        }
        "row-gap" => {
            if ident.as_deref() == Some("normal") {
                style.row_gap = LengthPercentageAuto::px(0.0);
            } else if let Some(v) = resolve_lpa_for_style(trimmed, ctx, false) {
                style.row_gap = v;
            }
        }
        "column-gap" => {
            if ident.as_deref() == Some("normal") {
                style.column_gap = LengthPercentageAuto::px(0.0);
            } else if let Some(v) = resolve_lpa_for_style(trimmed, ctx, false) {
                style.column_gap = v;
            }
        }
        "grid-template-columns" => {
            style.grid_template_columns = parse_grid_tracks(trimmed, ctx);
        }
        "grid-template-rows" => {
            style.grid_template_rows = parse_grid_tracks(trimmed, ctx);
        }
        "grid-template-areas" => {
            if ident.as_deref() == Some("none") {
                style.grid_template_areas.clear();
            } else {
                let mut rows = Vec::new();
                for t in trimmed {
                    if let Token::String(s) = t {
                        let cells: Vec<String> = s
                            .split_ascii_whitespace()
                            .map(|c| c.to_string())
                            .collect();
                        if !cells.is_empty() {
                            rows.push(cells);
                        }
                    }
                }
                if !rows.is_empty() {
                    style.grid_template_areas = rows;
                }
            }
        }
        "grid-area" => {
            if ident.as_deref() == Some("auto") {
                style.grid_area = None;
            } else if let [Token::Ident(s)] = trimmed {
                style.grid_area = Some(s.clone());
            }
        }
        "grid-auto-columns" => {
            style.grid_auto_columns = parse_grid_tracks(trimmed, ctx);
        }
        "grid-auto-rows" => {
            style.grid_auto_rows = parse_grid_tracks(trimmed, ctx);
        }
        "grid-auto-flow" => {
            let parts: Vec<String> = trimmed
                .iter()
                .filter_map(|t| match t {
                    Token::Ident(s) => Some(s.to_ascii_lowercase()),
                    _ => None,
                })
                .collect();
            let has_col = parts.iter().any(|p| p == "column");
            let has_dense = parts.iter().any(|p| p == "dense");
            style.grid_auto_flow = match (has_col, has_dense) {
                (false, false) => GridAutoFlow::Row,
                (false, true) => GridAutoFlow::RowDense,
                (true, false) => GridAutoFlow::Column,
                (true, true) => GridAutoFlow::ColumnDense,
            };
        }
        "grid-column-start" => {
            if let Some(p) = parse_grid_placement(trimmed) {
                style.grid_column_start = p;
            }
        }
        "grid-column-end" => {
            if let Some(p) = parse_grid_placement(trimmed) {
                style.grid_column_end = p;
            }
        }
        "grid-row-start" => {
            if let Some(p) = parse_grid_placement(trimmed) {
                style.grid_row_start = p;
            }
        }
        "grid-row-end" => {
            if let Some(p) = parse_grid_placement(trimmed) {
                style.grid_row_end = p;
            }
        }
        "list-style-type" => {
            if let Some(l) = ident.as_deref().and_then(parse_list_style_type) {
                style.list_style_type = l;
            }
        }
        "list-style-position" => {
            if let Some(s) = ident.as_deref() {
                style.list_style_position = match s {
                    "outside" => ListStylePosition::Outside,
                    "inside" => ListStylePosition::Inside,
                    _ => return,
                };
            }
        }
        "content" => {
            if matches!(ident.as_deref(), Some("none" | "normal")) {
                style.content = None;
            } else {
                let mut combined = String::new();
                let mut any = false;
                for t in trimmed {
                    if matches!(t, Token::Delim('/')) {
                        break;
                    }
                    if let Token::String(s) = t {
                        combined.push_str(s);
                        any = true;
                    }
                }
                if any {
                    style.content = Some(combined);
                }
            }
        }
        "cursor" => {
            if let Some(s) = ident.as_deref() {
                style.cursor = match s {
                    "auto" => Cursor::Auto,
                    "default" => Cursor::Default,
                    "none" => Cursor::None,
                    "context-menu" => Cursor::ContextMenu,
                    "help" => Cursor::Help,
                    "pointer" => Cursor::Pointer,
                    "progress" => Cursor::Progress,
                    "wait" => Cursor::Wait,
                    "cell" => Cursor::Cell,
                    "crosshair" => Cursor::Crosshair,
                    "text" => Cursor::Text,
                    "vertical-text" => Cursor::VerticalText,
                    "alias" => Cursor::Alias,
                    "copy" => Cursor::Copy,
                    "move" => Cursor::Move,
                    "no-drop" => Cursor::NoDrop,
                    "not-allowed" => Cursor::NotAllowed,
                    "grab" => Cursor::Grab,
                    "grabbing" => Cursor::Grabbing,
                    "e-resize" => Cursor::EResize,
                    "n-resize" => Cursor::NResize,
                    "ne-resize" => Cursor::NeResize,
                    "nw-resize" => Cursor::NwResize,
                    "s-resize" => Cursor::SResize,
                    "se-resize" => Cursor::SeResize,
                    "sw-resize" => Cursor::SwResize,
                    "w-resize" => Cursor::WResize,
                    "ew-resize" => Cursor::EwResize,
                    "ns-resize" => Cursor::NsResize,
                    "nesw-resize" => Cursor::NeswResize,
                    "nwse-resize" => Cursor::NwseResize,
                    "col-resize" => Cursor::ColResize,
                    "row-resize" => Cursor::RowResize,
                    "all-scroll" => Cursor::AllScroll,
                    "zoom-in" => Cursor::ZoomIn,
                    "zoom-out" => Cursor::ZoomOut,
                    _ => return,
                };
            }
        }
        "pointer-events" => {
            if let Some(s) = ident.as_deref() {
                style.pointer_events = match s {
                    "auto" => PointerEvents::Auto,
                    "none" => PointerEvents::None,
                    _ => return,
                };
            }
        }
        "user-select" => {
            if let Some(s) = ident.as_deref() {
                style.user_select = match s {
                    "auto" => UserSelect::Auto,
                    "none" => UserSelect::None,
                    "text" => UserSelect::Text,
                    "all" => UserSelect::All,
                    _ => return,
                };
            }
        }
        _ => {}
    }
}

fn parse_overflow_kw(s: &str) -> Option<Overflow> {
    Some(match s {
        "visible" => Overflow::Visible,
        "hidden" => Overflow::Hidden,
        "scroll" => Overflow::Scroll,
        "auto" => Overflow::Auto,
        "clip" => Overflow::Clip,
        _ => return None,
    })
}

fn parse_align_items_kw(s: &str) -> Option<AlignItems> {
    Some(match s {
        "stretch" => AlignItems::Stretch,
        "flex-start" => AlignItems::FlexStart,
        "flex-end" => AlignItems::FlexEnd,
        "center" => AlignItems::Center,
        "baseline" => AlignItems::Baseline,
        "start" => AlignItems::Start,
        "end" => AlignItems::End,
        _ => return None,
    })
}

fn parse_align_self_kw(s: &str) -> Option<AlignSelf> {
    Some(match s {
        "auto" => AlignSelf::Auto,
        "stretch" => AlignSelf::Stretch,
        "flex-start" => AlignSelf::FlexStart,
        "flex-end" => AlignSelf::FlexEnd,
        "center" => AlignSelf::Center,
        "baseline" => AlignSelf::Baseline,
        "start" => AlignSelf::Start,
        "end" => AlignSelf::End,
        _ => return None,
    })
}

fn parse_bg_pos_axis(tokens: &[Token], ctx: &CalcContext, is_x: bool) -> LengthPercentageAuto {
    if let Some(s) = single_ident_lower(tokens) {
        return match (s.as_str(), is_x) {
            ("left", _) | ("top", _) => LengthPercentageAuto::Percent(0.0),
            ("center", _) => LengthPercentageAuto::Percent(50.0),
            ("right", _) | ("bottom", _) => LengthPercentageAuto::Percent(100.0),
            _ => LengthPercentageAuto::Percent(0.0),
        };
    }
    resolve_lpa_for_style(tokens, ctx, false).unwrap_or(LengthPercentageAuto::Percent(0.0))
}

fn copy_property(dst: &mut ComputedStyle, src: &ComputedStyle, prop: &str) {
    match prop {
        "display" => dst.display = src.display,
        "position" => dst.position = src.position,
        "box-sizing" => dst.box_sizing = src.box_sizing,
        "float" => dst.float = src.float,
        "clear" => dst.clear = src.clear,
        "z-index" => dst.z_index = src.z_index,
        "width" => dst.width = src.width.clone(),
        "height" => dst.height = src.height.clone(),
        "min-width" => dst.min_width = src.min_width.clone(),
        "max-width" => dst.max_width = src.max_width.clone(),
        "min-height" => dst.min_height = src.min_height.clone(),
        "max-height" => dst.max_height = src.max_height.clone(),
        "aspect-ratio" => dst.aspect_ratio = src.aspect_ratio,
        "top" => dst.top = src.top.clone(),
        "right" => dst.right = src.right.clone(),
        "bottom" => dst.bottom = src.bottom.clone(),
        "left" => dst.left = src.left.clone(),
        "margin-top" => dst.margin_top = src.margin_top.clone(),
        "margin-right" => dst.margin_right = src.margin_right.clone(),
        "margin-bottom" => dst.margin_bottom = src.margin_bottom.clone(),
        "margin-left" => dst.margin_left = src.margin_left.clone(),
        "padding-top" => dst.padding_top = src.padding_top.clone(),
        "padding-right" => dst.padding_right = src.padding_right.clone(),
        "padding-bottom" => dst.padding_bottom = src.padding_bottom.clone(),
        "padding-left" => dst.padding_left = src.padding_left.clone(),
        "border-top-width" => dst.border_top_width = src.border_top_width,
        "border-right-width" => dst.border_right_width = src.border_right_width,
        "border-bottom-width" => dst.border_bottom_width = src.border_bottom_width,
        "border-left-width" => dst.border_left_width = src.border_left_width,
        "border-top-style" => dst.border_top_style = src.border_top_style,
        "border-right-style" => dst.border_right_style = src.border_right_style,
        "border-bottom-style" => dst.border_bottom_style = src.border_bottom_style,
        "border-left-style" => dst.border_left_style = src.border_left_style,
        "border-top-color" => dst.border_top_color = src.border_top_color,
        "border-right-color" => dst.border_right_color = src.border_right_color,
        "border-bottom-color" => dst.border_bottom_color = src.border_bottom_color,
        "border-left-color" => dst.border_left_color = src.border_left_color,
        "border-top-left-radius" => dst.border_top_left_radius = src.border_top_left_radius.clone(),
        "border-top-right-radius" => {
            dst.border_top_right_radius = src.border_top_right_radius.clone()
        }
        "border-bottom-right-radius" => {
            dst.border_bottom_right_radius = src.border_bottom_right_radius.clone()
        }
        "border-bottom-left-radius" => {
            dst.border_bottom_left_radius = src.border_bottom_left_radius.clone()
        }
        "border-collapse" => dst.border_collapse = src.border_collapse,
        "border-spacing" => {
            dst.border_spacing_x = src.border_spacing_x;
            dst.border_spacing_y = src.border_spacing_y;
        }
        "table-layout" => dst.table_layout = src.table_layout,
        "outline-width" => dst.outline_width = src.outline_width,
        "outline-style" => dst.outline_style = src.outline_style,
        "outline-color" => dst.outline_color = src.outline_color,
        "outline-offset" => dst.outline_offset = src.outline_offset,
        "color" => dst.color = src.color,
        "font-family" => dst.font_family = src.font_family.clone(),
        "font-size" => dst.font_size = src.font_size,
        "font-weight" => dst.font_weight = src.font_weight,
        "font-style" => dst.font_style = src.font_style,
        "line-height" => dst.line_height = src.line_height,
        "letter-spacing" => dst.letter_spacing = src.letter_spacing,
        "word-spacing" => dst.word_spacing = src.word_spacing,
        "text-align" => dst.text_align = src.text_align,
        "text-decoration-line" => dst.text_decoration_line = src.text_decoration_line,
        "text-decoration-color" => dst.text_decoration_color = src.text_decoration_color,
        "text-decoration-style" => dst.text_decoration_style = src.text_decoration_style,
        "text-transform" => dst.text_transform = src.text_transform,
        "text-indent" => dst.text_indent = src.text_indent,
        "text-overflow" => dst.text_overflow = src.text_overflow,
        "white-space" => dst.white_space = src.white_space,
        "word-break" => dst.word_break = src.word_break,
        "overflow-wrap" => dst.overflow_wrap = src.overflow_wrap,
        "vertical-align" => dst.vertical_align = src.vertical_align,
        "background-color" => dst.background_color = src.background_color,
        "background-image" => dst.background_image = src.background_image.clone(),
        "mask-image" | "-webkit-mask-image" => dst.mask_image = src.mask_image.clone(),
        "background-position" => {
            dst.background_position_x = src.background_position_x.clone();
            dst.background_position_y = src.background_position_y.clone();
        }
        "background-size" => dst.background_size = src.background_size.clone(),
        "background-repeat" => dst.background_repeat = src.background_repeat,
        "background-clip" => dst.background_clip = src.background_clip,
        "opacity" => dst.opacity = src.opacity,
        "visibility" => dst.visibility = src.visibility,
        "overflow-x" => dst.overflow_x = src.overflow_x,
        "overflow-y" => dst.overflow_y = src.overflow_y,
        "clip" | "clip-path" => dst.clip_hidden = src.clip_hidden,
        "box-shadow" => dst.box_shadow = src.box_shadow.clone(),
        "text-shadow" => dst.text_shadow = src.text_shadow.clone(),
        "transform" => dst.transform = src.transform.clone(),
        "filter" => dst.filter = src.filter.clone(),
        "backdrop-filter" => dst.backdrop_filter = src.backdrop_filter.clone(),
        "object-fit" => dst.object_fit = src.object_fit,
        "flex-direction" => dst.flex_direction = src.flex_direction,
        "flex-wrap" => dst.flex_wrap = src.flex_wrap,
        "flex-grow" => dst.flex_grow = src.flex_grow,
        "flex-shrink" => dst.flex_shrink = src.flex_shrink,
        "flex-basis" => dst.flex_basis = src.flex_basis.clone(),
        "justify-content" => dst.justify_content = src.justify_content,
        "align-items" => dst.align_items = src.align_items,
        "align-self" => dst.align_self = src.align_self,
        "align-content" => dst.align_content = src.align_content,
        "justify-items" => dst.justify_items = src.justify_items,
        "justify-self" => dst.justify_self = src.justify_self,
        "order" => dst.order = src.order,
        "row-gap" => dst.row_gap = src.row_gap.clone(),
        "column-gap" => dst.column_gap = src.column_gap.clone(),
        "grid-template-columns" => dst.grid_template_columns = src.grid_template_columns.clone(),
        "grid-template-rows" => dst.grid_template_rows = src.grid_template_rows.clone(),
        "grid-template-areas" => dst.grid_template_areas = src.grid_template_areas.clone(),
        "grid-auto-columns" => dst.grid_auto_columns = src.grid_auto_columns.clone(),
        "grid-auto-rows" => dst.grid_auto_rows = src.grid_auto_rows.clone(),
        "grid-auto-flow" => dst.grid_auto_flow = src.grid_auto_flow,
        "grid-area" => dst.grid_area = src.grid_area.clone(),
        "grid-column-start" => dst.grid_column_start = src.grid_column_start,
        "grid-column-end" => dst.grid_column_end = src.grid_column_end,
        "grid-row-start" => dst.grid_row_start = src.grid_row_start,
        "grid-row-end" => dst.grid_row_end = src.grid_row_end,
        "list-style-type" => dst.list_style_type = src.list_style_type,
        "list-style-position" => dst.list_style_position = src.list_style_position,
        "content" => dst.content = src.content.clone(),
        "cursor" => dst.cursor = src.cursor,
        "pointer-events" => dst.pointer_events = src.pointer_events,
        "user-select" => dst.user_select = src.user_select,
        _ => {}
    }
}

fn presentational_declarations(el: &ElementData) -> Vec<Declaration> {
    if !el.is_html() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let push_decl = |list: &mut Vec<Declaration>, name: &str, val: &str| {
        list.push(Declaration {
            name: name.to_string(),
            value: tokenize(val),
            important: false,
        });
    };

    if let Some(w) = el.get_attr("width") {
        if let Some(css_len) = html_dimension_to_css(w) {
            push_decl(&mut out, "width", &css_len);
        }
    }
    if let Some(h) = el.get_attr("height") {
        if let Some(css_len) = html_dimension_to_css(h) {
            push_decl(&mut out, "height", &css_len);
        }
    }
    if let Some(bg) = el.get_attr("bgcolor") {
        let trimmed = bg.trim();
        if !trimmed.is_empty() {
            push_decl(&mut out, "background-color", trimmed);
        }
    }
    if let Some(c) = el.get_attr("color") {
        let trimmed = c.trim();
        if !trimmed.is_empty() {
            push_decl(&mut out, "color", trimmed);
        }
    }
    if let Some(align) = el.get_attr("align") {
        let a = align.trim().to_ascii_lowercase();
        if el.tag_is("table") {
            match a.as_str() {
                "center" | "middle" => {
                    push_decl(&mut out, "margin-left", "auto");
                    push_decl(&mut out, "margin-right", "auto");
                }
                "left" => push_decl(&mut out, "float", "left"),
                "right" => push_decl(&mut out, "float", "right"),
                _ => {}
            }
        } else {
            match a.as_str() {
                "left" => push_decl(&mut out, "text-align", "left"),
                "right" => push_decl(&mut out, "text-align", "right"),
                "center" | "middle" => push_decl(&mut out, "text-align", "center"),
                "justify" => push_decl(&mut out, "text-align", "justify"),
                _ => {}
            }
        }
    }
    if let Some(valign) = el.get_attr("valign") {
        match valign.trim().to_ascii_lowercase().as_str() {
            "top" => push_decl(&mut out, "vertical-align", "top"),
            "middle" => push_decl(&mut out, "vertical-align", "middle"),
            "bottom" => push_decl(&mut out, "vertical-align", "bottom"),
            "baseline" => push_decl(&mut out, "vertical-align", "baseline"),
            _ => {}
        }
    }
    if let Some(border) = el.get_attr("border") {
        if let Ok(n) = border.trim().parse::<f32>() {
            if n > 0.0 {
                let s = format!("{}px solid #808080", n);
                push_decl(&mut out, "border", &s);
            } else {
                push_decl(&mut out, "border", "0px none");
            }
        }
    }
    if let Some(cp) = el.get_attr("cellpadding") {
        if let Ok(n) = cp.trim().parse::<f32>() {
            push_decl(&mut out, "padding", &format!("{}px", n.max(0.0)));
        }
    }
    if let Some(cs) = el.get_attr("cellspacing") {
        if let Ok(n) = cs.trim().parse::<f32>() {
            push_decl(&mut out, "border-spacing", &format!("{}px", n.max(0.0)));
        }
    }
    if el.tag_is("font") {
        if let Some(size) = el.get_attr("size") {
            let px = match size.trim() {
                "1" => Some("10px"),
                "2" => Some("13px"),
                "3" => Some("16px"),
                "4" => Some("18px"),
                "5" => Some("24px"),
                "6" => Some("32px"),
                "7" => Some("48px"),
                _ => None,
            };
            if let Some(v) = px {
                push_decl(&mut out, "font-size", v);
            }
        }
        if let Some(face) = el.get_attr("face") {
            if !face.trim().is_empty() {
                push_decl(&mut out, "font-family", face.trim());
            }
        }
    }
    if el.has_attr("nowrap") {
        push_decl(&mut out, "white-space", "nowrap");
    }

    out
}

fn html_dimension_to_css(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if s.ends_with('%') {
        if s[..s.len() - 1].trim().parse::<f32>().is_ok() {
            return Some(s.to_string());
        }
        return None;
    }
    if s.ends_with("px") || s.ends_with("em") || s.ends_with("rem") {
        return Some(s.to_string());
    }
    if let Ok(n) = s.parse::<f32>() {
        return Some(format!("{}px", n.max(0.0)));
    }
    None
}
