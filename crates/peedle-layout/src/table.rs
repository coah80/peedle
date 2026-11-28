use peedle_css::{BorderCollapse, LengthPercentageAuto};

use crate::box_tree::{BoxKind, LayoutBoxId};
use crate::engine::{LayoutEngine, SizeConstraint};

pub fn layout_table(
    engine: &mut LayoutEngine,
    id: LayoutBoxId,
    cb_width: f32,
    cb_height: f32,
) {
    let style = engine.tree.get(id).style.clone();
    let content_x = engine.tree.get(id).dims.content.x;
    let content_y = engine.tree.get(id).dims.content.y;
    let dims = engine.tree.get(id).dims;
    let table_w = if style.width == LengthPercentageAuto::Auto {
        (cb_width - dims.margin.horizontal() - dims.border.horizontal() - dims.padding.horizontal()).max(0.0)
    } else {
        dims.content.width
    };
    let explicit_h = engine.resolve_explicit_height(&style, cb_height);

    let (sx, sy) = if style.border_collapse == BorderCollapse::Collapse {
        (0.0, 0.0)
    } else {
        (style.border_spacing_x, style.border_spacing_y)
    };

    let mut row_entries: Vec<(Option<LayoutBoxId>, Vec<(LayoutBoxId, usize)>)> = Vec::new();
    collect_table_rows_and_cells(engine, id, &mut row_entries);

    let mut num_cols = 0usize;
    for (_, cells) in &row_entries {
        let cols_in_row: usize = cells.iter().map(|(_, cs)| if *cs == 0 { 1 } else { *cs }).sum();
        num_cols = num_cols.max(cols_in_row);
    }

    if num_cols == 0 {
        engine.tree.get_mut(id).dims.content.height = explicit_h.unwrap_or(0.0);
        return;
    }

    let mut col_preferred = vec![0.0f32; num_cols];
    let mut col_min = vec![0.0f32; num_cols];
    let mut col_fixed = vec![false; num_cols];
    for (_, cells) in &row_entries {
        let mut col = 0usize;
        for &(cid, colspan) in cells {
            if colspan == 0 {
                continue;
            }
            if colspan == 1 && col < num_cols {
                let cs = engine.tree.get(cid).style.clone();
                let cctx = engine.calc_ctx(&cs, table_w);
                let pb_h = cs.padding_left.resolve_or(&cctx, 0.0)
                    + cs.padding_right.resolve_or(&cctx, 0.0)
                    + cs.border_left_width
                    + cs.border_right_width;
                let min_w = engine.measure_min_word_width(cid, table_w) + pb_h;
                let pref = if let Some(w) = cs.width.resolve(&cctx) {
                    if matches!(cs.width, LengthPercentageAuto::Length(_)) {
                        col_fixed[col] = true;
                    }
                    (w + pb_h).max(min_w)
                } else {
                    (engine.measure_intrinsic_width(cid, table_w) + pb_h).max(min_w)
                };
                col_min[col] = col_min[col].max(min_w);
                col_preferred[col] = col_preferred[col].max(pref);
            }
            col += colspan;
        }
    }

    let total_spacing_x = sx * (num_cols as f32 + 1.0);
    let avail_w = (table_w - total_spacing_x).max(0.0);
    let pref_sum: f32 = col_preferred.iter().sum();
    let min_sum: f32 = col_min.iter().sum();

    let col_widths: Vec<f32> = if pref_sum <= 0.0 {
        vec![avail_w / num_cols as f32; num_cols]
    } else if pref_sum > avail_w {
        if min_sum > 0.0 && min_sum <= avail_w {
            let extra_avail = avail_w - min_sum;
            let extra_need: f32 = col_preferred
                .iter()
                .zip(col_min.iter())
                .map(|(p, m)| (p - m).max(0.0))
                .sum();
            col_preferred
                .iter()
                .zip(col_min.iter())
                .map(|(&p, &m)| {
                    if extra_need > 0.0 {
                        m + extra_avail * ((p - m).max(0.0) / extra_need)
                    } else {
                        m
                    }
                })
                .collect()
        } else {
            col_preferred
                .iter()
                .map(|&p| avail_w * (p / pref_sum))
                .collect()
        }
    } else if style.width != LengthPercentageAuto::Auto {
        let extra = avail_w - pref_sum;
        let flex_sum: f32 = col_preferred
            .iter()
            .enumerate()
            .filter(|(i, _)| !col_fixed[*i])
            .map(|(_, &p)| p)
            .sum();
        let flex_count = col_fixed.iter().filter(|&&f| !f).count();
        if flex_count > 0 {
            col_preferred
                .iter()
                .enumerate()
                .map(|(i, &p)| {
                    if col_fixed[i] {
                        p
                    } else if flex_sum > 0.0 {
                        p + extra * (p / flex_sum)
                    } else {
                        p + extra / flex_count as f32
                    }
                })
                .collect()
        } else {
            col_preferred
                .iter()
                .map(|&p| avail_w * (p / pref_sum))
                .collect()
        }
    } else {
        col_preferred
    };

    let actual_table_w = col_widths.iter().sum::<f32>() + total_spacing_x;
    if style.width == LengthPercentageAuto::Auto {
        engine.tree.get_mut(id).dims.content.width = actual_table_w;
    }

    let mut cursor_y = sy;
    for (rid_opt, cells) in &row_entries {
        let mut col = 0usize;
        let mut cursor_x = sx;
        let mut row_h = 0.0f32;

        for &(cid, colspan) in cells {
            let eff_span = if colspan == 0 { num_cols } else { colspan };
            let end_col = (col + eff_span).min(num_cols);
            let cell_w: f32 = col_widths[col..end_col].iter().sum::<f32>()
                + sx * (eff_span.saturating_sub(1) as f32);
            engine.layout_box(
                cid,
                table_w,
                explicit_h.unwrap_or(-1.0),
                content_x + cursor_x,
                content_y + cursor_y,
                SizeConstraint::DefiniteBorderBox(cell_w),
            );
            let ch = engine.tree.get(cid).dims.border_box().height;
            row_h = row_h.max(ch);
            cursor_x += cell_w + sx;
            col = end_col;
        }

        for &(cid, _) in cells {
            let b = engine.tree.get_mut(cid);
            let extra_h = b.dims.padding.vertical() + b.dims.border.vertical();
            b.dims.content.height = (row_h - extra_h).max(b.dims.content.height);
        }

        if let Some(rid) = *rid_opt {
            let rbox = engine.tree.get_mut(rid);
            rbox.dims.content.x = content_x;
            rbox.dims.content.y = content_y + cursor_y;
            rbox.dims.content.width = actual_table_w;
            rbox.dims.content.height = row_h;
        }

        cursor_y += row_h + sy;
    }

    engine.tree.get_mut(id).dims.content.height = explicit_h.unwrap_or(cursor_y);
}

