use peedle_css::{
    AlignItems, GridPlacement, GridRepeatCount, GridTrackSize, LengthPercentageAuto, Position,
};

use crate::box_tree::LayoutBoxId;
use crate::engine::{LayoutEngine, SizeConstraint};

pub fn layout_grid_container(
    engine: &mut LayoutEngine,
    id: LayoutBoxId,
    _cb_width: f32,
    cb_height: f32,
) {
    let style = engine.tree.get(id).style.clone();
    let content_x = engine.tree.get(id).dims.content.x;
    let content_y = engine.tree.get(id).dims.content.y;
    let content_w = engine.tree.get(id).dims.content.width;
    let explicit_h = engine.resolve_explicit_height(&style, cb_height);

    let ctx_w = engine.calc_ctx(&style, content_w);
    let ctx_h = engine.calc_ctx(&style, explicit_h.unwrap_or(cb_height));
    let col_gap = style.column_gap.resolve_or(&ctx_w, 0.0).max(0.0);
    let row_gap = style.row_gap.resolve_or(&ctx_h, 0.0).max(0.0);

    let col_tracks = expand_tracks(&style.grid_template_columns, content_w, col_gap);
    let num_cols = col_tracks.len().max(1);
    let col_widths = resolve_track_sizes(&col_tracks, content_w, col_gap, num_cols);

    let children = engine.tree.get(id).children.clone();
    let mut flow_children = Vec::new();
    let mut abs_children = Vec::new();
    for cid in children {
        let pos = engine.tree.get(cid).style.position;
        if matches!(pos, Position::Absolute | Position::Fixed) {
            abs_children.push(cid);
        } else {
            flow_children.push(cid);
        }
    }

    let mut placements: Vec<(LayoutBoxId, usize, usize, usize, usize)> = Vec::new();
    let mut occupied: Vec<Vec<bool>> = Vec::new();
    let mut cursor_r = 0usize;
    let mut cursor_c = 0usize;

    for cid in flow_children {
        let cs = engine.tree.get(cid).style.clone();
        let (c_start, c_span) = resolve_placement(cs.grid_column_start, cs.grid_column_end, num_cols);
        let (r_start, r_span) = resolve_placement(cs.grid_row_start, cs.grid_row_end, 100);

        let (row, col) = match (r_start, c_start) {
            (Some(r), Some(c)) => (r, c.min(num_cols - 1)),
            (Some(r), None) => {
                let mut c = 0;
                while c + c_span <= num_cols && is_area_occupied(&occupied, r, c, r_span, c_span) {
                    c += 1;
                }
                (r, c.min(num_cols - 1))
            }
            (None, Some(c)) => {
                let col = c.min(num_cols - 1);
                let mut r = 0;
                while is_area_occupied(&occupied, r, col, r_span, c_span) {
                    r += 1;
                }
                (r, col)
            }
            (None, None) => {
                loop {
                    if cursor_c + c_span > num_cols {
                        cursor_c = 0;
                        cursor_r += 1;
                    }
                    if !is_area_occupied(&occupied, cursor_r, cursor_c, r_span, c_span) {
                        break;
                    }
                    cursor_c += 1;
                }
                let res = (cursor_r, cursor_c);
                cursor_c += c_span;
                res
            }
        };

        mark_occupied(&mut occupied, row, col, r_span, c_span, num_cols);
        placements.push((cid, row, r_span, col, c_span.min(num_cols - col)));
    }

    let num_rows = occupied.len().max(1);
    let mut row_heights = vec![0.0f32; num_rows];
    let explicit_row_tracks = expand_tracks(&style.grid_template_rows, explicit_h.unwrap_or(cb_height), row_gap);
    for (r, track) in explicit_row_tracks.iter().enumerate() {
        if r < num_rows {
            if let Some(h) = fixed_track_size(track, explicit_h.unwrap_or(cb_height)) {
                row_heights[r] = h;
            }
        }
    }

    let mut col_offsets = Vec::with_capacity(num_cols);
    let mut cx = 0.0f32;
    for &w in &col_widths {
        col_offsets.push(cx);
        cx += w + col_gap;
    }

    for &(cid, row, r_span, col, c_span) in &placements {
        let cell_w: f32 = col_widths[col..col + c_span].iter().sum::<f32>()
            + col_gap * (c_span.saturating_sub(1) as f32);
        let cs = engine.tree.get(cid).style.clone();
        let cctx = engine.calc_ctx(&cs, cell_w);
        let mh = cs.margin_left.resolve_or(&cctx, 0.0) + cs.margin_right.resolve_or(&cctx, 0.0);
        let mv = cs.margin_top.resolve_or(&cctx, 0.0) + cs.margin_bottom.resolve_or(&cctx, 0.0);

        let constraint = if cs.width == LengthPercentageAuto::Auto {
            SizeConstraint::DefiniteBorderBox((cell_w - mh).max(0.0))
        } else {
            SizeConstraint::Available((cell_w - mh).max(0.0))
        };
        engine.layout_box(
            cid,
            cell_w,
            explicit_h.unwrap_or(cb_height),
            0.0,
            0.0,
            constraint,
        );
        let item_h = engine.tree.get(cid).dims.border_box().height + mv;
        if r_span == 1 {
            row_heights[row] = row_heights[row].max(item_h);
        } else {
            let cur: f32 = row_heights[row..row + r_span].iter().sum::<f32>()
                + row_gap * (r_span - 1) as f32;
            if item_h > cur {
                let add = (item_h - cur) / r_span as f32;
                for rh in &mut row_heights[row..row + r_span] {
                    *rh += add;
                }
            }
        }
    }

    let mut row_offsets = Vec::with_capacity(num_rows);
    let mut cy = 0.0f32;
    for &h in &row_heights {
        row_offsets.push(cy);
        cy += h + row_gap;
    }

    for &(cid, row, r_span, col, c_span) in &placements {
        let cell_w: f32 = col_widths[col..col + c_span].iter().sum::<f32>()
            + col_gap * (c_span.saturating_sub(1) as f32);
        let cell_h: f32 = row_heights[row..row + r_span].iter().sum::<f32>()
            + row_gap * (r_span.saturating_sub(1) as f32);

        let cs = engine.tree.get(cid).style.clone();
        let cctx = engine.calc_ctx(&cs, cell_w);
        let ml = cs.margin_left.resolve_or(&cctx, 0.0);
        let mt = cs.margin_top.resolve_or(&cctx, 0.0);

        let cur_bb = engine.tree.get(cid).dims.border_box();
        let free_x = (cell_w - cur_bb.width - ml).max(0.0);
        let free_y = (cell_h - cur_bb.height - mt).max(0.0);

        let ox = match style.justify_items {
            AlignItems::Center => free_x * 0.5,
            AlignItems::FlexEnd => free_x,
            _ => 0.0,
        };
        let oy = match style.align_items {
            AlignItems::Center => free_y * 0.5,
            AlignItems::FlexEnd => free_y,
            _ => 0.0,
        };

        let target_x = content_x + col_offsets[col] + ml + ox;
        let target_y = content_y + row_offsets[row] + mt + oy;
        engine.translate_subtree(cid, target_x - cur_bb.x, target_y - cur_bb.y);
        engine.apply_relative_offset(cid, content_w, explicit_h.unwrap_or(cb_height));
    }

    let total_h = row_heights.iter().sum::<f32>() + row_gap * (num_rows.saturating_sub(1) as f32);
    engine.tree.get_mut(id).dims.content.height = explicit_h.unwrap_or(total_h);

    for cid in abs_children {
        engine.layout_out_of_flow(cid, id);
    }
}

