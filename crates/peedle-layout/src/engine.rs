use std::collections::HashMap;

use peedle_css::{
    BoxSizing, CalcContext, Clear, ComputedStyle, Display, Float, LengthPercentageAuto,
    ListStyleType, Position, StyleMap, TextAlign, TextTransform, WhiteSpace,
};
use peedle_html::{Document, Namespace, NodeData, NodeId};

use crate::box_tree::{BoxKind, LayoutBoxId, LayoutTree, ReplacedKind, TextFragment};
use crate::flex::layout_flex_container;
use crate::geometry::{EdgeSizes, Rect};
use crate::grid::layout_grid_container;
use crate::table::layout_table;
use crate::text::TextMeasurer;

#[derive(Clone, Copy, Debug)]
pub enum SizeConstraint {
    Available(f32),
    DefiniteBorderBox(f32),
}

#[derive(Clone, Copy, Debug)]
struct FloatRect {
    side: Float,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Default)]
struct FloatContext {
    floats: Vec<FloatRect>,
}

impl FloatContext {
    fn available_span(&self, y: f32, line_h: f32, left_bound: f32, right_bound: f32) -> (f32, f32) {
        let mut left = left_bound;
        let mut right = right_bound;
        let y_end = y + line_h.max(1.0);
        for f in &self.floats {
            if f.y + f.height > y + 0.01 && f.y < y_end - 0.01 {
                match f.side {
                    Float::Left => left = left.max(f.x + f.width),
                    Float::Right => right = right.min(f.x),
                    Float::None => {}
                }
            }
        }
        (left, right.max(left))
    }

    fn clear_y(&self, clear: Clear, current_y: f32) -> f32 {
        let mut y = current_y;
        for f in &self.floats {
            let matches = match clear {
                Clear::Left => f.side == Float::Left,
                Clear::Right => f.side == Float::Right,
                Clear::Both => f.side != Float::None,
                Clear::None => false,
            };
            if matches {
                y = y.max(f.y + f.height);
            }
        }
        y
    }

    fn place_float(
        &self,
        side: Float,
        start_y: f32,
        width: f32,
        height: f32,
        left_bound: f32,
        right_bound: f32,
    ) -> (f32, f32) {
        let mut y = start_y;
        let check_h = height.max(1.0);
        for _ in 0..64 {
            let (fl, fr) = self.available_span(y, check_h, left_bound, right_bound);
            if fr - fl >= width - 0.5 || (fl <= left_bound + 0.01 && fr >= right_bound - 0.01) {
                let x = match side {
                    Float::Left => fl,
                    Float::Right => (fr - width).max(left_bound),
                    Float::None => left_bound,
                };
                return (x, y);
            }
            let mut next_y = f32::INFINITY;
            for f in &self.floats {
                let fb = f.y + f.height;
                if fb > y + 0.01 && f.y < y + check_h - 0.01 {
                    next_y = next_y.min(fb);
                }
            }
            if next_y.is_finite() && next_y > y + 0.01 {
                y = next_y;
            } else {
                break;
            }
        }
        let (fl, fr) = self.available_span(y, check_h, left_bound, right_bound);
        let x = match side {
            Float::Left => fl,
            Float::Right => (fr - width).max(left_bound),
            Float::None => left_bound,
        };
        (x, y)
    }

    fn max_bottom(&self) -> f32 {
        self.floats
            .iter()
            .map(|f| f.y + f.height)
            .fold(0.0f32, f32::max)
    }
}

pub struct LayoutEngine {
    pub tree: LayoutTree,
    pub measurer: TextMeasurer,
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub root_font_size: f32,
    float_ctx: FloatContext,
    positioned_cb_stack: Vec<LayoutBoxId>,
}

pub fn layout_document(
    doc: &Document,
    styles: &StyleMap,
    image_sizes: &HashMap<String, (u32, u32)>,
    viewport_w: f32,
    viewport_h: f32,
) -> LayoutTree {
    let root_font_size = doc
        .document_element()
        .and_then(|id| styles.get(id))
        .map(|s| s.font_size)
        .unwrap_or(16.0);

    let mut engine = LayoutEngine {
        tree: LayoutTree {
            boxes: Vec::new(),
            root: None,
            node_to_box: HashMap::new(),
            viewport_width: viewport_w,
            viewport_height: viewport_h,
            content_width: viewport_w,
            content_height: viewport_h,
        },
        measurer: TextMeasurer::new(),
        viewport_w,
        viewport_h,
        root_font_size,
        float_ctx: FloatContext::default(),
        positioned_cb_stack: Vec::new(),
    };

    if let Some(root_el) = doc.document_element() {
        let built = engine.build_box(doc, styles, image_sizes, root_el, 0);
        if let Some(root_box) = built.into_iter().next() {
            engine.tree.root = Some(root_box);
            engine.layout_box(
                root_box,
                viewport_w,
                viewport_h,
                0.0,
                0.0,
                SizeConstraint::Available(viewport_w),
            );
            let bb = engine.tree.get(root_box).dims.margin_box();
            engine.tree.content_width = bb.right().max(viewport_w);
            engine.tree.content_height = bb.bottom().max(viewport_h);
        }
    }

    engine.tree
}

impl LayoutEngine {
    pub fn calc_ctx(&self, style: &ComputedStyle, percentage_base: f32) -> CalcContext {
        CalcContext {
            font_size: style.font_size,
            root_font_size: self.root_font_size,
            viewport_width: self.viewport_w,
            viewport_height: self.viewport_h,
            percentage_base,
        }
    }