fn collect_table_rows_and_cells(
    engine: &LayoutEngine,
    parent: LayoutBoxId,
    out: &mut Vec<(Option<LayoutBoxId>, Vec<(LayoutBoxId, usize)>)>,
) {
    let kids = engine.tree.get(parent).children.clone();
    let mut anon_cells: Vec<(LayoutBoxId, usize)> = Vec::new();
    for cid in kids {
        match engine.tree.get(cid).kind {
            BoxKind::TableRow => {
                if !anon_cells.is_empty() {
                    out.push((None, std::mem::take(&mut anon_cells)));
                }
                let mut cells = Vec::new();
                for &cell_id in &engine.tree.get(cid).children {
                    let colspan = match engine.tree.get(cell_id).kind {
                        BoxKind::TableCell { colspan, .. } => colspan.max(1),
                        BoxKind::Text(ref t) if t.trim().is_empty() => continue,
                        _ => 1,
                    };
                    cells.push((cell_id, colspan));
                }
                out.push((Some(cid), cells));
            }
            BoxKind::TableRowGroup => {
                if !anon_cells.is_empty() {
                    out.push((None, std::mem::take(&mut anon_cells)));
                }
                collect_table_rows_and_cells(engine, cid, out);
            }
            BoxKind::Text(ref t) if t.trim().is_empty() => {}
            BoxKind::TableCell { colspan, .. } => {
                anon_cells.push((cid, colspan.max(1)));
            }
            _ if engine.tree.get(cid).style.display == peedle_css::Display::TableCaption => {
                if !anon_cells.is_empty() {
                    out.push((None, std::mem::take(&mut anon_cells)));
                }
                out.push((None, vec![(cid, 0)]));
            }
            _ => {
                anon_cells.push((cid, 1));
            }
        }
    }
    if !anon_cells.is_empty() {
        out.push((None, anon_cells));
    }
}
