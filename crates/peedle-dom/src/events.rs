use std::rc::Rc;

use peedle_html::NodeId;
use peedle_js::{JsError, JsRuntime, JsValue, ObjectId, PropertyDescriptor};

use crate::nodes::wrap_node;
use crate::state::{EventData, EventListenerEntry, NodeHandle, SharedDom};

pub fn define_event_system(rt: &mut JsRuntime, dom: SharedDom, event_target_proto: ObjectId, event_proto: ObjectId) {
    let d1 = dom.clone();
    let add_fn = rt.create_native_function(
        "addEventListener",
        2,
        Rc::new(move |rt, this, args| {
            let etype = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let cb = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            if cb.is_nullish() {
                return Ok(JsValue::Undefined);
            }
            let (capture, once, passive) = parse_listener_options(rt, args.get(2))?;
            let target = extract_target_node(rt, this);
            d1.borrow_mut().add_listener(
                target,
                EventListenerEntry {
                    event_type: etype,
                    callback: cb,
                    capture,
                    once,
                    passive,
                },
            );
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(event_target_proto, "addEventListener", JsValue::Object(add_fn));

    let d2 = dom.clone();
    let rem_fn = rt.create_native_function(
        "removeEventListener",
        2,
        Rc::new(move |rt, this, args| {
            let etype = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let cb = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let (capture, _, _) = parse_listener_options(rt, args.get(2))?;
            let target = extract_target_node(rt, this);
            d2.borrow_mut().remove_listener(target, &etype, &cb, capture);
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(event_target_proto, "removeEventListener", JsValue::Object(rem_fn));

    let d3 = dom.clone();
    let disp_fn = rt.create_native_function(
        "dispatchEvent",
        1,
        Rc::new(move |rt, this, args| {
            let Some(ev_oid) = args.first().and_then(|v| v.as_object()) else {
                return Err(JsError::type_err("dispatchEvent requires an Event"));
            };
            let target = extract_target_node(rt, this);
            let ok = dispatch_event_object(rt, &d3, target, ev_oid)?;
            Ok(JsValue::Boolean(ok))
        }),
    );
    let _ = rt.set_property(event_target_proto, "dispatchEvent", JsValue::Object(disp_fn));

    let stop_prop = rt.create_native_function(
        "stopPropagation",
        0,
        Rc::new(|rt, this, _| {
            if let Some(oid) = this.as_object() {
                if let Some(ev) = rt.get_host_data_mut::<EventData>(oid) {
                    ev.propagation_stopped = true;
                }
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(event_proto, "stopPropagation", JsValue::Object(stop_prop));

    let stop_imm = rt.create_native_function(
        "stopImmediatePropagation",
        0,
        Rc::new(|rt, this, _| {
            if let Some(oid) = this.as_object() {
                if let Some(ev) = rt.get_host_data_mut::<EventData>(oid) {
                    ev.propagation_stopped = true;
                    ev.immediate_propagation_stopped = true;
                }
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(event_proto, "stopImmediatePropagation", JsValue::Object(stop_imm));

    let prev_def = rt.create_native_function(
        "preventDefault",
        0,
        Rc::new(|rt, this, _| {
            if let Some(oid) = this.as_object() {
                let mut set_flag = false;
                if let Some(ev) = rt.get_host_data_mut::<EventData>(oid) {
                    if ev.cancelable {
                        ev.default_prevented = true;
                        set_flag = true;
                    }
                }
                if set_flag {
                    let _ = rt.set_property(oid, "defaultPrevented", JsValue::Boolean(true));
                }
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(event_proto, "preventDefault", JsValue::Object(prev_def));

    let composed_path = rt.create_native_function(
        "composedPath",
        0,
        Rc::new(|rt, this, _| {
            let items = this
                .as_object()
                .and_then(|oid| rt.get_host_data::<EventData>(oid))
                .map(|ev| ev.path.clone())
                .unwrap_or_default();
            Ok(JsValue::Object(rt.create_array(items)))
        }),
    );
    let _ = rt.set_property(event_proto, "composedPath", JsValue::Object(composed_path));

    let init_ev = rt.create_native_function(
        "initEvent",
        1,
        Rc::new(|rt, this, args| {
            let etype = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let bubbles = args.get(1).map_or(false, |v| rt.to_boolean(v));
            let cancelable = args.get(2).map_or(false, |v| rt.to_boolean(v));
            if let Some(oid) = this.as_object() {
                if let Some(ev) = rt.get_host_data_mut::<EventData>(oid) {
                    ev.event_type = etype.clone();
                    ev.bubbles = bubbles;
                    ev.cancelable = cancelable;
                }
                let _ = rt.set_property(oid, "type", JsValue::String(etype));
                let _ = rt.set_property(oid, "bubbles", JsValue::Boolean(bubbles));
                let _ = rt.set_property(oid, "cancelable", JsValue::Boolean(cancelable));
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(event_proto, "initEvent", JsValue::Object(init_ev));

    for (name, num) in [
        ("NONE", 0.0),
        ("CAPTURING_PHASE", 1.0),
        ("AT_TARGET", 2.0),
        ("BUBBLING_PHASE", 3.0),
    ] {
        let _ = rt.set_property(event_proto, name, JsValue::Number(num));
    }

    let global = rt.global_object();
    let event_ctor = create_event_constructor(rt, "Event", event_proto, false);
    let _ = rt.set_property(global, "Event", JsValue::Object(event_ctor));
    for (name, num) in [
        ("NONE", 0.0),
        ("CAPTURING_PHASE", 1.0),
        ("AT_TARGET", 2.0),
        ("BUBBLING_PHASE", 3.0),
    ] {
        let _ = rt.set_property(event_ctor, name, JsValue::Number(num));
    }

    let custom_proto = rt.create_object_with_proto(Some(event_proto));
    let custom_ctor = create_event_constructor(rt, "CustomEvent", custom_proto, true);
    let _ = rt.set_property(global, "CustomEvent", JsValue::Object(custom_ctor));

    for sub in [
        "UIEvent",
        "MouseEvent",
        "KeyboardEvent",
        "InputEvent",
        "FocusEvent",
        "PointerEvent",
        "WheelEvent",
        "ErrorEvent",
        "HashChangeEvent",
        "PopStateEvent",
        "MessageEvent",
    ] {
        let sub_proto = rt.create_object_with_proto(Some(event_proto));
        let sub_ctor = create_event_constructor(rt, sub, sub_proto, false);
        let _ = rt.set_property(global, sub, JsValue::Object(sub_ctor));
    }

    let et_ctor = rt.create_native_constructor(
        "EventTarget",
        0,
        event_target_proto,
        Rc::new(move |rt, _, _| Ok(JsValue::Object(rt.create_object_with_proto(Some(event_target_proto))))),
    );
    let _ = rt.set_property(global, "EventTarget", JsValue::Object(et_ctor));
}

fn create_event_constructor(
    rt: &mut JsRuntime,
    name: &str,
    proto: ObjectId,
    is_custom: bool,
) -> ObjectId {
    rt.create_native_constructor(
        name,
        1,
        proto,
        Rc::new(move |rt, _, args| {
            let etype = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mut bubbles = false;
            let mut cancelable = false;
            let mut composed = false;
            let mut detail = JsValue::Null;
            let init_obj = args.get(1).and_then(|v| v.as_object());
            if let Some(init) = init_obj {
                bubbles = rt.get_property(init, "bubbles").map_or(false, |v| rt.to_boolean(&v));
                cancelable = rt.get_property(init, "cancelable").map_or(false, |v| rt.to_boolean(&v));
                composed = rt.get_property(init, "composed").map_or(false, |v| rt.to_boolean(&v));
                if is_custom {
                    detail = rt.get_property(init, "detail").unwrap_or(JsValue::Null);
                }
            }
            let obj = build_event_object(rt, proto, etype, bubbles, cancelable, composed);
            if is_custom {
                let _ = rt.set_property(obj, "detail", detail);
            }
            if let Some(init) = init_obj {
                if let Ok(keys) = rt.own_keys(init) {
                    for k in keys {
                        if let Some(ks) = k.as_str() {
                            if !matches!(ks, "bubbles" | "cancelable" | "composed" | "detail") {
                                if let Ok(v) = rt.get_property(init, ks) {
                                    let _ = rt.set_property(obj, ks, v);
                                }
                            }
                        }
                    }
                }
            }
            Ok(JsValue::Object(obj))
        }),
    )
}

pub fn build_event_object(
    rt: &mut JsRuntime,
    proto: ObjectId,
    event_type: String,
    bubbles: bool,
    cancelable: bool,
    composed: bool,
) -> ObjectId {
    let obj = rt.create_object_with_proto(Some(proto));
    rt.set_host_data(
        obj,
        EventData {
            event_type: event_type.clone(),
            bubbles,
            cancelable,
            composed,
            default_prevented: false,
            propagation_stopped: false,
            immediate_propagation_stopped: false,
            event_phase: 0,
            target: JsValue::Null,
            current_target: JsValue::Null,
            path: Vec::new(),
        },
    );
    let _ = rt.set_property(obj, "type", JsValue::String(event_type));
    let _ = rt.set_property(obj, "bubbles", JsValue::Boolean(bubbles));
    let _ = rt.set_property(obj, "cancelable", JsValue::Boolean(cancelable));
    let _ = rt.set_property(obj, "composed", JsValue::Boolean(composed));
    let _ = rt.set_property(obj, "defaultPrevented", JsValue::Boolean(false));
    let _ = rt.set_property(obj, "eventPhase", JsValue::Number(0.0));
    let _ = rt.set_property(obj, "target", JsValue::Null);
    let _ = rt.set_property(obj, "currentTarget", JsValue::Null);
    let _ = rt.set_property(obj, "isTrusted", JsValue::Boolean(true));
    let _ = rt.set_property(obj, "timeStamp", JsValue::Number(0.0));
    obj
}

pub fn dispatch_event_object(
    rt: &mut JsRuntime,
    dom: &SharedDom,
    target: Option<NodeId>,
    ev_oid: ObjectId,
) -> Result<bool, JsError> {
    let (event_type, bubbles) = match rt.get_host_data::<EventData>(ev_oid) {
        Some(ev) => (ev.event_type.clone(), ev.bubbles),
        None => {
            let et = rt
                .get_property(ev_oid, "type")
                .and_then(|v| rt.to_string(&v))
                .unwrap_or_default();
            let b = rt
                .get_property(ev_oid, "bubbles")
                .map_or(false, |v| rt.to_boolean(&v));
            rt.set_host_data(
                ev_oid,
                EventData {
                    event_type: et.clone(),
                    bubbles: b,
                    cancelable: false,
                    composed: false,
                    default_prevented: false,
                    propagation_stopped: false,
                    immediate_propagation_stopped: false,
                    event_phase: 0,
                    target: JsValue::Null,
                    current_target: JsValue::Null,
                    path: Vec::new(),
                },
            );
            (et, b)
        }
    };

    let mut chain: Vec<Option<NodeId>> = Vec::new();
    if let Some(start) = target {
        let mut cur = Some(start);
        while let Some(nid) = cur {
            chain.push(Some(nid));
            let parent = dom.borrow().doc.node(nid).parent;
            cur = parent;
        }
        chain.push(None);
    } else {
        chain.push(None);
    }

    let path_vals: Vec<JsValue> = chain
        .iter()
        .map(|opt| match opt {
            Some(nid) => JsValue::Object(wrap_node(rt, dom, *nid)),
            None => JsValue::Object(rt.global_object()),
        })
        .collect();
    let target_val = path_vals.first().cloned().unwrap_or(JsValue::Null);

    if let Some(ev) = rt.get_host_data_mut::<EventData>(ev_oid) {
        ev.target = target_val.clone();
        ev.path = path_vals.clone();
        ev.propagation_stopped = false;
        ev.immediate_propagation_stopped = false;
    }
    let _ = rt.set_property(ev_oid, "target", target_val.clone());
    let _ = rt.set_property(ev_oid, "srcElement", target_val);

    let capture_path: Vec<(Option<NodeId>, JsValue)> = chain
        .iter()
        .copied()
        .zip(path_vals.iter().cloned())
        .skip(1)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    for (node_opt, obj_val) in capture_path {
        if is_stopped(rt, ev_oid) {
            break;
        }
        invoke_listeners(rt, dom, node_opt, obj_val, ev_oid, &event_type, 1, true)?;
    }

    if !is_stopped(rt, ev_oid) {
        let target_obj = path_vals[0].clone();
        invoke_listeners(rt, dom, chain[0], target_obj.clone(), ev_oid, &event_type, 2, true)?;
        if !is_imm_stopped(rt, ev_oid) {
            invoke_listeners(rt, dom, chain[0], target_obj, ev_oid, &event_type, 2, false)?;
        }
    }

    if bubbles {
        for (node_opt, obj_val) in chain
            .iter()
            .copied()
            .zip(path_vals.iter().cloned())
            .skip(1)
        {
            if is_stopped(rt, ev_oid) {
                break;
            }
            invoke_listeners(rt, dom, node_opt, obj_val, ev_oid, &event_type, 3, false)?;
        }
    }

    if let Some(ev) = rt.get_host_data_mut::<EventData>(ev_oid) {
        ev.event_phase = 0;
        ev.current_target = JsValue::Null;
    }
    let _ = rt.set_property(ev_oid, "eventPhase", JsValue::Number(0.0));
    let _ = rt.set_property(ev_oid, "currentTarget", JsValue::Null);

    let prevented = rt
        .get_host_data::<EventData>(ev_oid)
        .map_or(false, |e| e.default_prevented);
    Ok(!prevented)
}

fn invoke_listeners(
    rt: &mut JsRuntime,
    dom: &SharedDom,
    node_opt: Option<NodeId>,
    current_target: JsValue,
    ev_oid: ObjectId,
    event_type: &str,
    phase: u8,
    capture_only: bool,
) -> Result<(), JsError> {
    if let Some(ev) = rt.get_host_data_mut::<EventData>(ev_oid) {
        ev.event_phase = phase;
        ev.current_target = current_target.clone();
    }
    let _ = rt.set_property(ev_oid, "eventPhase", JsValue::Number(phase as f64));
    let _ = rt.set_property(ev_oid, "currentTarget", current_target.clone());

    let snapshot: Vec<EventListenerEntry> = {
        let st = dom.borrow();
        let list = match node_opt {
            Some(nid) => st.listeners.get(&nid),
            None => Some(&st.window_listeners),
        };
        list.map(|l| {
            l.iter()
                .filter(|e| e.event_type == event_type && e.capture == capture_only)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
    };

    for entry in snapshot {
        if is_imm_stopped(rt, ev_oid) {
            break;
        }
        if entry.once {
            dom.borrow_mut().remove_listener(
                node_opt,
                &entry.event_type,
                &entry.callback,
                entry.capture,
            );
        }
        if rt.is_callable(&entry.callback) {
            let _ = rt.call_function(
                entry.callback,
                current_target.clone(),
                &[JsValue::Object(ev_oid)],
            );
        } else if let Some(handler_oid) = entry.callback.as_object() {
            if let Ok(he) = rt.get_property(handler_oid, "handleEvent") {
                if rt.is_callable(&he) {
                    let _ = rt.call_function(
                        he,
                        JsValue::Object(handler_oid),
                        &[JsValue::Object(ev_oid)],
                    );
                }
            }
        }
    }

    if !capture_only && !is_imm_stopped(rt, ev_oid) {
        let mut invoked_prop = false;
        if let Some(ct_oid) = current_target.as_object() {
            let on_prop = format!("on{}", event_type);
            if let Ok(handler) = rt.get_property(ct_oid, &on_prop) {
                if rt.is_callable(&handler) {
                    invoked_prop = true;
                    let res = rt.call_function(
                        handler,
                        current_target.clone(),
                        &[JsValue::Object(ev_oid)],
                    );
                    if let Ok(JsValue::Boolean(false)) = res {
                        if let Some(ev) = rt.get_host_data_mut::<EventData>(ev_oid) {
                            if ev.cancelable {
                                ev.default_prevented = true;
                            }
                        }
                    }
                }
            }
        }
        if !invoked_prop {
            if let Some(nid) = node_opt {
                let on_attr = format!("on{}", event_type);
                let attr_code = dom
                    .borrow()
                    .doc
                    .element(nid)
                    .and_then(|e| e.get_attr(&on_attr))
                    .map(|s| s.to_string());
                if let Some(code) = attr_code {
                    let wrapped = format!("(function(event){{ {} }})", code);
                    if let Ok(fval) = rt.eval(&wrapped) {
                        if rt.is_callable(&fval) {
                            let _ = rt.call_function(
                                fval,
                                current_target.clone(),
                                &[JsValue::Object(ev_oid)],
                            );
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn is_stopped(rt: &JsRuntime, ev_oid: ObjectId) -> bool {
    rt.get_host_data::<EventData>(ev_oid)
        .map_or(false, |e| e.propagation_stopped || e.immediate_propagation_stopped)
}

fn is_imm_stopped(rt: &JsRuntime, ev_oid: ObjectId) -> bool {
    rt.get_host_data::<EventData>(ev_oid)
        .map_or(false, |e| e.immediate_propagation_stopped)
}

fn parse_listener_options(
    rt: &mut JsRuntime,
    arg: Option<&JsValue>,
) -> Result<(bool, bool, bool), JsError> {
    let Some(v) = arg else {
        return Ok((false, false, false));
    };
    if let Some(b) = v.as_bool() {
        return Ok((b, false, false));
    }
    if let Some(oid) = v.as_object() {
        let capture = rt.get_property(oid, "capture").map_or(false, |x| rt.to_boolean(&x));
        let once = rt.get_property(oid, "once").map_or(false, |x| rt.to_boolean(&x));
        let passive = rt.get_property(oid, "passive").map_or(false, |x| rt.to_boolean(&x));
        return Ok((capture, once, passive));
    }
    Ok((false, false, false))
}

fn extract_target_node(rt: &JsRuntime, this: JsValue) -> Option<NodeId> {
    let oid = this.as_object()?;
    rt.get_host_data::<NodeHandle>(oid).map(|h| h.0)
}

pub fn define_getter_setter(
    rt: &mut JsRuntime,
    proto: ObjectId,
    name: &str,
    get: Option<ObjectId>,
    set: Option<ObjectId>,
) {
    let key = peedle_js::PropertyKey::from(name);
    let _ = rt.define_property(
        proto,
        key,
        PropertyDescriptor {
            value: None,
            writable: None,
            get: get.map(JsValue::Object),
            set: set.map(JsValue::Object),
            enumerable: Some(true),
            configurable: Some(true),
        },
    );
}