    fn build_box(
        &mut self,
        doc: &Document,
        styles: &StyleMap,
        image_sizes: &HashMap<String, (u32, u32)>,
        node_id: NodeId,
        list_index: usize,
    ) -> Vec<LayoutBoxId> {
        match &doc.node(node_id).data {
            NodeData::Text(text) => {
                if text.is_empty() {
                    return Vec::new();
                }
                let parent_style = doc
                    .node(node_id)
                    .parent
                    .and_then(|p| styles.get(p))
                    .cloned()
                    .unwrap_or_default();
                let text_style = inherited_only_style(&parent_style, Display::Inline);
                let id = self.tree.alloc(Some(node_id), BoxKind::Text(text.clone()), text_style);
                vec![id]
            }
            NodeData::Element(el) => {
                let style = styles.get(node_id).cloned().unwrap_or_default();
                if style.display == Display::None {
                    return Vec::new();
                }
                if style.display == Display::Contents {
                    let mut out = Vec::new();
                    let mut li_idx = 0usize;
                    for child in doc.children(node_id) {
                        if doc.element(child).map_or(false, |e| e.tag_is("li")) {
                            li_idx += 1;
                        }
                        out.extend(self.build_box(doc, styles, image_sizes, child, li_idx));
                    }
                    return out;
                }

                if let Some(replaced) = detect_replaced(doc, node_id, el, image_sizes) {
                    let id = self
                        .tree
                        .alloc(Some(node_id), BoxKind::Replaced(replaced), style);
                    return vec![id];
                }

                let kind = match style.display {
                    Display::Flex | Display::InlineFlex => BoxKind::Flex,
                    Display::Grid | Display::InlineGrid => BoxKind::Grid,
                    Display::Table | Display::InlineTable => BoxKind::Table,
                    Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup => {
                        BoxKind::TableRowGroup
                    }
                    Display::TableRow => BoxKind::TableRow,
                    Display::TableCell => {
                        let colspan = el
                            .get_attr("colspan")
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(1)
                            .max(1);
                        let rowspan = el
                            .get_attr("rowspan")
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(1)
                            .max(1);
                        BoxKind::TableCell { colspan, rowspan }
                    }
                    Display::Inline => BoxKind::Inline,
                    Display::InlineBlock => BoxKind::InlineBlock,
                    _ => BoxKind::Block,
                };

                let box_id = self.tree.alloc(Some(node_id), kind.clone(), style.clone());
                if style.display == Display::ListItem && style.list_style_type != ListStyleType::None {
                    let marker = match style.list_style_type {
                        ListStyleType::Disc => "\u{2022} ".to_string(),
                        ListStyleType::Circle => "\u{25E6} ".to_string(),
                        ListStyleType::Square => "\u{25AA} ".to_string(),
                        ListStyleType::Decimal => format!("{}. ", list_index.max(1)),
                        ListStyleType::LowerAlpha => {
                            let c = (b'a' + ((list_index.saturating_sub(1)) % 26) as u8) as char;
                            format!("{}. ", c)
                        }
                        ListStyleType::UpperAlpha => {
                            let c = (b'A' + ((list_index.saturating_sub(1)) % 26) as u8) as char;
                            format!("{}. ", c)
                        }
                        _ => "\u{2022} ".to_string(),
                    };
                    self.tree.get_mut(box_id).list_marker = Some(marker);
                }

                let mut raw_children = Vec::new();

                if let Some(before_style) = styles.before(node_id) {
                    if before_style.display != Display::None {
                        if let Some(ref content) = before_style.content {
                            let pkind = match before_style.display {
                                Display::Block | Display::FlowRoot | Display::ListItem => BoxKind::Block,
                                Display::InlineBlock => BoxKind::InlineBlock,
                                Display::Flex | Display::InlineFlex => BoxKind::Flex,
                                Display::Grid | Display::InlineGrid => BoxKind::Grid,
                                _ => BoxKind::Inline,
                            };
                            let pbox = self.tree.alloc(None, pkind, before_style.clone());
                            if !content.is_empty() {
                                let tstyle = inherited_only_style(before_style, Display::Inline);
                                let tbox = self.tree.alloc(
                                    None,
                                    BoxKind::Text(content.clone()),
                                    tstyle,
                                );
                                self.tree.get_mut(pbox).children.push(tbox);
                            }
                            raw_children.push(pbox);
                        }
                    }
                }

                let child_source = el.shadow_root.unwrap_or(node_id);
                let skip_fallback_children = el.is_html() && el.tag_is("iframe");
                if !skip_fallback_children {
                    let mut li_counter = 0usize;
                    for child in doc.children(child_source) {
                        if doc.element(child).map_or(false, |e| e.tag_is("li")) {
                            li_counter += 1;
                        }
                        raw_children.extend(self.build_box(doc, styles, image_sizes, child, li_counter));
                    }
                }

                if let Some(after_style) = styles.after(node_id) {
                    if after_style.display != Display::None {
                        if let Some(ref content) = after_style.content {
                            let pkind = match after_style.display {
                                Display::Block | Display::FlowRoot | Display::ListItem => BoxKind::Block,
                                Display::InlineBlock => BoxKind::InlineBlock,
                                Display::Flex | Display::InlineFlex => BoxKind::Flex,
                                Display::Grid | Display::InlineGrid => BoxKind::Grid,
                                _ => BoxKind::Inline,
                            };
                            let pbox = self.tree.alloc(None, pkind, after_style.clone());
                            if !content.is_empty() {
                                let tstyle = inherited_only_style(after_style, Display::Inline);
                                let tbox = self.tree.alloc(
                                    None,
                                    BoxKind::Text(content.clone()),
                                    tstyle,
                                );
                                self.tree.get_mut(pbox).children.push(tbox);
                            }
                            raw_children.push(pbox);
                        }
                    }
                }

                let final_children = if matches!(
                    kind,
                    BoxKind::Block | BoxKind::InlineBlock | BoxKind::TableCell { .. }
                ) {
                    self.normalize_block_children(raw_children, &style)
                } else if kind == BoxKind::Flex || kind == BoxKind::Grid {
                    let mut wrapped = Vec::new();
                    for cid in raw_children {
                        if let BoxKind::Text(ref t) = self.tree.get(cid).kind {
                            if t.trim().is_empty() {
                                continue;
                            }
                            let anon_style = inherited_only_style(&style, Display::Block);
                            let anon = self.tree.alloc(None, BoxKind::AnonymousBlock, anon_style);
                            self.tree.get_mut(anon).children.push(cid);
                            wrapped.push(anon);
                        } else {
                            wrapped.push(cid);
                        }
                    }
                    wrapped
                } else {
                    raw_children
                };

                self.tree.get_mut(box_id).children = final_children;
                vec![box_id]
            }
            _ => Vec::new(),
        }
    }

    fn normalize_block_children(
        &mut self,
        children: Vec<LayoutBoxId>,
        parent_style: &ComputedStyle,
    ) -> Vec<LayoutBoxId> {
        let has_block = children.iter().any(|&cid| {
            let cb = self.tree.get(cid);
            cb.style.float == Float::None && self.is_block_level(cid)
        });
        if !has_block {
            return children;
        }
        let mut out = Vec::new();
        let mut inline_run: Vec<LayoutBoxId> = Vec::new();
        for cid in children {
            if self.is_block_level(cid) {
                self.flush_inline_run(&mut inline_run, parent_style, &mut out);
                out.push(cid);
            } else {
                inline_run.push(cid);
            }
        }
        self.flush_inline_run(&mut inline_run, parent_style, &mut out);
        out
    }

    fn flush_inline_run(
        &mut self,
        run: &mut Vec<LayoutBoxId>,
        parent_style: &ComputedStyle,
        out: &mut Vec<LayoutBoxId>,
    ) {
        if run.is_empty() {
            return;
        }
        let all_ws = run.iter().all(|&cid| match &self.tree.get(cid).kind {
            BoxKind::Text(t) => t.chars().all(|c| c.is_ascii_whitespace()),
            _ => false,
        });
        if all_ws {
            run.clear();
            return;
        }
        let anon_style = inherited_only_style(parent_style, Display::Block);
        let anon_id = self.tree.alloc(None, BoxKind::AnonymousBlock, anon_style);
        self.tree.get_mut(anon_id).children = std::mem::take(run);
        out.push(anon_id);
    }

    fn is_block_level(&self, id: LayoutBoxId) -> bool {
        let b = self.tree.get(id);
        if matches!(b.style.position, Position::Absolute | Position::Fixed) {
            return true;
        }
        if b.style.float != Float::None {
            return true;
        }
        match b.kind {
            BoxKind::Block
            | BoxKind::Flex
            | BoxKind::Grid
            | BoxKind::Table
            | BoxKind::AnonymousBlock => !matches!(
                b.style.display,
                Display::Inline | Display::InlineBlock | Display::InlineFlex | Display::InlineGrid | Display::InlineTable
            ),
            BoxKind::Replaced(ReplacedKind::Hr) => true,
            _ => false,
        }
    }

    pub fn resolve_explicit_height(&self, style: &ComputedStyle, cb_height: f32) -> Option<f32> {
        if cb_height < 0.0 && matches!(style.height, LengthPercentageAuto::Percent(_)) {
            return None;
        }
        let ctx = self.calc_ctx(style, cb_height.max(0.0));
        let h = style.height.resolve(&ctx)?;
        if style.box_sizing == BoxSizing::BorderBox {
            let pv = style.padding_top.resolve_or(&ctx, 0.0)
                + style.padding_bottom.resolve_or(&ctx, 0.0)
                + style.border_top_width
                + style.border_bottom_width;
            Some((h - pv).max(0.0))
        } else {
            Some(h.max(0.0))
        }
    }

