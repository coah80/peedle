use peedle_css::{
    AlignContent, AlignItems, AlignSelf, CalcContext, FlexDirection, FlexWrap, JustifyContent,
    LengthPercentageAuto, Position,
};

use crate::box_tree::LayoutBoxId;
use crate::engine::{LayoutEngine, SizeConstraint};

struct FlexItem {
    id: LayoutBoxId,
    order: i32,
    orig_index: usize,
    base_size: f32,
    hypothetical_main: f32,
    main_size: f32,
    cross_size: f32,
    flex_grow: f32,
    flex_shrink: f32,
    margin_main_start: f32,
    margin_main_end: f32,
    margin_cross_start: f32,
    margin_cross_end: f32,
    margin_main_start_auto: bool,
    margin_main_end_auto: bool,
    margin_cross_start_auto: bool,
    margin_cross_end_auto: bool,
    align_self: AlignItems,
}

pub fn layout_flex_container(
    engine: &mut LayoutEngine,
    id: LayoutBoxId,
    _cb_width: f32,
    cb_height: f32,
) {
    let style = engine.tree.get(id).style.clone();
    let is_row = matches!(
        style.flex_direction,
        FlexDirection::Row | FlexDirection::RowReverse
    );
    let is_reverse = matches!(
        style.flex_direction,
        FlexDirection::RowReverse | FlexDirection::ColumnReverse
    );
    let wrap = style.flex_wrap != FlexWrap::Nowrap;
    let wrap_reverse = style.flex_wrap == FlexWrap::WrapReverse;

    let content_x = engine.tree.get(id).dims.content.x;
    let content_y = engine.tree.get(id).dims.content.y;
    let content_w = engine.tree.get(id).dims.content.width;
    let explicit_h = engine.resolve_explicit_height(&style, cb_height);
    let child_cb_h = explicit_h.unwrap_or(-1.0);
    let container_main = if is_row {
        content_w
    } else {
        explicit_h.unwrap_or(0.0)
    };

    let ctx_w = engine.calc_ctx(&style, content_w);
    let ctx_h = engine.calc_ctx(&style, child_cb_h.max(0.0));
    let main_gap = if is_row {
        style.column_gap.resolve_or(&ctx_w, 0.0).max(0.0)
    } else if child_cb_h < 0.0 && matches!(style.row_gap, LengthPercentageAuto::Percent(_)) {
        0.0
    } else {
        style.row_gap.resolve_or(&ctx_h, 0.0).max(0.0)
    };
    let cross_gap = if is_row {
        if child_cb_h < 0.0 && matches!(style.row_gap, LengthPercentageAuto::Percent(_)) {
            0.0
        } else {
            style.row_gap.resolve_or(&ctx_h, 0.0).max(0.0)
        }
    } else {
        style.column_gap.resolve_or(&ctx_w, 0.0).max(0.0)
    };

    let children = engine.tree.get(id).children.clone();
    let mut items = Vec::new();
    let mut abs_children = Vec::new();

    for (orig_index, &cid) in children.iter().enumerate() {
        let cs = engine.tree.get(cid).style.clone();
        if matches!(cs.position, Position::Absolute | Position::Fixed) {
            abs_children.push(cid);
            continue;
        }
        let cctx_w = engine.calc_ctx(&cs, content_w);
        let cctx_h = engine.calc_ctx(&cs, child_cb_h.max(0.0));

        let (m_ms, m_me, m_cs, m_ce, ms_auto, me_auto, cs_auto, ce_auto) = if is_row {
            (
                cs.margin_left.resolve_or(&cctx_w, 0.0),
                cs.margin_right.resolve_or(&cctx_w, 0.0),
                cs.margin_top.resolve_or(&cctx_w, 0.0),
                cs.margin_bottom.resolve_or(&cctx_w, 0.0),
                cs.margin_left == LengthPercentageAuto::Auto,
                cs.margin_right == LengthPercentageAuto::Auto,
                cs.margin_top == LengthPercentageAuto::Auto,
                cs.margin_bottom == LengthPercentageAuto::Auto,
            )
        } else {
            (
                cs.margin_top.resolve_or(&cctx_w, 0.0),
                cs.margin_bottom.resolve_or(&cctx_w, 0.0),
                cs.margin_left.resolve_or(&cctx_w, 0.0),
                cs.margin_right.resolve_or(&cctx_w, 0.0),
                cs.margin_top == LengthPercentageAuto::Auto,
                cs.margin_bottom == LengthPercentageAuto::Auto,
                cs.margin_left == LengthPercentageAuto::Auto,
                cs.margin_right == LengthPercentageAuto::Auto,
            )
        };

        let basis_opt = match &cs.flex_basis {
            LengthPercentageAuto::Auto => {
                if is_row {
                    cs.width.resolve(&cctx_w)
                } else {
                    resolve_dim(&cs.height, &cctx_h, child_cb_h)
                }
            }
            other => {
                if is_row {
                    other.resolve(&cctx_w)
                } else {
                    resolve_dim(other, &cctx_h, child_cb_h)
                }
            }
        };

        let (pad_b_main, pad_b_cross) = if is_row {
            let ph = cs.padding_left.resolve_or(&cctx_w, 0.0)
                + cs.padding_right.resolve_or(&cctx_w, 0.0)
                + cs.border_left_width
                + cs.border_right_width;
            let pv = cs.padding_top.resolve_or(&cctx_w, 0.0)
                + cs.padding_bottom.resolve_or(&cctx_w, 0.0)
                + cs.border_top_width
                + cs.border_bottom_width;
            (ph, pv)
        } else {
            let pv = cs.padding_top.resolve_or(&cctx_w, 0.0)
                + cs.padding_bottom.resolve_or(&cctx_w, 0.0)
                + cs.border_top_width
                + cs.border_bottom_width;
            let ph = cs.padding_left.resolve_or(&cctx_w, 0.0)
                + cs.padding_right.resolve_or(&cctx_w, 0.0)
                + cs.border_left_width
                + cs.border_right_width;
            (pv, ph)
        };

        let base_size = if let Some(b) = basis_opt {
            if cs.box_sizing == peedle_css::BoxSizing::BorderBox {
                b.max(pad_b_main)
            } else {
                b + pad_b_main
            }
        } else if is_row {
            let intrinsic = engine.measure_intrinsic_width(cid, content_w);
            (intrinsic + pad_b_main).min(content_w.max(pad_b_main))
        } else {
            engine.layout_box_in_bfc(
                cid,
                content_w,
                child_cb_h,
                0.0,
                0.0,
                SizeConstraint::Available(content_w),
            );
            engine.tree.get(cid).dims.border_box().height
        };
        let _ = pad_b_cross;

        let min_main = if is_row {
            cs.min_width.resolve(&cctx_w).unwrap_or(0.0)
        } else {
            resolve_dim(&cs.min_height, &cctx_h, child_cb_h).unwrap_or(0.0)
        };
        let max_main = if is_row {
            cs.max_width.resolve(&cctx_w).unwrap_or(f32::INFINITY)
        } else {
            resolve_dim(&cs.max_height, &cctx_h, child_cb_h).unwrap_or(f32::INFINITY)
        };
        let hypothetical = base_size.clamp(min_main, max_main);

        let align_self = match cs.align_self {
            AlignSelf::Auto => style.align_items,
            AlignSelf::Stretch => AlignItems::Stretch,
            AlignSelf::FlexStart | AlignSelf::Start => AlignItems::FlexStart,
            AlignSelf::FlexEnd | AlignSelf::End => AlignItems::FlexEnd,
            AlignSelf::Center => AlignItems::Center,
            AlignSelf::Baseline => AlignItems::Baseline,
        };

        items.push(FlexItem {
            id: cid,
            order: cs.order,
            orig_index,
            base_size: hypothetical,
            hypothetical_main: hypothetical,
            main_size: hypothetical,
            cross_size: 0.0,
            flex_grow: cs.flex_grow.max(0.0),
            flex_shrink: cs.flex_shrink.max(0.0),
            margin_main_start: m_ms,
            margin_main_end: m_me,
            margin_cross_start: m_cs,
            margin_cross_end: m_ce,
            margin_main_start_auto: ms_auto,
            margin_main_end_auto: me_auto,
            margin_cross_start_auto: cs_auto,
            margin_cross_end_auto: ce_auto,
            align_self,
        });
    }

    items.sort_by(|a, b| a.order.cmp(&b.order).then(a.orig_index.cmp(&b.orig_index)));

    let mut lines: Vec<Vec<usize>> = Vec::new();
    if !items.is_empty() {
        let mut current_line = Vec::new();
        let mut line_used = 0.0f32;
        let limit = if wrap && (is_row || explicit_h.is_some()) {
            container_main
        } else {
            f32::INFINITY
        };
        for idx in 0..items.len() {
            let outer = items[idx].hypothetical_main
                + items[idx].margin_main_start
                + items[idx].margin_main_end;
            let gap = if current_line.is_empty() { 0.0 } else { main_gap };
            if !current_line.is_empty() && line_used + gap + outer > limit + 0.5 {
                lines.push(std::mem::take(&mut current_line));
                line_used = 0.0;
            }
            if !current_line.is_empty() {
                line_used += main_gap;
            }
            line_used += outer;
            current_line.push(idx);
        }
        if !current_line.is_empty() {
            lines.push(current_line);
        }
    }

    let mut line_cross_sizes: Vec<f32> = Vec::with_capacity(lines.len());
    for line in &lines {
        let gaps = main_gap * (line.len().saturating_sub(1) as f32);
        let used_hyp: f32 = line
            .iter()
            .map(|&i| items[i].hypothetical_main + items[i].margin_main_start + items[i].margin_main_end)
            .sum::<f32>()
            + gaps;

        let target_main = if is_row {
            container_main
        } else {
            explicit_h.unwrap_or(used_hyp)
        };
        let free = target_main - used_hyp;

        if free > 0.0 {
            let total_grow: f32 = line.iter().map(|&i| items[i].flex_grow).sum();
            if total_grow > 0.0 {
                for &i in line {
                    items[i].main_size = items[i].base_size + free * (items[i].flex_grow / total_grow);
                }
            }
        } else if free < 0.0 {
            let total_scaled: f32 = line
                .iter()
                .map(|&i| items[i].flex_shrink * items[i].base_size.max(1.0))
                .sum();
            if total_scaled > 0.0 {
                for &i in line {
                    let ratio = (items[i].flex_shrink * items[i].base_size.max(1.0)) / total_scaled;
                    items[i].main_size = (items[i].base_size + free * ratio).max(0.0);
                }
            }
        }

        let mut max_cross = 0.0f32;
        for &i in line {
            let cid = items[i].id;
            if is_row {
                engine.layout_box_in_bfc(
                    cid,
                    content_w,
                    child_cb_h,
                    0.0,
                    0.0,
                    SizeConstraint::DefiniteBorderBox(items[i].main_size),
                );
                let h = engine.tree.get(cid).dims.border_box().height;
                items[i].cross_size = h;
                max_cross = max_cross.max(h + items[i].margin_cross_start + items[i].margin_cross_end);
            } else {
                let cs = engine.tree.get(cid).style.clone();
                let cctx = engine.calc_ctx(&cs, content_w);
                let constraint = if cs.width != LengthPercentageAuto::Auto {
                    SizeConstraint::Available(content_w)
                } else if items[i].align_self == AlignItems::Stretch {
                    SizeConstraint::DefiniteBorderBox(
                        (content_w - items[i].margin_cross_start - items[i].margin_cross_end).max(0.0),
                    )
                } else {
                    let iw = engine.measure_intrinsic_width(cid, content_w);
                    let ph = cs.padding_left.resolve_or(&cctx, 0.0)
                        + cs.padding_right.resolve_or(&cctx, 0.0)
                        + cs.border_left_width
                        + cs.border_right_width;
                    SizeConstraint::DefiniteBorderBox((iw + ph).min(content_w))
                };
                engine.layout_box_in_bfc(
                    cid,
                    content_w,
                    child_cb_h,
                    0.0,
                    0.0,
                    constraint,
                );
                let bb = engine.tree.get(cid).dims.border_box();
                items[i].cross_size = bb.width;
                if items[i].main_size <= 0.0 {
                    items[i].main_size = bb.height;
                }
                max_cross = max_cross.max(bb.width + items[i].margin_cross_start + items[i].margin_cross_end);
            }
        }
        line_cross_sizes.push(max_cross);
    }

    if is_row && lines.len() == 1 {
        if let Some(eh) = explicit_h {
            line_cross_sizes[0] = line_cross_sizes[0].max(eh);
        }
    } else if !is_row && lines.len() == 1 {
        line_cross_sizes[0] = line_cross_sizes[0].max(content_w);
    }

    let total_cross_lines: f32 = line_cross_sizes.iter().sum::<f32>()
        + cross_gap * (lines.len().saturating_sub(1) as f32);
    let container_cross = if is_row {
        explicit_h.unwrap_or(total_cross_lines)
    } else {
        content_w
    };

    let (mut cross_cursor, extra_line_gap) = distribute_space(
        match style.align_content {
            AlignContent::FlexStart | AlignContent::Start | AlignContent::Stretch => {
                JustifyContent::FlexStart
            }
            AlignContent::FlexEnd | AlignContent::End => JustifyContent::FlexEnd,
            AlignContent::Center => JustifyContent::Center,
            AlignContent::SpaceBetween => JustifyContent::SpaceBetween,
            AlignContent::SpaceAround => JustifyContent::SpaceAround,
            AlignContent::SpaceEvenly => JustifyContent::SpaceEvenly,
        },
        container_cross - total_cross_lines,
        lines.len(),
    );

    let line_indices: Vec<usize> = if wrap_reverse {
        (0..lines.len()).rev().collect()
    } else {
        (0..lines.len()).collect()
    };

    let mut max_content_bottom = 0.0f32;
    for lidx in line_indices {
        let line = &lines[lidx];
        let line_cross = line_cross_sizes[lidx];
        let gaps = main_gap * (line.len().saturating_sub(1) as f32);
        let used_main: f32 = line
            .iter()
            .map(|&i| items[i].main_size + items[i].margin_main_start + items[i].margin_main_end)
            .sum::<f32>()
            + gaps;
        let target_main = if is_row {
            container_main
        } else {
            explicit_h.unwrap_or(used_main)
        };
        let remaining = (target_main - used_main).max(0.0);

        let auto_margins: usize = line
            .iter()
            .map(|&i| {
                (items[i].margin_main_start_auto as usize)
                    + (items[i].margin_main_end_auto as usize)
            })
            .sum();

        let (mut main_cursor, extra_main_gap) = if auto_margins > 0 {
            (0.0, 0.0)
        } else {
            distribute_space(style.justify_content, remaining, line.len())
        };
        let auto_margin_share = if auto_margins > 0 {
            remaining / auto_margins as f32
        } else {
            0.0
        };

        let ordered: Vec<usize> = if is_reverse {
            line.iter().rev().copied().collect()
        } else {
            line.clone()
        };

        for (pos_in_line, &i) in ordered.iter().enumerate() {
            if pos_in_line > 0 {
                main_cursor += main_gap + extra_main_gap;
            }
            let ms = if items[i].margin_main_start_auto {
                auto_margin_share
            } else {
                items[i].margin_main_start
            };
            let me = if items[i].margin_main_end_auto {
                auto_margin_share
            } else {
                items[i].margin_main_end
            };

            let cid = items[i].id;
            let cs = engine.tree.get(cid).style.clone();
            if is_row
                && items[i].align_self == AlignItems::Stretch
                && cs.height == LengthPercentageAuto::Auto
                && !items[i].margin_cross_start_auto
                && !items[i].margin_cross_end_auto
            {
                let target_h = (line_cross - items[i].margin_cross_start - items[i].margin_cross_end).max(0.0);
                let b = engine.tree.get_mut(cid);
                let non_content = b.dims.padding.vertical() + b.dims.border.vertical();
                let new_ch = (target_h - non_content).max(b.dims.content.height);
                b.dims.content.height = new_ch;
                items[i].cross_size = new_ch + non_content;
            }

            let cross_free = (line_cross
                - items[i].cross_size
                - items[i].margin_cross_start
                - items[i].margin_cross_end)
                .max(0.0);
            let cross_offset = if items[i].margin_cross_start_auto && items[i].margin_cross_end_auto {
                cross_free * 0.5
            } else if items[i].margin_cross_start_auto {
                cross_free
            } else if items[i].margin_cross_end_auto {
                0.0
            } else {
                match items[i].align_self {
                    AlignItems::FlexEnd => cross_free,
                    AlignItems::Center => cross_free * 0.5,
                    _ => 0.0,
                }
            };

            let (target_bx, target_by) = if is_row {
                (
                    content_x + main_cursor + ms,
                    content_y + cross_cursor + items[i].margin_cross_start + cross_offset,
                )
            } else {
                (
                    content_x + cross_cursor + items[i].margin_cross_start + cross_offset,
                    content_y + main_cursor + ms,
                )
            };

            let cur_bb = engine.tree.get(cid).dims.border_box();
            let dx = target_bx - cur_bb.x;
            let dy = target_by - cur_bb.y;
            engine.translate_subtree(cid, dx, dy);
            engine.apply_relative_offset(cid, content_w, child_cb_h.max(0.0));

            let final_bb = engine.tree.get(cid).dims.border_box();
            max_content_bottom = max_content_bottom.max(
                final_bb.bottom() - content_y
                    + if is_row {
                        items[i].margin_cross_end
                    } else {
                        me
                    },
            );

            main_cursor += ms + items[i].main_size + me;
        }
        cross_cursor += line_cross + cross_gap + extra_line_gap;
    }

    let final_h = explicit_h.unwrap_or(max_content_bottom.max(0.0));
    let own_ctx_h = CalcContext {
        percentage_base: cb_height.max(0.0),
        font_size: style.font_size,
        root_font_size: engine.root_font_size,
        viewport_width: engine.viewport_w,
        viewport_height: engine.viewport_h,
    };
    let min_h = resolve_dim(&style.min_height, &own_ctx_h, cb_height);
    let max_h = resolve_dim(&style.max_height, &own_ctx_h, cb_height);
    let mut clamped_h = final_h;
    if let Some(mh) = min_h {
        clamped_h = clamped_h.max(mh);
    }
    if let Some(xh) = max_h {
        clamped_h = clamped_h.min(xh);
    }
    engine.tree.get_mut(id).dims.content.height = clamped_h;

    for cid in abs_children {
        engine.layout_out_of_flow(cid, id);
    }
}

fn resolve_dim(val: &LengthPercentageAuto, ctx: &CalcContext, cb_size: f32) -> Option<f32> {
    if cb_size < 0.0 && matches!(val, LengthPercentageAuto::Percent(_)) {
        None
    } else {
        val.resolve(ctx)
    }
}

fn distribute_space(justify: JustifyContent, remaining: f32, count: usize) -> (f32, f32) {
    if count == 0 || remaining <= 0.0 {
        return (0.0, 0.0);
    }
    match justify {
        JustifyContent::FlexStart | JustifyContent::Start | JustifyContent::Stretch => (0.0, 0.0),
        JustifyContent::FlexEnd | JustifyContent::End => (remaining, 0.0),
        JustifyContent::Center => (remaining * 0.5, 0.0),
        JustifyContent::SpaceBetween => {
            if count <= 1 {
                (0.0, 0.0)
            } else {
                (0.0, remaining / (count - 1) as f32)
            }
        }
        JustifyContent::SpaceAround => {
            let slot = remaining / count as f32;
            (slot * 0.5, slot)
        }
        JustifyContent::SpaceEvenly => {
            let slot = remaining / (count + 1) as f32;
            (slot, slot)
        }
    }
}
