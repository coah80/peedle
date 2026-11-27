use peedle_css::{BorderCollapse, LengthPercentageAuto};

use crate::box_tree::{BoxKind, LayoutBoxId};
use crate::engine::{LayoutEngine, SizeConstraint};

pub fn layout_table(
    engine: &mut LayoutEngine,
    id: LayoutBoxId,
    _cb_width: f32,
    cb_height: f32,
) {
    let style = engine.tree.get(id).style.clone();
    let content_x = engine.tree.get(id).dims.content.x;
    let content_y = engine.tree.get(id).dims.content.y;
    let table_w = engine.tree.get(id).dims.content.width;
    let explicit_h = engine.resolve_explicit_height(&style, cb_height);

    let (sx, sy) = if style.border_collapse == BorderCollapse::Collapse {
        (0.0, 0.0)
    } else {
        (style.border_spacing_x, style.border_spacing_y)
    };

    let mut rows: Vec<LayoutBoxId> = Vec::new();
    collect_table_rows(engine, id, &mut rows);

    let mut row_cells: Vec<Vec<(LayoutBoxId, usize)>> = Vec::new();
    let mut num_cols = 0usize;
    for &rid in &rows {
        let mut cells = Vec::new();
        let mut cols_in_row = 0usize;
        for &cid in &engine.tree.get(rid).children {
            let colspan = match engine.tree.get(cid).kind {
                BoxKind::TableCell { colspan, .. } => colspan.max(1),
                _ => 1,
            };
            cells.push((cid, colspan));
            cols_in_row += colspan;
        }
        num_cols = num_cols.max(cols_in_row);
        row_cells.push(cells);
    }

    if num_cols == 0 {
        engine.tree.get_mut(id).dims.content.height = explicit_h.unwrap_or(0.0);
        return;
    }

    let mut col_preferred = vec![0.0f32; num_cols];
    for cells in &row_cells {
        let mut col = 0usize;
        for &(cid, colspan) in cells {
            if colspan == 1 && col < num_cols {
                let cs = engine.tree.get(cid).style.clone();
                let cctx = engine.calc_ctx(&cs, table_w);
                let pref = if let Some(w) = cs.width.resolve(&cctx) {
                    w
                } else {
                    engine.measure_intrinsic_width(cid, table_w)
                        + cs.padding_left.resolve_or(&cctx, 0.0)
                        + cs.padding_right.resolve_or(&cctx, 0.0)
                        + cs.border_left_width
                        + cs.border_right_width
                };
                col_preferred[col] = col_preferred[col].max(pref.max(24.0));
            }
            col += colspan;
        }
    }

    let total_spacing_x = sx * (num_cols as f32 + 1.0);
    let avail_w = (table_w - total_spacing_x).max(0.0);
    let pref_sum: f32 = col_preferred.iter().sum();

    let col_widths: Vec<f32> = if pref_sum <= 0.0 {
        vec![avail_w / num_cols as f32; num_cols]
    } else if style.width != LengthPercentageAuto::Auto || pref_sum > avail_w {
        col_preferred
            .iter()
            .map(|&p| avail_w * (p / pref_sum))
            .collect()
    } else {
        col_preferred
    };

    let actual_table_w = col_widths.iter().sum::<f32>() + total_spacing_x;
    if style.width == LengthPercentageAuto::Auto {
        engine.tree.get_mut(id).dims.content.width = actual_table_w;
    }

    let mut cursor_y = sy;
    for (r_idx, &rid) in rows.iter().enumerate() {
        let cells = &row_cells[r_idx];
        let mut col = 0usize;
        let mut cursor_x = sx;
        let mut row_h = 0.0f32;

        for &(cid, colspan) in cells {
            let end_col = (col + colspan).min(num_cols);
            let cell_w: f32 = col_widths[col..end_col].iter().sum::<f32>()
                + sx * (colspan.saturating_sub(1) as f32);
            engine.layout_box(
                cid,
                table_w,
                explicit_h.unwrap_or(cb_height),
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

        let rbox = engine.tree.get_mut(rid);
        rbox.dims.content.x = content_x;
        rbox.dims.content.y = content_y + cursor_y;
        rbox.dims.content.width = actual_table_w;
        rbox.dims.content.height = row_h;

        cursor_y += row_h + sy;
    }

    engine.tree.get_mut(id).dims.content.height = explicit_h.unwrap_or(cursor_y);
}

fn collect_table_rows(engine: &LayoutEngine, parent: LayoutBoxId, out: &mut Vec<LayoutBoxId>) {
    let kids = engine.tree.get(parent).children.clone();
    for cid in kids {
        match engine.tree.get(cid).kind {
            BoxKind::TableRow => out.push(cid),
            BoxKind::TableRowGroup => collect_table_rows(engine, cid, out),
            _ => {}
        }
    }
}