    pub fn layout_box(
        &mut self,
        id: LayoutBoxId,
        cb_width: f32,
        cb_height: f32,
        origin_x: f32,
        origin_y: f32,
        width_constraint: SizeConstraint,
    ) {
        let style = self.tree.get(id).style.clone();
        let kind = self.tree.get(id).kind.clone();
        let ctx = self.calc_ctx(&style, cb_width);

        let padding = EdgeSizes {
            top: style.padding_top.resolve_or(&ctx, 0.0).max(0.0),
            right: style.padding_right.resolve_or(&ctx, 0.0).max(0.0),
            bottom: style.padding_bottom.resolve_or(&ctx, 0.0).max(0.0),
            left: style.padding_left.resolve_or(&ctx, 0.0).max(0.0),
        };
        let border = EdgeSizes {
            top: style.border_top_width.max(0.0),
            right: style.border_right_width.max(0.0),
            bottom: style.border_bottom_width.max(0.0),
            left: style.border_left_width.max(0.0),
        };
        let mut margin = EdgeSizes {
            top: style.margin_top.resolve_or(&ctx, 0.0),
            right: style.margin_right.resolve_or(&ctx, 0.0),
            bottom: style.margin_bottom.resolve_or(&ctx, 0.0),
            left: style.margin_left.resolve_or(&ctx, 0.0),
        };

        let pb_h = padding.horizontal() + border.horizontal();
        let pb_v = padding.vertical() + border.vertical();

        let content_w = match width_constraint {
            SizeConstraint::DefiniteBorderBox(bb_w) => (bb_w - pb_h).max(0.0),
            SizeConstraint::Available(avail_w) => {
                if let BoxKind::Replaced(ref rep) = kind {
                    let (rw, _) = self.resolve_replaced_size(rep, &style, cb_width, cb_height);
                    rw
                } else {
                    let raw_w = style.width.resolve(&ctx);
                    let mut cw = if let Some(w) = raw_w {
                        if style.box_sizing == BoxSizing::BorderBox {
                            (w - pb_h).max(0.0)
                        } else {
                            w.max(0.0)
                        }
                    } else if matches!(
                        kind,
                        BoxKind::InlineBlock | BoxKind::Table
                    ) || style.float != Float::None
                    {
                        self.measure_intrinsic_width(id, avail_w).min((avail_w - pb_h).max(0.0))
                    } else {
                        (avail_w - margin.horizontal() - pb_h).max(0.0)
                    };

                    if let Some(max_w) = style.max_width.resolve(&ctx) {
                        let max_cw = if style.box_sizing == BoxSizing::BorderBox {
                            (max_w - pb_h).max(0.0)
                        } else {
                            max_w.max(0.0)
                        };
                        cw = cw.min(max_cw);
                    }
                    if let Some(min_w) = style.min_width.resolve(&ctx) {
                        let min_cw = if style.box_sizing == BoxSizing::BorderBox {
                            (min_w - pb_h).max(0.0)
                        } else {
                            min_w.max(0.0)
                        };
                        cw = cw.max(min_cw);
                    }

                    let ml_auto = style.margin_left == LengthPercentageAuto::Auto;
                    let mr_auto = style.margin_right == LengthPercentageAuto::Auto;
                    let remaining = (avail_w - cw - pb_h).max(0.0);
                    if ml_auto && mr_auto {
                        margin.left = remaining * 0.5;
                        margin.right = remaining * 0.5;
                    } else if ml_auto {
                        margin.left = (remaining - margin.right).max(0.0);
                    } else if mr_auto {
                        margin.right = (remaining - margin.left).max(0.0);
                    }
                    cw
                }
            }
        };

        let content_x = origin_x + margin.left + border.left + padding.left;
        let content_y = origin_y + margin.top + border.top + padding.top;

        {
            let b = self.tree.get_mut(id);
            b.dims.padding = padding;
            b.dims.border = border;
            b.dims.margin = margin;
            b.dims.content = Rect::new(content_x, content_y, content_w, 0.0);
        }

        let is_positioned_cb = style.position != Position::Static;
        if is_positioned_cb {
            self.positioned_cb_stack.push(id);
        }

        match kind {
            BoxKind::Replaced(rep) => {
                let (rw, rh) = self.resolve_replaced_size(&rep, &style, cb_width, cb_height);
                let b = self.tree.get_mut(id);
                if matches!(width_constraint, SizeConstraint::Available(_)) {
                    b.dims.content.width = rw;
                }
                b.dims.content.height = rh;
            }
            BoxKind::Flex => {
                layout_flex_container(self, id, cb_width, cb_height);
            }
            BoxKind::Grid => {
                layout_grid_container(self, id, cb_width, cb_height);
            }
            BoxKind::Table => {
                layout_table(self, id, cb_width, cb_height);
            }
            _ => {
                let children = self.tree.get(id).children.clone();
                let has_block_children = children.iter().any(|&cid| {
                    let cb = self.tree.get(cid);
                    cb.style.float == Float::None && self.is_block_level(cid)
                });
                let explicit_h = self.resolve_explicit_height(&style, cb_height);
                let is_root = self.tree.root == Some(id);
                let child_cb_h = explicit_h.unwrap_or(if is_root { cb_height } else { -1.0 });
                let establishes_bfc = style.overflow_x != peedle_css::Overflow::Visible
                    || style.overflow_y != peedle_css::Overflow::Visible
                    || style.display == Display::FlowRoot
                    || style.display == Display::InlineBlock
                    || style.float != Float::None
                    || matches!(style.position, Position::Absolute | Position::Fixed);
                let saved_floats = if establishes_bfc {
                    Some(std::mem::take(&mut self.float_ctx))
                } else {
                    None
                };

                let mut content_h = if has_block_children {
                    self.layout_block_children(
                        id,
                        &children,
                        content_x,
                        content_y,
                        content_w,
                        child_cb_h,
                        explicit_h,
                    )
                } else {
                    self.layout_inline_children(id, &children, content_x, content_y, content_w, child_cb_h)
                };
                if establishes_bfc {
                    content_h = content_h.max((self.float_ctx.max_bottom() - content_y).max(0.0));
                }
                if let Some(prev) = saved_floats {
                    self.float_ctx = prev;
                }

                let mut final_h = explicit_h.unwrap_or(content_h);
                if explicit_h.is_none() {
                    if let Some(ar) = style.aspect_ratio {
                        if ar > 0.0 && content_w > 0.0 {
                            final_h = content_w / ar;
                        }
                    }
                }
                let ctx_h = self.calc_ctx(&style, cb_height.max(0.0));
                let max_h_opt = if cb_height < 0.0
                    && matches!(style.max_height, LengthPercentageAuto::Percent(_))
                {
                    None
                } else {
                    style.max_height.resolve(&ctx_h)
                };
                if let Some(max_h) = max_h_opt {
                    let xh = if style.box_sizing == BoxSizing::BorderBox {
                        (max_h - pb_v).max(0.0)
                    } else {
                        max_h
                    };
                    final_h = final_h.min(xh);
                }
                let min_h_opt = if cb_height < 0.0
                    && matches!(style.min_height, LengthPercentageAuto::Percent(_))
                {
                    None
                } else {
                    style.min_height.resolve(&ctx_h)
                };
                if let Some(min_h) = min_h_opt {
                    let mh = if style.box_sizing == BoxSizing::BorderBox {
                        (min_h - pb_v).max(0.0)
                    } else {
                        min_h
                    };
                    final_h = final_h.max(mh);
                }
                self.tree.get_mut(id).dims.content.height = final_h.max(0.0);
            }
        }

        if is_positioned_cb {
            self.positioned_cb_stack.pop();
        }
    }