fn expand_tracks(tracks: &[GridTrackSize], available: f32, gap: f32) -> Vec<GridTrackSize> {
    let mut out = Vec::new();
    for t in tracks {
        match t {
            GridTrackSize::Repeat(count, inner) => {
                let reps = match count {
                    GridRepeatCount::Count(n) => (*n as usize).clamp(1, 64),
                    GridRepeatCount::AutoFill | GridRepeatCount::AutoFit => {
                        let min_w: f32 = inner
                            .iter()
                            .map(|tr| min_track_estimate(tr, available))
                            .sum::<f32>()
                            .max(40.0);
                        (((available + gap) / (min_w + gap)).floor() as usize).clamp(1, 32)
                    }
                };
                for _ in 0..reps {
                    for it in inner {
                        out.push(it.clone());
                    }
                }
            }
            other => out.push(other.clone()),
        }
    }
    if out.is_empty() {
        out.push(GridTrackSize::Fr(1.0));
    }
    out
}

fn min_track_estimate(t: &GridTrackSize, available: f32) -> f32 {
    match t {
        GridTrackSize::Px(v) => *v,
        GridTrackSize::Percent(p) => available * p / 100.0,
        GridTrackSize::MinMax(min_t, _) => min_track_estimate(min_t, available),
        GridTrackSize::FitContent(v) => *v,
        _ => 100.0,
    }
}

