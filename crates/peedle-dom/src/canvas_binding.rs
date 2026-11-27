use std::rc::Rc;

use peedle_css::{parse_color, Color};
use peedle_html::NodeId;
use peedle_js::{JsRuntime, JsValue, ObjectId};
use peedle_render::{Canvas2D, RgbaColor};

use crate::events::define_getter_setter;
use crate::state::{Canvas2DHandle, SharedDom};

fn parse_canvas_color(s: &str) -> RgbaColor {
    if let Some(Color { r, g, b, a }) = parse_color(s) {
        RgbaColor::new(r, g, b, a)
    } else {
        RgbaColor::BLACK
    }
}

pub fn ensure_canvas(dom: &SharedDom, nid: NodeId) -> (u32, u32) {
    let mut st = dom.borrow_mut();
    let (w, h) = st
        .doc
        .element(nid)
        .map(|el| {
            let w = el
                .get_attr("width")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(300)
                .clamp(1, 4096);
            let h = el
                .get_attr("height")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(150)
                .clamp(1, 4096);
            (w, h)
        })
        .unwrap_or((300, 150));
    st.canvases.entry(nid).or_insert_with(|| Canvas2D::new(w, h));
    (w, h)
}

pub fn define_canvas2d_proto(rt: &mut JsRuntime, dom: SharedDom, proto: ObjectId) {
    let d = dom.clone();
    let set_fill = rt.create_native_function(
        "set fillStyle",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = this.as_object().and_then(|o| rt.get_host_data::<Canvas2DHandle>(o)).map(|h| h.0) else {
                return Ok(JsValue::Undefined);
            };
            let s = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            if let Some(cv) = d.borrow_mut().canvases.get_mut(&nid) {
                cv.state.fill_style = parse_canvas_color(&s);
            }
            Ok(JsValue::Undefined)
        }),
    );
    let d_g = dom.clone();
    let get_fill = rt.create_native_function(
        "get fillStyle",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = this.as_object().and_then(|o| rt.get_host_data::<Canvas2DHandle>(o)).map(|h| h.0) else {
                return Ok(JsValue::String("#000000".into()));
            };
            let s = d_g
                .borrow()
                .canvases
                .get(&nid)
                .map(|cv| {
                    let c = cv.state.fill_style;
                    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
                })
                .unwrap_or_else(|| "#000000".to_string());
            Ok(JsValue::String(s))
        }),
    );
    define_getter_setter(rt, proto, "fillStyle", Some(get_fill), Some(set_fill));

    let d = dom.clone();
    let set_stroke = rt.create_native_function(
        "set strokeStyle",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = this.as_object().and_then(|o| rt.get_host_data::<Canvas2DHandle>(o)).map(|h| h.0) else {
                return Ok(JsValue::Undefined);
            };
            let s = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            if let Some(cv) = d.borrow_mut().canvases.get_mut(&nid) {
                cv.state.stroke_style = parse_canvas_color(&s);
            }
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, proto, "strokeStyle", None, Some(set_stroke));

    let d = dom.clone();
    let set_lw = rt.create_native_function(
        "set lineWidth",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = this.as_object().and_then(|o| rt.get_host_data::<Canvas2DHandle>(o)).map(|h| h.0) else {
                return Ok(JsValue::Undefined);
            };
            let n = args.first().map(|v| rt.to_number(v)).transpose()?.unwrap_or(1.0) as f32;
            if let Some(cv) = d.borrow_mut().canvases.get_mut(&nid) {
                if n > 0.0 {
                    cv.state.line_width = n;
                }
            }
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, proto, "lineWidth", None, Some(set_lw));

    let d = dom.clone();
    let set_alpha = rt.create_native_function(
        "set globalAlpha",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = this.as_object().and_then(|o| rt.get_host_data::<Canvas2DHandle>(o)).map(|h| h.0) else {
                return Ok(JsValue::Undefined);
            };
            let n = args.first().map(|v| rt.to_number(v)).transpose()?.unwrap_or(1.0) as f32;
            if let Some(cv) = d.borrow_mut().canvases.get_mut(&nid) {
                cv.state.global_alpha = n.clamp(0.0, 1.0);
            }
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, proto, "globalAlpha", None, Some(set_alpha));

    macro_rules! cv_method {
        ($name:expr, $arity:expr, $body:expr) => {{
            let dm = dom.clone();
            let f = rt.create_native_function(
                $name,
                $arity,
                Rc::new(move |rt, this, args| {
                    let Some(nid) = this
                        .as_object()
                        .and_then(|o| rt.get_host_data::<Canvas2DHandle>(o))
                        .map(|h| h.0)
                    else {
                        return Ok(JsValue::Undefined);
                    };
                    ($body)(rt, &dm, nid, args)
                }),
            );
            let _ = rt.set_property(proto, $name, JsValue::Object(f));
        }};
    }

    cv_method!("fillRect", 4, |rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, args: &[JsValue]| {
        let x = arg_f32(rt, args, 0, 0.0);
        let y = arg_f32(rt, args, 1, 0.0);
        let w = arg_f32(rt, args, 2, 0.0);
        let h = arg_f32(rt, args, 3, 0.0);
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.fill_rect(x, y, w, h);
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("strokeRect", 4, |rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, args: &[JsValue]| {
        let x = arg_f32(rt, args, 0, 0.0);
        let y = arg_f32(rt, args, 1, 0.0);
        let w = arg_f32(rt, args, 2, 0.0);
        let h = arg_f32(rt, args, 3, 0.0);
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.stroke_rect(x, y, w, h);
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("clearRect", 4, |rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, args: &[JsValue]| {
        let x = arg_f32(rt, args, 0, 0.0);
        let y = arg_f32(rt, args, 1, 0.0);
        let w = arg_f32(rt, args, 2, 0.0);
        let h = arg_f32(rt, args, 3, 0.0);
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.clear_rect(x, y, w, h);
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("beginPath", 0, |_rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, _args: &[JsValue]| {
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.begin_path();
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("closePath", 0, |_rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, _args: &[JsValue]| {
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.close_path();
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("moveTo", 2, |rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, args: &[JsValue]| {
        let x = arg_f32(rt, args, 0, 0.0);
        let y = arg_f32(rt, args, 1, 0.0);
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.move_to(x, y);
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("lineTo", 2, |rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, args: &[JsValue]| {
        let x = arg_f32(rt, args, 0, 0.0);
        let y = arg_f32(rt, args, 1, 0.0);
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.line_to(x, y);
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("rect", 4, |rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, args: &[JsValue]| {
        let x = arg_f32(rt, args, 0, 0.0);
        let y = arg_f32(rt, args, 1, 0.0);
        let w = arg_f32(rt, args, 2, 0.0);
        let h = arg_f32(rt, args, 3, 0.0);
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.rect(x, y, w, h);
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("roundRect", 5, |rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, args: &[JsValue]| {
        let x = arg_f32(rt, args, 0, 0.0);
        let y = arg_f32(rt, args, 1, 0.0);
        let w = arg_f32(rt, args, 2, 0.0);
        let h = arg_f32(rt, args, 3, 0.0);
        let r = arg_f32(rt, args, 4, 0.0);
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.round_rect(x, y, w, h, r);
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("ellipse", 7, |rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, args: &[JsValue]| {
        let cx = arg_f32(rt, args, 0, 0.0);
        let cy = arg_f32(rt, args, 1, 0.0);
        let rx = arg_f32(rt, args, 2, 0.0);
        let ry = arg_f32(rt, args, 3, 0.0);
        let rot = arg_f32(rt, args, 4, 0.0);
        let sa = arg_f32(rt, args, 5, 0.0);
        let ea = arg_f32(rt, args, 6, std::f32::consts::TAU);
        let ccw = args.get(7).map_or(false, |v| rt.to_boolean(v));
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.ellipse(cx, cy, rx, ry, rot, sa, ea, ccw);
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("quadraticCurveTo", 4, |rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, args: &[JsValue]| {
        let cpx = arg_f32(rt, args, 0, 0.0);
        let cpy = arg_f32(rt, args, 1, 0.0);
        let x = arg_f32(rt, args, 2, 0.0);
        let y = arg_f32(rt, args, 3, 0.0);
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.quadratic_curve_to(cpx, cpy, x, y);
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("bezierCurveTo", 6, |rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, args: &[JsValue]| {
        let cp1x = arg_f32(rt, args, 0, 0.0);
        let cp1y = arg_f32(rt, args, 1, 0.0);
        let cp2x = arg_f32(rt, args, 2, 0.0);
        let cp2y = arg_f32(rt, args, 3, 0.0);
        let x = arg_f32(rt, args, 4, 0.0);
        let y = arg_f32(rt, args, 5, 0.0);
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.bezier_curve_to(cp1x, cp1y, cp2x, cp2y, x, y);
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("arc", 5, |rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, args: &[JsValue]| {
        let x = arg_f32(rt, args, 0, 0.0);
        let y = arg_f32(rt, args, 1, 0.0);
        let r = arg_f32(rt, args, 2, 0.0);
        let sa = arg_f32(rt, args, 3, 0.0);
        let ea = arg_f32(rt, args, 4, std::f32::consts::TAU);
        let ccw = args.get(5).map_or(false, |v| rt.to_boolean(v));
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.arc(x, y, r, sa, ea, ccw);
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("fill", 0, |rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, args: &[JsValue]| {
        let eo = args
            .first()
            .and_then(|v| v.as_str())
            .map_or(false, |s| s == "evenodd");
        let _ = rt;
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.fill(eo);
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("stroke", 0, |_rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, _args: &[JsValue]| {
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.stroke();
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("save", 0, |_rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, _args: &[JsValue]| {
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.save();
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("restore", 0, |_rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, _args: &[JsValue]| {
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.restore();
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("translate", 2, |rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, args: &[JsValue]| {
        let x = arg_f32(rt, args, 0, 0.0);
        let y = arg_f32(rt, args, 1, 0.0);
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.translate(x, y);
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("scale", 2, |rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, args: &[JsValue]| {
        let x = arg_f32(rt, args, 0, 1.0);
        let y = arg_f32(rt, args, 1, 1.0);
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.scale(x, y);
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("rotate", 1, |rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, args: &[JsValue]| {
        let a = arg_f32(rt, args, 0, 0.0);
        if let Some(cv) = dm.borrow_mut().canvases.get_mut(&nid) {
            cv.rotate(a);
        }
        Ok(JsValue::Undefined)
    });

    cv_method!("getImageData", 4, |rt: &mut JsRuntime, dm: &SharedDom, nid: NodeId, args: &[JsValue]| {
        let sx = arg_f32(rt, args, 0, 0.0) as i32;
        let sy = arg_f32(rt, args, 1, 0.0) as i32;
        let sw = arg_f32(rt, args, 2, 1.0).max(1.0) as u32;
        let sh = arg_f32(rt, args, 3, 1.0).max(1.0) as u32;
        let bytes = dm
            .borrow()
            .canvases
            .get(&nid)
            .map(|cv| cv.get_image_data(sx, sy, sw, sh))
            .unwrap_or_else(|| vec![0u8; (sw * sh * 4) as usize]);
        let arr = rt.create_array(bytes.into_iter().map(|b| JsValue::Number(b as f64)).collect());
        let obj = rt.create_object();
        let _ = rt.set_property(obj, "width", JsValue::Number(sw as f64));
        let _ = rt.set_property(obj, "height", JsValue::Number(sh as f64));
        let _ = rt.set_property(obj, "data", JsValue::Object(arr));
        Ok(JsValue::Object(obj))
    });

    cv_method!("measureText", 1, |rt: &mut JsRuntime, _dm: &SharedDom, _nid: NodeId, args: &[JsValue]| {
        let s = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
        let obj = rt.create_object();
        let _ = rt.set_property(obj, "width", JsValue::Number(s.chars().count() as f64 * 7.0));
        Ok(JsValue::Object(obj))
    });
}

fn arg_f32(rt: &mut JsRuntime, args: &[JsValue], idx: usize, default: f32) -> f32 {
    args.get(idx)
        .and_then(|v| rt.to_number(v).ok())
        .map(|n| n as f32)
        .unwrap_or(default)
}