    fn is_self_collapsing_empty(&self, id: LayoutBoxId, content_w: f32, cb_height: f32) -> bool {
        let b = self.tree.get(id);
        if !matches!(b.kind, BoxKind::Block | BoxKind::AnonymousBlock) {
            return false;
        }
        if matches!(b.style.position, Position::Absolute | Position::Fixed)
            || b.style.float != Float::None
            || b.style.clear != Clear::None
        {
            return false;
        }
        let ctx = self.calc_ctx(&b.style, content_w);
        let pad_v = b.style.padding_top.resolve_or(&ctx, 0.0)
            + b.style.padding_bottom.resolve_or(&ctx, 0.0);
        let bord_v = b.style.border_top_width + b.style.border_bottom_width;
        if pad_v > 0.0 || bord_v > 0.0 {
            return false;
        }
        if let Some(eh) = self.resolve_explicit_height(&b.style, cb_height) {
            if eh > 0.0 {
                return false;
            }
        }
        if let Some(mh) = b.style.min_height.resolve(&self.calc_ctx(&b.style, cb_height.max(0.0))) {
            if mh > 0.0 {
                return false;
            }
        }
        for &cid in &b.children {
            let ck = &self.tree.get(cid).kind;
            match ck {
                BoxKind::Text(t) if t.chars().all(|c| c.is_ascii_whitespace()) => {}
                _ => {
                    if !self.is_self_collapsing_empty(cid, content_w, -1.0) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn collect_empty_margins(&self, id: LayoutBoxId, content_w: f32, pos: &mut f32, neg: &mut f32) {
        let b = self.tree.get(id);
        let ctx = self.calc_ctx(&b.style, content_w);
        let mt = b.style.margin_top.resolve_or(&ctx, 0.0);
        let mb = b.style.margin_bottom.resolve_or(&ctx, 0.0);
        *pos = pos.max(mt.max(0.0)).max(mb.max(0.0));
        *neg = neg.min(mt.min(0.0)).min(mb.min(0.0));
        for &cid in &b.children {
            if matches!(self.tree.get(cid).kind, BoxKind::Block | BoxKind::AnonymousBlock) {
                self.collect_empty_margins(cid, content_w, pos, neg);
            }
        }
    }

    fn layout_block_children(
        &mut self,
        container_id: LayoutBoxId,
        children: &[LayoutBoxId],
        content_x: f32,
        content_y: f32,
        content_w: f32,
        cb_height: f32,
        explicit_h: Option<f32>,
    ) -> f32 {
        let container_box = self.tree.get(container_id);
        let container_establishes_bfc = container_box.style.overflow_x != peedle_css::Overflow::Visible
            || container_box.style.overflow_y != peedle_css::Overflow::Visible
            || container_box.style.display == Display::FlowRoot
            || container_box.style.display == Display::InlineBlock
            || container_box.style.float != Float::None
            || matches!(container_box.style.position, Position::Absolute | Position::Fixed)
            || matches!(container_box.kind, BoxKind::TableCell { .. } | BoxKind::Flex | BoxKind::Grid);
        let collapse_first_top = !container_establishes_bfc
            && self.tree.root != Some(container_id)
            && container_box.dims.border.top == 0.0
            && container_box.dims.padding.top == 0.0;
        let container_mt = container_box.dims.margin.top;

        let mut base_y = content_y;
        let mut pending_pos = 0.0f32;
        let mut pending_neg = 0.0f32;
        let mut has_prev = false;
        let mut abs_children = Vec::new();

        for &cid in children {
            let cs = self.tree.get(cid).style.clone();
            if matches!(cs.position, Position::Absolute | Position::Fixed) {
                abs_children.push(cid);
                continue;
            }
            if cs.float != Float::None {
                let mut float_y = if has_prev {
                    base_y + pending_pos + pending_neg
                } else {
                    base_y
                };
                if cs.clear != Clear::None {
                    float_y = self.float_ctx.clear_y(cs.clear, float_y);
                }
                self.layout_box(
                    cid,
                    content_w,
                    cb_height,
                    content_x,
                    float_y,
                    SizeConstraint::Available(content_w),
                );
                let mb = self.tree.get(cid).dims.margin_box();
                let (target_x, target_y) = self.float_ctx.place_float(
                    cs.float,
                    float_y,
                    mb.width,
                    mb.height,
                    content_x,
                    content_x + content_w,
                );
                self.translate_subtree(cid, target_x - mb.x, target_y - mb.y);
                let final_mb = self.tree.get(cid).dims.margin_box();
                self.float_ctx.floats.push(FloatRect {
                    side: cs.float,
                    x: final_mb.x,
                    y: final_mb.y,
                    width: final_mb.width,
                    height: final_mb.height,
                });
                continue;
            }

            if self.is_self_collapsing_empty(cid, content_w, cb_height) {
                self.collect_empty_margins(cid, content_w, &mut pending_pos, &mut pending_neg);
                let empty_y = base_y + pending_pos + pending_neg;
                self.layout_box(
                    cid,
                    content_w,
                    cb_height,
                    content_x,
                    empty_y,
                    SizeConstraint::Available(content_w),
                );
                has_prev = true;
                continue;
            }

            let cctx = self.calc_ctx(&cs, content_w);
            let mt = cs.margin_top.resolve_or(&cctx, 0.0);
            let comb_margin = if has_prev {
                pending_pos.max(mt.max(0.0)) + pending_neg.min(mt.min(0.0))
            } else if collapse_first_top && container_mt >= 0.0 && mt >= 0.0 && mt <= container_mt {
                0.0
            } else {
                mt
            };
            let mut border_top_y = base_y + comb_margin;
            if cs.clear != Clear::None {
                let float_bottom = self.float_ctx.clear_y(cs.clear, f32::NEG_INFINITY);
                if float_bottom > border_top_y {
                    border_top_y = float_bottom;
                }
            }
            let origin_y = border_top_y - mt;

            self.layout_box(
                cid,
                content_w,
                cb_height,
                content_x,
                origin_y,
                SizeConstraint::Available(content_w),
            );

            let bb = self.tree.get(cid).dims.border_box();
            base_y = bb.bottom();
            let mb = self.tree.get(cid).dims.margin.bottom;
            pending_pos = mb.max(0.0);
            pending_neg = mb.min(0.0);
            has_prev = true;

            self.apply_relative_offset(cid, content_w, cb_height);
        }

        let cursor_y = if has_prev {
            base_y + pending_pos + pending_neg
        } else {
            base_y
        };
        let total_h = (cursor_y - content_y).max(0.0);

        self.tree.get_mut(container_id).dims.content.height = explicit_h.unwrap_or(total_h);
        for cid in abs_children {
            self.layout_out_of_flow(cid, container_id);
        }
        total_h
    }

    fn layout_inline_children(
        &mut self,
        container_id: LayoutBoxId,
        children: &[LayoutBoxId],
        content_x: f32,
        content_y: f32,
        content_w: f32,
        cb_height: f32,
    ) -> f32 {
        let container_style = self.tree.get(container_id).style.clone();
        let mut flat: Vec<LayoutBoxId> = Vec::new();
        self.collect_inline_items(children, &mut flat);

        let base_line_h = container_style.line_height.resolve(container_style.font_size);
        let mut cur_y = content_y;
        let (mut line_left, mut line_right) =
            self.float_ctx
                .available_span(cur_y, base_line_h, content_x, content_x + content_w);
        let mut cur_x = line_left + container_style.text_indent;
        let mut line_h = base_line_h;
        let mut line_items: Vec<(LayoutBoxId, usize)> = Vec::new();
        let mut line_box_items: Vec<LayoutBoxId> = Vec::new();
        let mut line_has_content = false;

        let finish_line = |engine: &mut LayoutEngine,
                           text_refs: &mut Vec<(LayoutBoxId, usize)>,
                           box_refs: &mut Vec<LayoutBoxId>,
                           line_l: f32,
                           line_r: f32,
                           cur_end_x: f32,
                           line_top_y: f32,
                           cur_line_h: &mut f32,
                           align: TextAlign| {
            let used = (cur_end_x - line_l).max(0.0);
            let avail = (line_r - line_l).max(0.0);
            let shift = match align {
                TextAlign::Center => ((avail - used) * 0.5).max(0.0),
                TextAlign::Right | TextAlign::End => (avail - used).max(0.0),
                _ => 0.0,
            };
            if shift > 0.01 {
                for &(tid, fidx) in text_refs.iter() {
                    if let Some(frag) = engine.tree.get_mut(tid).text_fragments.get_mut(fidx) {
                        frag.x += shift;
                    }
                }
                for &bid in box_refs.iter() {
                    engine.translate_subtree(bid, shift, 0.0);
                }
            }
            let has_valign_offset = box_refs.iter().any(|&bid| {
                matches!(
                    engine.tree.get(bid).style.vertical_align,
                    peedle_css::VerticalAlign::Length(_)
                        | peedle_css::VerticalAlign::Bottom
                        | peedle_css::VerticalAlign::Middle
                )
            });
            if has_valign_offset && !box_refs.is_empty() {
                let baseline_y = line_top_y + *cur_line_h;
                let mut min_top = line_top_y;
                let mut max_bot = line_top_y + *cur_line_h;
                for &bid in box_refs.iter() {
                    let va = engine.tree.get(bid).style.vertical_align;
                    let mb = engine.tree.get(bid).dims.margin_box();
                    let target_y = match va {
                        peedle_css::VerticalAlign::Length(off) => baseline_y - off - mb.height,
                        peedle_css::VerticalAlign::Bottom => baseline_y - mb.height,
                        peedle_css::VerticalAlign::Middle => {
                            line_top_y + (*cur_line_h - mb.height) * 0.5
                        }
                        _ => mb.y,
                    };
                    let dy = target_y - mb.y;
                    if dy.abs() > 0.01 {
                        engine.translate_subtree(bid, 0.0, dy);
                    }
                    let new_mb = engine.tree.get(bid).dims.margin_box();
                    min_top = min_top.min(new_mb.y);
                    max_bot = max_bot.max(new_mb.bottom());
                }
                if min_top < line_top_y - 0.01 {
                    let down = line_top_y - min_top;
                    for &(tid, fidx) in text_refs.iter() {
                        if let Some(frag) = engine.tree.get_mut(tid).text_fragments.get_mut(fidx) {
                            frag.y += down;
                            frag.baseline += down;
                        }
                    }
                    for &bid in box_refs.iter() {
                        engine.translate_subtree(bid, 0.0, down);
                    }
                    max_bot += down;
                }
                *cur_line_h = (*cur_line_h).max(max_bot - line_top_y);
            }
            text_refs.clear();
            box_refs.clear();
        };

        for item_id in flat {
            let item_kind = self.tree.get(item_id).kind.clone();
            let item_style = self.tree.get(item_id).style.clone();
            if matches!(item_style.position, Position::Absolute | Position::Fixed) {
                self.layout_out_of_flow(item_id, container_id);
                continue;
            }
            if item_style.float != Float::None {
                self.layout_box(
                    item_id,
                    content_w,
                    cb_height,
                    content_x,
                    cur_y,
                    SizeConstraint::Available(content_w),
                );
                let mb = self.tree.get(item_id).dims.margin_box();
                let (target_x, target_y) = self.float_ctx.place_float(
                    item_style.float,
                    cur_y,
                    mb.width,
                    mb.height,
                    content_x,
                    content_x + content_w,
                );
                self.translate_subtree(item_id, target_x - mb.x, target_y - mb.y);
                let final_mb = self.tree.get(item_id).dims.margin_box();
                self.float_ctx.floats.push(FloatRect {
                    side: item_style.float,
                    x: final_mb.x,
                    y: final_mb.y,
                    width: final_mb.width,
                    height: final_mb.height,
                });
                let (nl, nr) = self.float_ctx.available_span(
                    cur_y,
                    line_h.max(base_line_h),
                    content_x,
                    content_x + content_w,
                );
                line_left = nl;
                line_right = nr;
                if cur_x < line_left {
                    cur_x = line_left;
                }
                continue;
            }
            match item_kind {
                BoxKind::Replaced(ReplacedKind::Br) => {
                    finish_line(
                        self,
                        &mut line_items,
                        &mut line_box_items,
                        line_left,
                        line_right,
                        cur_x,
                        cur_y,
                        &mut line_h,
                        container_style.text_align,
                    );
                    cur_y += line_h;
                    let (nl, nr) = self.float_ctx.available_span(
                        cur_y,
                        base_line_h,
                        content_x,
                        content_x + content_w,
                    );
                    line_left = nl;
                    line_right = nr;
                    cur_x = line_left;
                    line_h = base_line_h;
                    line_has_content = false;
                }
                BoxKind::Text(raw_text) => {
                    let transformed = apply_text_transform(&raw_text, item_style.text_transform);
                    let preserve_nl = matches!(
                        item_style.white_space,
                        WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::PreLine | WhiteSpace::BreakSpaces
                    );
                    let no_wrap = matches!(item_style.white_space, WhiteSpace::Nowrap | WhiteSpace::Pre);
                    let preserve_spaces = matches!(
                        item_style.white_space,
                        WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces
                    );

                    let lh = item_style.line_height.resolve(item_style.font_size);
                    let baseline_off = item_style.font_size * 0.82 + (lh - item_style.font_size) * 0.5;
                    let raw_tokens = tokenize_inline_text(&transformed, preserve_spaces, preserve_nl);
                    let break_long_words = matches!(
                        item_style.word_break,
                        peedle_css::WordBreak::BreakAll | peedle_css::WordBreak::BreakWord
                    ) || matches!(
                        item_style.overflow_wrap,
                        peedle_css::OverflowWrap::BreakWord | peedle_css::OverflowWrap::Anywhere
                    );
                    let max_line_w = (line_right - line_left).max(16.0);
                    let mut tokens: Vec<String> = Vec::with_capacity(raw_tokens.len());
                    for t in raw_tokens {
                        if break_long_words && !no_wrap && t != " " && t != "\n" && t.chars().count() > 1 {
                            let tw = self.measurer.measure_str(
                                &t,
                                &item_style.font_family,
                                item_style.font_size,
                                item_style.font_weight,
                            );
                            if tw > max_line_w {
                                let mut chunk = String::new();
                                for ch in t.chars() {
                                    let mut cand = chunk.clone();
                                    cand.push(ch);
                                    let cw = self.measurer.measure_str(
                                        &cand,
                                        &item_style.font_family,
                                        item_style.font_size,
                                        item_style.font_weight,
                                    );
                                    if !chunk.is_empty() && cw > max_line_w {
                                        tokens.push(std::mem::take(&mut chunk));
                                    }
                                    chunk.push(ch);
                                }
                                if !chunk.is_empty() {
                                    tokens.push(chunk);
                                }
                                continue;
                            }
                        }
                        tokens.push(t);
                    }

                    let use_ellipsis = no_wrap
                        && container_style.text_overflow == peedle_css::TextOverflow::Ellipsis
                        && container_style.overflow_x != peedle_css::Overflow::Visible;
                    let ellipsis_w = if use_ellipsis {
                        self.measurer.measure_str(
                            "...",
                            &item_style.font_family,
                            item_style.font_size,
                            item_style.font_weight,
                        )
                    } else {
                        0.0
                    };

                    self.tree.get_mut(item_id).text_fragments.clear();
                    for tok in tokens {
                        if tok == "\n" {
                            finish_line(
                                self,
                                &mut line_items,
                                &mut line_box_items,
                                line_left,
                                line_right,
                                cur_x,
                                cur_y,
                                &mut line_h,
                                container_style.text_align,
                            );
                            cur_y += line_h;
                            let (nl, nr) = self.float_ctx.available_span(
                                cur_y,
                                base_line_h,
                                content_x,
                                content_x + content_w,
                            );
                            line_left = nl;
                            line_right = nr;
                            cur_x = line_left;
                            line_h = base_line_h;
                            line_has_content = false;
                            continue;
                        }
                        if !preserve_spaces && tok == " " && !line_has_content {
                            continue;
                        }
                        let w = self.measurer.measure_str(
                            &tok,
                            &item_style.font_family,
                            item_style.font_size,
                            item_style.font_weight,
                        );
                        if use_ellipsis && cur_x + w > line_right - ellipsis_w {
                            let avail = (line_right - ellipsis_w - cur_x).max(0.0);
                            let mut kept = String::new();
                            for ch in tok.chars() {
                                let mut cand = kept.clone();
                                cand.push(ch);
                                let cw = self.measurer.measure_str(
                                    &cand,
                                    &item_style.font_family,
                                    item_style.font_size,
                                    item_style.font_weight,
                                );
                                if cw > avail {
                                    break;
                                }
                                kept.push(ch);
                            }
                            kept.push_str("...");
                            let kw = self.measurer.measure_str(
                                &kept,
                                &item_style.font_family,
                                item_style.font_size,
                                item_style.font_weight,
                            );
                            let frag = TextFragment {
                                text: kept,
                                x: cur_x,
                                y: cur_y,
                                width: kw,
                                height: lh,
                                baseline: cur_y + baseline_off,
                            };
                            let fidx = self.tree.get(item_id).text_fragments.len();
                            self.tree.get_mut(item_id).text_fragments.push(frag);
                            line_items.push((item_id, fidx));
                            cur_x += kw;
                            line_h = line_h.max(lh);
                            line_has_content = true;
                            break;
                        }
                        if !no_wrap && line_has_content && cur_x + w > line_right + 0.5 && tok != " " {
                            finish_line(
                                self,
                                &mut line_items,
                                &mut line_box_items,
                                line_left,
                                line_right,
                                cur_x,
                                cur_y,
                                &mut line_h,
                                container_style.text_align,
                            );
                            cur_y += line_h;
                            let (nl, nr) = self.float_ctx.available_span(
                                cur_y,
                                lh,
                                content_x,
                                content_x + content_w,
                            );
                            line_left = nl;
                            line_right = nr;
                            cur_x = line_left;
                            line_h = lh.max(base_line_h);
                        }
                        let frag = TextFragment {
                            text: tok,
                            x: cur_x,
                            y: cur_y,
                            width: w,
                            height: lh,
                            baseline: cur_y + baseline_off,
                        };
                        let fidx = self.tree.get(item_id).text_fragments.len();
                        self.tree.get_mut(item_id).text_fragments.push(frag);
                        line_items.push((item_id, fidx));
                        cur_x += w;
                        line_h = line_h.max(lh);
                        line_has_content = true;
                    }
                    let frags = self.tree.get(item_id).text_fragments.clone();
                    if let (Some(first), Some(last)) = (frags.first(), frags.last()) {
                        let min_x = frags.iter().map(|f| f.x).fold(first.x, f32::min);
                        let min_y = frags.iter().map(|f| f.y).fold(first.y, f32::min);
                        let max_x = frags.iter().map(|f| f.x + f.width).fold(last.x + last.width, f32::max);
                        let max_y = frags.iter().map(|f| f.y + f.height).fold(last.y + last.height, f32::max);
                        self.tree.get_mut(item_id).dims.content =
                            Rect::new(min_x, min_y, max_x - min_x, max_y - min_y);
                    }
                }
                _ => {
                    self.layout_box(
                        item_id,
                        content_w,
                        cb_height,
                        cur_x,
                        cur_y,
                        SizeConstraint::Available(content_w),
                    );
                    let mb = self.tree.get(item_id).dims.margin_box();
                    if line_has_content && cur_x + mb.width > line_right + 0.5 {
                        finish_line(
                            self,
                            &mut line_items,
                            &mut line_box_items,
                            line_left,
                            line_right,
                            cur_x,
                            cur_y,
                            &mut line_h,
                            container_style.text_align,
                        );
                        cur_y += line_h;
                        let (nl, nr) = self.float_ctx.available_span(
                            cur_y,
                            mb.height.max(base_line_h),
                            content_x,
                            content_x + content_w,
                        );
                        line_left = nl;
                        line_right = nr;
                        cur_x = line_left;
                        line_h = base_line_h;
                        let dx = cur_x - mb.x;
                        let dy = cur_y - mb.y;
                        self.translate_subtree(item_id, dx, dy);
                    }
                    let final_mb = self.tree.get(item_id).dims.margin_box();
                    if matches!(item_kind, BoxKind::Replaced(_))
                        && item_style.vertical_align == peedle_css::VerticalAlign::Baseline
                    {
                        let strut_ascent = container_style.font_size * 0.82
                            + (base_line_h - container_style.font_size) * 0.5;
                        let target_y = cur_y + (strut_ascent - final_mb.height).max(0.0);
                        let dy = target_y - final_mb.y;
                        if dy.abs() > 0.01 {
                            self.translate_subtree(item_id, 0.0, dy);
                        }
                    }
                    let final_mb = self.tree.get(item_id).dims.margin_box();
                    line_box_items.push(item_id);
                    cur_x += final_mb.width;
                    line_h = line_h.max(final_mb.height);
                    line_has_content = true;
                }
            }
        }

        if line_has_content {
            finish_line(
                self,
                &mut line_items,
                &mut line_box_items,
                line_left,
                line_right,
                cur_x,
                cur_y,
                &mut line_h,
                container_style.text_align,
            );
            cur_y += line_h;
        }

        self.update_inline_wrapper_bounds(children);
        (cur_y - content_y).max(0.0)
    }

    fn collect_inline_items(&self, children: &[LayoutBoxId], out: &mut Vec<LayoutBoxId>) {
        for &cid in children {
            let b = self.tree.get(cid);
            if b.kind == BoxKind::Inline && !matches!(b.style.position, Position::Absolute | Position::Fixed) {
                let sub = b.children.clone();
                self.collect_inline_items(&sub, out);
            } else {
                out.push(cid);
            }
        }
    }

    fn update_inline_wrapper_bounds(&mut self, children: &[LayoutBoxId]) {
        for &cid in children {
            if self.tree.get(cid).kind == BoxKind::Inline {
                let sub = self.tree.get(cid).children.clone();
                self.update_inline_wrapper_bounds(&sub);
                let mut min_x = f32::INFINITY;
                let mut min_y = f32::INFINITY;
                let mut max_x = f32::NEG_INFINITY;
                let mut max_y = f32::NEG_INFINITY;
                for &sc in &sub {
                    let r = self.tree.get(sc).dims.border_box();
                    if r.width > 0.0 || r.height > 0.0 {
                        min_x = min_x.min(r.x);
                        min_y = min_y.min(r.y);
                        max_x = max_x.max(r.right());
                        max_y = max_y.max(r.bottom());
                    }
                }
                if min_x.is_finite() {
                    let s = self.tree.get(cid).style.clone();
                    let ctx = self.calc_ctx(&s, self.viewport_w);
                    let pad = EdgeSizes {
                        top: s.padding_top.resolve_or(&ctx, 0.0),
                        right: s.padding_right.resolve_or(&ctx, 0.0),
                        bottom: s.padding_bottom.resolve_or(&ctx, 0.0),
                        left: s.padding_left.resolve_or(&ctx, 0.0),
                    };
                    let b = self.tree.get_mut(cid);
                    b.dims.padding = pad;
                    b.dims.border = EdgeSizes {
                        top: s.border_top_width,
                        right: s.border_right_width,
                        bottom: s.border_bottom_width,
                        left: s.border_left_width,
                    };
                    b.dims.content = Rect::new(min_x, min_y, max_x - min_x, max_y - min_y);
                }
            }
        }
    }

    pub fn layout_out_of_flow(&mut self, id: LayoutBoxId, _container_id: LayoutBoxId) {
        let style = self.tree.get(id).style.clone();
        let cb = if style.position == Position::Fixed {
            Rect::new(0.0, 0.0, self.viewport_w, self.viewport_h)
        } else if let Some(&cb_id) = self.positioned_cb_stack.last() {
            self.tree.get(cb_id).dims.padding_box()
        } else {
            Rect::new(0.0, 0.0, self.viewport_w, self.viewport_h)
        };
        let ctx_w = self.calc_ctx(&style, cb.width);
        let ctx_h = self.calc_ctx(&style, cb.height);

        let left = style.left.resolve(&ctx_w);
        let right = style.right.resolve(&ctx_w);
        let top = style.top.resolve(&ctx_h);
        let bottom = style.bottom.resolve(&ctx_h);

        let constraint = if style.width != LengthPercentageAuto::Auto {
            SizeConstraint::Available(cb.width)
        } else if let (Some(l), Some(r)) = (left, right) {
            SizeConstraint::DefiniteBorderBox((cb.width - l - r).max(0.0))
        } else {
            let iw = self.measure_intrinsic_width(id, cb.width);
            let pb = style.padding_left.resolve_or(&ctx_w, 0.0)
                + style.padding_right.resolve_or(&ctx_w, 0.0)
                + style.border_left_width
                + style.border_right_width;
            SizeConstraint::DefiniteBorderBox((iw + pb).min(cb.width))
        };

        self.layout_box(id, cb.width, cb.height, cb.x, cb.y, constraint);

        if style.height == LengthPercentageAuto::Auto {
            if let (Some(t), Some(bt)) = (top, bottom) {
                let target_h = (cb.height - t - bt).max(0.0);
                let b = self.tree.get_mut(id);
                let extra = b.dims.padding.vertical() + b.dims.border.vertical();
                b.dims.content.height = (target_h - extra).max(0.0);
            }
        }

        let mb = self.tree.get(id).dims.margin_box();
        let target_x = if let Some(l) = left {
            cb.x + l
        } else if let Some(r) = right {
            cb.right() - r - mb.width
        } else {
            cb.x
        };
        let target_y = if let Some(t) = top {
            cb.y + t
        } else if let Some(bt) = bottom {
            cb.bottom() - bt - mb.height
        } else {
            cb.y
        };
        self.translate_subtree(id, target_x - mb.x, target_y - mb.y);
    }

    pub fn apply_relative_offset(&mut self, id: LayoutBoxId, cb_width: f32, cb_height: f32) {
        let style = self.tree.get(id).style.clone();
        if style.position != Position::Relative {
            return;
        }
        let ctx_w = self.calc_ctx(&style, cb_width);
        let ctx_h = self.calc_ctx(&style, cb_height);
        let dx = style
            .left
            .resolve(&ctx_w)
            .or_else(|| style.right.resolve(&ctx_w).map(|r| -r))
            .unwrap_or(0.0);
        let dy = style
            .top
            .resolve(&ctx_h)
            .or_else(|| style.bottom.resolve(&ctx_h).map(|b| -b))
            .unwrap_or(0.0);
        if dx != 0.0 || dy != 0.0 {
            self.translate_subtree(id, dx, dy);
        }
    }

    pub fn translate_subtree(&mut self, id: LayoutBoxId, dx: f32, dy: f32) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        let children = {
            let b = self.tree.get_mut(id);
            b.dims.content.x += dx;
            b.dims.content.y += dy;
            for f in &mut b.text_fragments {
                f.x += dx;
                f.y += dy;
                f.baseline += dy;
            }
            b.children.clone()
        };
        for cid in children {
            self.translate_subtree(cid, dx, dy);
        }
    }

    pub fn measure_intrinsic_width(&mut self, id: LayoutBoxId, cb_width: f32) -> f32 {
        let kind = self.tree.get(id).kind.clone();
        let style = self.tree.get(id).style.clone();
        let ctx = self.calc_ctx(&style, cb_width);
        if let Some(w) = style.width.resolve(&ctx) {
            return w;
        }
        match kind {
            BoxKind::Text(ref t) => {
                let s = apply_text_transform(t, style.text_transform);
                self.measurer
                    .measure_str(s.trim(), &style.font_family, style.font_size, style.font_weight)
            }
            BoxKind::Replaced(ref rep) => self.resolve_replaced_size(rep, &style, cb_width, self.viewport_h).0,
            BoxKind::Grid => {
                let ctx_g = self.calc_ctx(&style, cb_width);
                let gap = style.column_gap.resolve_or(&ctx_g, 0.0).max(0.0);
                let mut total = 0.0f32;
                let mut count = 0usize;
                for tr in &style.grid_template_columns {
                    match tr {
                        peedle_css::GridTrackSize::Px(v) => {
                            total += *v;
                            count += 1;
                        }
                        peedle_css::GridTrackSize::Repeat(peedle_css::GridRepeatCount::Count(n), inner) => {
                            for _ in 0..*n {
                                for it in inner {
                                    if let peedle_css::GridTrackSize::Px(v) = it {
                                        total += *v;
                                        count += 1;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                if count > 0 {
                    total + gap * (count.saturating_sub(1) as f32)
                } else {
                    cb_width
                }
            }
            _ => {
                let kids = self.tree.get(id).children.clone();
                let mut sum_inline = 0.0f32;
                let mut max_block = 0.0f32;
                for cid in kids {
                    let cw = self.measure_intrinsic_width(cid, cb_width);
                    let extra = if matches!(self.tree.get(cid).kind, BoxKind::Text(_)) {
                        0.0
                    } else {
                        let cs = self.tree.get(cid).style.clone();
                        let cctx = self.calc_ctx(&cs, cb_width);
                        cs.padding_left.resolve_or(&cctx, 0.0)
                            + cs.padding_right.resolve_or(&cctx, 0.0)
                            + cs.border_left_width
                            + cs.border_right_width
                            + cs.margin_left.resolve_or(&cctx, 0.0)
                            + cs.margin_right.resolve_or(&cctx, 0.0)
                    };
                    if self.is_block_level(cid) && style.display != Display::Flex {
                        max_block = max_block.max(cw + extra);
                    } else {
                        sum_inline += cw + extra;
                    }
                }
                max_block.max(sum_inline)
            }
        }
    }

    fn resolve_replaced_size(
        &self,
        rep: &ReplacedKind,
        style: &ComputedStyle,
        cb_width: f32,
        cb_height: f32,
    ) -> (f32, f32) {
        let ctx_w = self.calc_ctx(style, cb_width);
        let ctx_h = self.calc_ctx(style, cb_height);
        let (iw, ih) = match rep {
            ReplacedKind::Image { intrinsic_w, intrinsic_h, .. } => (*intrinsic_w, *intrinsic_h),
            ReplacedKind::Canvas { width, height } => (*width, *height),
            ReplacedKind::Svg { width, height } => (*width, *height),
            ReplacedKind::Input { input_type, .. } => match input_type.as_str() {
                "checkbox" | "radio" => (16.0, 16.0),
                "hidden" => (0.0, 0.0),
                "submit" | "button" | "reset" => (96.0, 30.0),
                "range" => (140.0, 20.0),
                _ => (180.0, 30.0),
            },
            ReplacedKind::Textarea { rows, cols, .. } => {
                ((*cols as f32 * 8.0).max(160.0), (*rows as f32 * 20.0).max(52.0))
            }
            ReplacedKind::Select { .. } => (160.0, 30.0),
            ReplacedKind::Hr => (cb_width, 2.0),
            ReplacedKind::Br => (0.0, 0.0),
        };
        let ew = style.width.resolve(&ctx_w);
        let eh = style.height.resolve(&ctx_h);
        match (ew, eh) {
            (Some(w), Some(h)) => (w.max(0.0), h.max(0.0)),
            (Some(w), None) => {
                let h = if iw > 0.0 { w * (ih / iw) } else { ih };
                (w.max(0.0), h.max(0.0))
            }
            (None, Some(h)) => {
                let w = if ih > 0.0 { h * (iw / ih) } else { iw };
                (w.max(0.0), h.max(0.0))
            }
            (None, None) => {
                let mut w = iw;
                let mut h = ih;
                if let Some(max_w) = style.max_width.resolve(&ctx_w) {
                    if w > max_w && w > 0.0 {
                        let scale = max_w / w;
                        w = max_w;
                        h *= scale;
                    }
                }
                (w, h)
            }
        }
    }
}

fn inherited_only_style(parent: &ComputedStyle, display: Display) -> ComputedStyle {
    let mut s = ComputedStyle::default();
    s.display = display;
    s.color = parent.color;
    s.font_family = parent.font_family.clone();
    s.font_size = parent.font_size;
    s.font_weight = parent.font_weight;
    s.font_style = parent.font_style;
    s.line_height = parent.line_height;
    s.letter_spacing = parent.letter_spacing;
    s.word_spacing = parent.word_spacing;
    s.text_align = parent.text_align;
    s.text_decoration_line = parent.text_decoration_line;
    s.text_decoration_style = parent.text_decoration_style;
    s.text_decoration_color = parent.text_decoration_color;
    s.text_transform = parent.text_transform;
    s.text_indent = parent.text_indent;
    s.text_shadow = parent.text_shadow.clone();
    s.white_space = parent.white_space;
    s.word_break = parent.word_break;
    s.overflow_wrap = parent.overflow_wrap;
    s.visibility = parent.visibility;
    s.cursor = parent.cursor;
    s.pointer_events = parent.pointer_events;
    s.user_select = parent.user_select;
    s
}

fn detect_replaced(
    doc: &Document,
    node_id: NodeId,
    el: &peedle_html::ElementData,
    image_sizes: &HashMap<String, (u32, u32)>,
) -> Option<ReplacedKind> {
    if el.namespace == Namespace::Svg && el.tag_name.eq_ignore_ascii_case("svg") {
        let w = el
            .get_attr("width")
            .and_then(|s| s.trim_end_matches("px").parse::<f32>().ok())
            .unwrap_or(150.0);
        let h = el
            .get_attr("height")
            .and_then(|s| s.trim_end_matches("px").parse::<f32>().ok())
            .unwrap_or(150.0);
        return Some(ReplacedKind::Svg { width: w, height: h });
    }
    if !el.is_html() {
        return None;
    }
    match el.tag_name.as_str() {
        "img" => {
            let url = el.get_attr("src").unwrap_or("").to_string();
            let attr_w = el.get_attr("width").and_then(|s| s.parse::<f32>().ok());
            let attr_h = el.get_attr("height").and_then(|s| s.parse::<f32>().ok());
            let (nat_w, nat_h) = image_sizes
                .get(&url)
                .map(|&(w, h)| (w as f32, h as f32))
                .unwrap_or((0.0, 0.0));
            let w = attr_w.unwrap_or(if nat_w > 0.0 { nat_w } else { 24.0 });
            let h = attr_h.unwrap_or(if nat_h > 0.0 { nat_h } else { 24.0 });
            Some(ReplacedKind::Image {
                url,
                intrinsic_w: w,
                intrinsic_h: h,
            })
        }
        "object" => {
            let url = el.get_attr("data")?;
            let &(nat_w, nat_h) = image_sizes.get(url)?;
            let attr_w = el.get_attr("width").and_then(|s| s.parse::<f32>().ok());
            let attr_h = el.get_attr("height").and_then(|s| s.parse::<f32>().ok());
            Some(ReplacedKind::Image {
                url: url.to_string(),
                intrinsic_w: attr_w.unwrap_or(nat_w as f32),
                intrinsic_h: attr_h.unwrap_or(nat_h as f32),
            })
        }
        "canvas" => {
            let w = el
                .get_attr("width")
                .and_then(|s| s.parse::<f32>().ok())
                .unwrap_or(300.0);
            let h = el
                .get_attr("height")
                .and_then(|s| s.parse::<f32>().ok())
                .unwrap_or(150.0);
            Some(ReplacedKind::Canvas { width: w, height: h })
        }
        "input" => Some(ReplacedKind::Input {
            input_type: el.get_attr("type").unwrap_or("text").to_ascii_lowercase(),
            value: el.current_value(),
            placeholder: el.get_attr("placeholder").unwrap_or("").to_string(),
            checked: el.is_checked(),
        }),
        "textarea" => {
            let val = el
                .input_value
                .clone()
                .unwrap_or_else(|| doc.text_content(node_id));
            let rows = el
                .get_attr("rows")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(3);
            let cols = el
                .get_attr("cols")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(24);
            Some(ReplacedKind::Textarea { value: val, rows, cols })
        }
        "select" => {
            let mut label = String::new();
            for c in doc.descendants(node_id) {
                if let Some(opt) = doc.element(c) {
                    if opt.tag_is("option") {
                        let txt = doc.text_content(c).trim().to_string();
                        if opt.has_attr("selected") || label.is_empty() {
                            label = txt;
                            if opt.has_attr("selected") {
                                break;
                            }
                        }
                    }
                }
            }
            Some(ReplacedKind::Select { label })
        }
        "hr" => Some(ReplacedKind::Hr),
        "br" => Some(ReplacedKind::Br),
        _ => None,
    }
}

fn apply_text_transform(text: &str, tf: TextTransform) -> String {
    match tf {
        TextTransform::None => text.to_string(),
        TextTransform::Uppercase => text.to_uppercase(),
        TextTransform::Lowercase => text.to_lowercase(),
        TextTransform::Capitalize => {
            let mut out = String::with_capacity(text.len());
            let mut cap_next = true;
            for c in text.chars() {
                if c.is_whitespace() {
                    cap_next = true;
                    out.push(c);
                } else if cap_next {
                    for uc in c.to_uppercase() {
                        out.push(uc);
                    }
                    cap_next = false;
                } else {
                    out.push(c);
                }
            }
            out
        }
    }
}

fn tokenize_inline_text(text: &str, preserve_spaces: bool, preserve_newlines: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut last_was_space = false;

    for c in text.chars() {
        if c == '\n' && preserve_newlines {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            out.push("\n".to_string());
            last_was_space = false;
            continue;
        }
        if matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C') {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            if preserve_spaces {
                out.push(" ".to_string());
            } else if !last_was_space {
                out.push(" ".to_string());
            }
            last_was_space = true;
        } else {
            word.push(c);
            last_was_space = false;
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}