fn fixed_track_size(t: &GridTrackSize, available: f32) -> Option<f32> {
    match t {
        GridTrackSize::Px(v) => Some(*v),
        GridTrackSize::Percent(p) => Some(available * p / 100.0),
        _ => None,
    }
}

fn resolve_track_sizes(
    tracks: &[GridTrackSize],
    available: f32,
    gap: f32,
    num_cols: usize,
) -> Vec<f32> {
    let mut sizes = vec![0.0f32; num_cols];
    let mut fr_weights = vec![0.0f32; num_cols];
    let mut used = gap * (num_cols.saturating_sub(1) as f32);

    for (i, t) in tracks.iter().take(num_cols).enumerate() {
        match t {
            GridTrackSize::Px(v) => {
                sizes[i] = *v;
                used += *v;
            }
            GridTrackSize::Percent(p) => {
                let v = available * p / 100.0;
                sizes[i] = v;
                used += v;
            }
            GridTrackSize::Fr(w) => {
                fr_weights[i] = w.max(0.0);
            }
            GridTrackSize::MinMax(min_t, max_t) => {
                let min_v = fixed_track_size(min_t, available).unwrap_or(0.0);
                sizes[i] = min_v;
                used += min_v;
                if let GridTrackSize::Fr(w) = **max_t {
                    fr_weights[i] = w.max(0.0);
                } else if let Some(max_v) = fixed_track_size(max_t, available) {
                    let extra = (max_v - min_v).max(0.0);
                    sizes[i] += extra;
                    used += extra;
                } else {
                    fr_weights[i] = 1.0;
                }
            }
            GridTrackSize::FitContent(v) => {
                sizes[i] = (*v).min(available);
                used += sizes[i];
            }
            GridTrackSize::Auto | GridTrackSize::MinContent | GridTrackSize::MaxContent => {
                fr_weights[i] = 1.0;
            }
            GridTrackSize::Repeat(_, _) => {
                fr_weights[i] = 1.0;
            }
        }
    }

    let total_fr: f32 = fr_weights.iter().sum();
    let remaining = (available - used).max(0.0);
    if total_fr > 0.0 && remaining > 0.0 {
        for i in 0..num_cols {
            if fr_weights[i] > 0.0 {
                sizes[i] += remaining * (fr_weights[i] / total_fr);
            }
        }
    }
    sizes
}

fn resolve_placement(
    start: GridPlacement,
    end: GridPlacement,
    max_tracks: usize,
) -> (Option<usize>, usize) {
    let line_to_idx = |l: i32| -> usize {
        if l > 0 {
            (l - 1) as usize
        } else if l < 0 {
            (max_tracks as i32 + 1 + l).max(0) as usize
        } else {
            0
        }
    };
    match (start, end) {
        (GridPlacement::Line(s), GridPlacement::Line(e)) => {
            let si = line_to_idx(s);
            let ei = line_to_idx(e);
            (Some(si), ei.saturating_sub(si).max(1))
        }
        (GridPlacement::Line(s), GridPlacement::Span(sp)) => {
            (Some(line_to_idx(s)), (sp as usize).max(1))
        }
        (GridPlacement::Line(s), GridPlacement::Auto) => (Some(line_to_idx(s)), 1),
        (GridPlacement::Span(sp), _) | (GridPlacement::Auto, GridPlacement::Span(sp)) => {
            (None, (sp as usize).max(1))
        }
        (GridPlacement::Auto, GridPlacement::Line(e)) => {
            let ei = line_to_idx(e);
            (Some(ei.saturating_sub(1)), 1)
        }
        (GridPlacement::Auto, GridPlacement::Auto) => (None, 1),
    }
}

fn is_area_occupied(
    occ: &[Vec<bool>],
    row: usize,
    col: usize,
    r_span: usize,
    c_span: usize,
) -> bool {
    for r in row..row + r_span {
        if let Some(line) = occ.get(r) {
            for c in col..col + c_span {
                if line.get(c).copied().unwrap_or(false) {
                    return true;
                }
            }
        }
    }
    false
}

fn mark_occupied(
    occ: &mut Vec<Vec<bool>>,
    row: usize,
    col: usize,
    r_span: usize,
    c_span: usize,
    num_cols: usize,
) {
    while occ.len() < row + r_span {
        occ.push(vec![false; num_cols]);
    }
    for r in row..row + r_span {
        for c in col..(col + c_span).min(num_cols) {
            occ[r][c] = true;
        }
    }
}
