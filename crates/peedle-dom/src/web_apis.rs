use std::rc::Rc;

use peedle_css::{get_computed_property, parse_media_query_list, Viewport};
use peedle_html::parse_html;
use peedle_js::{JsError, JsRuntime, JsValue, ObjectId};
use peedle_net::{HttpMethod, HttpRequest};
use peedle_url::{Url, UrlSearchParams};
use sha1::Digest as Sha1Digest;
use sha2::{Sha256, Sha384, Sha512};

use crate::events::define_getter_setter;
use crate::nodes::{node_id_of, wrap_node};
use crate::state::{MutationObserverEntry, MutationTarget, SharedDom, TimerEntry};

pub fn define_web_apis(rt: &mut JsRuntime, dom: SharedDom) {
    let global = rt.global_object();

    let _ = rt.set_property(global, "window", JsValue::Object(global));
    let _ = rt.set_property(global, "self", JsValue::Object(global));
    let _ = rt.set_property(global, "top", JsValue::Object(global));
    let _ = rt.set_property(global, "parent", JsValue::Object(global));
    let _ = rt.set_property(global, "frames", JsValue::Object(global));

    let d = dom.clone();
    let get_iw = rt.create_native_function(
        "get innerWidth",
        0,
        Rc::new(move |_, _, _| Ok(JsValue::Number(d.borrow().viewport_w as f64))),
    );
    let d = dom.clone();
    let get_ih = rt.create_native_function(
        "get innerHeight",
        0,
        Rc::new(move |_, _, _| Ok(JsValue::Number(d.borrow().viewport_h as f64))),
    );
    define_getter_setter(rt, global, "innerWidth", Some(get_iw), None);
    define_getter_setter(rt, global, "innerHeight", Some(get_ih), None);
    define_getter_setter(rt, global, "outerWidth", Some(get_iw), None);
    define_getter_setter(rt, global, "outerHeight", Some(get_ih), None);
    let _ = rt.set_property(global, "devicePixelRatio", JsValue::Number(1.0));

    define_timers_and_channels(rt, dom.clone(), global);
    define_location_and_history(rt, dom.clone(), global);
    define_navigator_screen_perf(rt, dom.clone(), global);
    define_storage(rt, dom.clone(), global);
    define_url_apis(rt, dom.clone(), global);
    define_fetch_and_xhr(rt, dom.clone(), global);
    define_observers_and_misc(rt, dom.clone(), global);
    define_crypto(rt, global);
}

fn define_timers_and_channels(rt: &mut JsRuntime, dom: SharedDom, global: ObjectId) {
    let d = dom.clone();
    let set_timeout = rt.create_native_function(
        "setTimeout",
        2,
        Rc::new(move |rt, _, args| {
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            let delay = args
                .get(1)
                .and_then(|v| rt.to_number(v).ok())
                .unwrap_or(0.0)
                .max(0.0) as u64;
            let extra = if args.len() > 2 {
                args[2..].to_vec()
            } else {
                Vec::new()
            };
            let mut st = d.borrow_mut();
            let id = st.next_timer_id;
            st.next_timer_id += 1;
            let due = st.now_ms + delay;
            st.timers.push(TimerEntry {
                id,
                callback: cb,
                args: extra,
                delay_ms: delay,
                due_ms: due,
                repeat: false,
                cancelled: false,
                is_raf: false,
            });
            Ok(JsValue::Number(id as f64))
        }),
    );
    let _ = rt.set_property(global, "setTimeout", JsValue::Object(set_timeout));

    let d = dom.clone();
    let set_interval = rt.create_native_function(
        "setInterval",
        2,
        Rc::new(move |rt, _, args| {
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            let delay = args
                .get(1)
                .and_then(|v| rt.to_number(v).ok())
                .unwrap_or(10.0)
                .max(4.0) as u64;
            let extra = if args.len() > 2 {
                args[2..].to_vec()
            } else {
                Vec::new()
            };
            let mut st = d.borrow_mut();
            let id = st.next_timer_id;
            st.next_timer_id += 1;
            let due = st.now_ms + delay;
            st.timers.push(TimerEntry {
                id,
                callback: cb,
                args: extra,
                delay_ms: delay,
                due_ms: due,
                repeat: true,
                cancelled: false,
                is_raf: false,
            });
            Ok(JsValue::Number(id as f64))
        }),
    );
    let _ = rt.set_property(global, "setInterval", JsValue::Object(set_interval));

    let d = dom.clone();
    let clear_timer = rt.create_native_function(
        "clearTimeout",
        1,
        Rc::new(move |rt, _, args| {
            let id = args.first().and_then(|v| rt.to_number(v).ok()).unwrap_or(0.0) as u32;
            for t in &mut d.borrow_mut().timers {
                if t.id == id {
                    t.cancelled = true;
                }
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(global, "clearTimeout", JsValue::Object(clear_timer));
    let _ = rt.set_property(global, "clearInterval", JsValue::Object(clear_timer));
    let _ = rt.set_property(global, "cancelAnimationFrame", JsValue::Object(clear_timer));

    let d = dom.clone();
    let raf = rt.create_native_function(
        "requestAnimationFrame",
        1,
        Rc::new(move |_, _, args| {
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            let mut st = d.borrow_mut();
            let id = st.next_timer_id;
            st.next_timer_id += 1;
            let due = st.now_ms + 16;
            st.timers.push(TimerEntry {
                id,
                callback: cb,
                args: vec![JsValue::Number(due as f64)],
                delay_ms: 16,
                due_ms: due,
                repeat: false,
                cancelled: false,
                is_raf: true,
            });
            Ok(JsValue::Number(id as f64))
        }),
    );
    let _ = rt.set_property(global, "requestAnimationFrame", JsValue::Object(raf));

    let mc_proto = rt.create_object();
    let d = dom.clone();
    let mc_ctor = rt.create_native_constructor(
        "MessageChannel",
        0,
        mc_proto,
        Rc::new(move |rt, _, _| {
            let chan = rt.create_object_with_proto(Some(mc_proto));
            let port1 = rt.create_object();
            let port2 = rt.create_object();
            let _ = rt.set_property(port1, "onmessage", JsValue::Null);
            let _ = rt.set_property(port2, "onmessage", JsValue::Null);

            let d1 = d.clone();
            let post1 = rt.create_native_function(
                "postMessage",
                1,
                Rc::new(move |rt, _, args| {
                    let data = args.first().cloned().unwrap_or(JsValue::Undefined);
                    let ev = rt.create_object();
                    let _ = rt.set_property(ev, "data", data);
                    d1.borrow_mut()
                        .message_queue
                        .push((JsValue::Object(port2), JsValue::Object(ev)));
                    Ok(JsValue::Undefined)
                }),
            );
            let d2 = d.clone();
            let post2 = rt.create_native_function(
                "postMessage",
                1,
                Rc::new(move |rt, _, args| {
                    let data = args.first().cloned().unwrap_or(JsValue::Undefined);
                    let ev = rt.create_object();
                    let _ = rt.set_property(ev, "data", data);
                    d2.borrow_mut()
                        .message_queue
                        .push((JsValue::Object(port1), JsValue::Object(ev)));
                    Ok(JsValue::Undefined)
                }),
            );
            let _ = rt.set_property(port1, "postMessage", JsValue::Object(post1));
            let _ = rt.set_property(port2, "postMessage", JsValue::Object(post2));
            let noop = rt.create_native_function("close", 0, Rc::new(|_, _, _| Ok(JsValue::Undefined)));
            let _ = rt.set_property(port1, "close", JsValue::Object(noop));
            let _ = rt.set_property(port2, "close", JsValue::Object(noop));
            let _ = rt.set_property(port1, "start", JsValue::Object(noop));
            let _ = rt.set_property(port2, "start", JsValue::Object(noop));

            let _ = rt.set_property(chan, "port1", JsValue::Object(port1));
            let _ = rt.set_property(chan, "port2", JsValue::Object(port2));
            Ok(JsValue::Object(chan))
        }),
    );
    let _ = rt.set_property(global, "MessageChannel", JsValue::Object(mc_ctor));
}

fn define_location_and_history(rt: &mut JsRuntime, dom: SharedDom, global: ObjectId) {
    let loc = rt.create_object();
    for (prop, part) in [
        ("href", 0u8),
        ("protocol", 1),
        ("host", 2),
        ("hostname", 3),
        ("port", 4),
        ("pathname", 5),
        ("search", 6),
        ("hash", 7),
        ("origin", 8),
    ] {
        let d = dom.clone();
        let gf = rt.create_native_function(
            prop,
            0,
            Rc::new(move |_, _, _| {
                let raw = d.borrow().url.clone();
                let val = match Url::parse(&raw) {
                    Ok(u) => match part {
                        0 => u.href(),
                        1 => u.protocol(),
                        2 => u.host_str(),
                        3 => u.hostname(),
                        4 => u.port_str(),
                        5 => u.pathname(),
                        6 => u.search(),
                        7 => u.hash(),
                        _ => u.origin(),
                    },
                    Err(_) => {
                        if part == 0 {
                            raw
                        } else {
                            String::new()
                        }
                    }
                };
                Ok(JsValue::String(val))
            }),
        );
        let setter = if part == 0 {
            let ds = dom.clone();
            Some(rt.create_native_function(
                "set href",
                1,
                Rc::new(move |rt, _, args| {
                    let next = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
                    ds.borrow_mut().pending_navigation = Some(next);
                    Ok(JsValue::Undefined)
                }),
            ))
        } else {
            None
        };
        define_getter_setter(rt, loc, prop, Some(gf), setter);
    }
    let d = dom.clone();
    let to_str = rt.create_native_function(
        "toString",
        0,
        Rc::new(move |_, _, _| Ok(JsValue::String(d.borrow().url.clone()))),
    );
    let _ = rt.set_property(loc, "toString", JsValue::Object(to_str));
    let _ = rt.set_property(global, "location", JsValue::Object(loc));

    let hist = rt.create_object();
    let d = dom.clone();
    let get_hlen = rt.create_native_function(
        "get length",
        0,
        Rc::new(move |_, _, _| Ok(JsValue::Number(d.borrow().history_stack.len() as f64))),
    );
    define_getter_setter(rt, hist, "length", Some(get_hlen), None);
    let d = dom.clone();
    let get_hstate = rt.create_native_function(
        "get state",
        0,
        Rc::new(move |_, _, _| {
            let st = d.borrow();
            Ok(st
                .history_stack
                .get(st.history_index)
                .map(|(_, s)| s.clone())
                .unwrap_or(JsValue::Null))
        }),
    );
    define_getter_setter(rt, hist, "state", Some(get_hstate), None);

    let d = dom.clone();
    let push_state = rt.create_native_function(
        "pushState",
        3,
        Rc::new(move |rt, _, args| {
            let state = args.first().cloned().unwrap_or(JsValue::Null);
            let url_arg = args.get(2).and_then(|v| {
                if v.is_nullish() {
                    None
                } else {
                    rt.to_string(v).ok()
                }
            });
            let mut st = d.borrow_mut();
            let new_url = if let Some(rel) = url_arg {
                Url::parse(&st.url)
                    .ok()
                    .and_then(|b| b.join(&rel).ok())
                    .map(|u| u.href())
                    .unwrap_or(rel)
            } else {
                st.url.clone()
            };
            st.url = new_url.clone();
            let next_len = st.history_index + 1;
            st.history_stack.truncate(next_len);
            st.history_stack.push((new_url, state));
            st.history_index = st.history_stack.len() - 1;
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(hist, "pushState", JsValue::Object(push_state));

    let d = dom.clone();
    let replace_state = rt.create_native_function(
        "replaceState",
        3,
        Rc::new(move |rt, _, args| {
            let state = args.first().cloned().unwrap_or(JsValue::Null);
            let url_arg = args.get(2).and_then(|v| {
                if v.is_nullish() {
                    None
                } else {
                    rt.to_string(v).ok()
                }
            });
            let mut st = d.borrow_mut();
            let new_url = if let Some(rel) = url_arg {
                Url::parse(&st.url)
                    .ok()
                    .and_then(|b| b.join(&rel).ok())
                    .map(|u| u.href())
                    .unwrap_or(rel)
            } else {
                st.url.clone()
            };
            st.url = new_url.clone();
            let idx = st.history_index;
            if let Some(slot) = st.history_stack.get_mut(idx) {
                *slot = (new_url, state);
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(hist, "replaceState", JsValue::Object(replace_state));
    let _ = rt.set_property(global, "history", JsValue::Object(hist));
}

fn define_navigator_screen_perf(rt: &mut JsRuntime, dom: SharedDom, global: ObjectId) {
    let nav = rt.create_object();
    let _ = rt.set_property(
        nav,
        "userAgent",
        JsValue::String("Mozilla/5.0 (X11; Linux x86_64) Peedle/0.1".into()),
    );
    let _ = rt.set_property(nav, "language", JsValue::String("en-US".into()));
    let langs = rt.create_array(vec![JsValue::String("en-US".into()), JsValue::String("en".into())]);
    let _ = rt.set_property(nav, "languages", JsValue::Object(langs));
    let _ = rt.set_property(nav, "platform", JsValue::String("Linux x86_64".into()));
    let _ = rt.set_property(nav, "vendor", JsValue::String("Peedle".into()));
    let _ = rt.set_property(nav, "cookieEnabled", JsValue::Boolean(true));
    let _ = rt.set_property(nav, "onLine", JsValue::Boolean(true));
    let _ = rt.set_property(nav, "hardwareConcurrency", JsValue::Number(8.0));
    let _ = rt.set_property(global, "navigator", JsValue::Object(nav));

    let screen = rt.create_object();
    let _ = rt.set_property(screen, "width", JsValue::Number(1920.0));
    let _ = rt.set_property(screen, "height", JsValue::Number(1080.0));
    let _ = rt.set_property(screen, "availWidth", JsValue::Number(1920.0));
    let _ = rt.set_property(screen, "availHeight", JsValue::Number(1040.0));
    let _ = rt.set_property(screen, "colorDepth", JsValue::Number(24.0));
    let _ = rt.set_property(screen, "pixelDepth", JsValue::Number(24.0));
    let _ = rt.set_property(global, "screen", JsValue::Object(screen));

    let perf = rt.create_object();
    let start = std::time::Instant::now();
    let now_fn = rt.create_native_function(
        "now",
        0,
        Rc::new(move |_, _, _| Ok(JsValue::Number(start.elapsed().as_secs_f64() * 1000.0))),
    );
    let _ = rt.set_property(perf, "now", JsValue::Object(now_fn));
    let _ = rt.set_property(perf, "timeOrigin", JsValue::Number(0.0));
    let noop = rt.create_native_function("noop", 0, Rc::new(|_, _, _| Ok(JsValue::Undefined)));
    let _ = rt.set_property(perf, "mark", JsValue::Object(noop));
    let _ = rt.set_property(perf, "measure", JsValue::Object(noop));
    let _ = rt.set_property(global, "performance", JsValue::Object(perf));

    let _ = dom;
}

fn define_storage(rt: &mut JsRuntime, dom: SharedDom, global: ObjectId) {
    for (name, is_local) in [("localStorage", true), ("sessionStorage", false)] {
        let obj = rt.create_object();
        let d = dom.clone();
        let get_item = rt.create_native_function(
            "getItem",
            1,
            Rc::new(move |rt, _, args| {
                let k = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
                let st = d.borrow();
                let map = if is_local { &st.local_storage } else { &st.session_storage };
                Ok(map
                    .get(&k)
                    .map(|v| JsValue::String(v.clone()))
                    .unwrap_or(JsValue::Null))
            }),
        );
        let d = dom.clone();
        let set_item = rt.create_native_function(
            "setItem",
            2,
            Rc::new(move |rt, _, args| {
                let k = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
                let v = args.get(1).map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
                let mut st = d.borrow_mut();
                let map = if is_local { &mut st.local_storage } else { &mut st.session_storage };
                map.insert(k, v);
                Ok(JsValue::Undefined)
            }),
        );
        let d = dom.clone();
        let rem_item = rt.create_native_function(
            "removeItem",
            1,
            Rc::new(move |rt, _, args| {
                let k = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
                let mut st = d.borrow_mut();
                let map = if is_local { &mut st.local_storage } else { &mut st.session_storage };
                map.remove(&k);
                Ok(JsValue::Undefined)
            }),
        );
        let d = dom.clone();
        let clear = rt.create_native_function(
            "clear",
            0,
            Rc::new(move |_, _, _| {
                let mut st = d.borrow_mut();
                if is_local {
                    st.local_storage.clear();
                } else {
                    st.session_storage.clear();
                }
                Ok(JsValue::Undefined)
            }),
        );
        let d = dom.clone();
        let get_len = rt.create_native_function(
            "get length",
            0,
            Rc::new(move |_, _, _| {
                let st = d.borrow();
                let n = if is_local {
                    st.local_storage.len()
                } else {
                    st.session_storage.len()
                };
                Ok(JsValue::Number(n as f64))
            }),
        );
        let _ = rt.set_property(obj, "getItem", JsValue::Object(get_item));
        let _ = rt.set_property(obj, "setItem", JsValue::Object(set_item));
        let _ = rt.set_property(obj, "removeItem", JsValue::Object(rem_item));
        let _ = rt.set_property(obj, "clear", JsValue::Object(clear));
        define_getter_setter(rt, obj, "length", Some(get_len), None);
        let _ = rt.set_property(global, name, JsValue::Object(obj));
    }
}

fn define_url_apis(rt: &mut JsRuntime, dom: SharedDom, global: ObjectId) {
    let protos = dom.borrow().protos.unwrap();
    let usp_proto = protos.url_search_params_proto;
    let url_proto = protos.url_proto;

    let usp_get = rt.create_native_function(
        "get",
        1,
        Rc::new(|rt, this, args| {
            let k = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let res = this
                .as_object()
                .and_then(|o| rt.get_host_data::<UrlSearchParams>(o))
                .and_then(|p| p.get(&k).map(|s| JsValue::String(s.to_string())))
                .unwrap_or(JsValue::Null);
            Ok(res)
        }),
    );
    let usp_set = rt.create_native_function(
        "set",
        2,
        Rc::new(|rt, this, args| {
            let k = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let v = args.get(1).map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            if let Some(o) = this.as_object() {
                if let Some(p) = rt.get_host_data_mut::<UrlSearchParams>(o) {
                    p.set(k, v);
                }
            }
            Ok(JsValue::Undefined)
        }),
    );
    let usp_append = rt.create_native_function(
        "append",
        2,
        Rc::new(|rt, this, args| {
            let k = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let v = args.get(1).map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            if let Some(o) = this.as_object() {
                if let Some(p) = rt.get_host_data_mut::<UrlSearchParams>(o) {
                    p.append(k, v);
                }
            }
            Ok(JsValue::Undefined)
        }),
    );
    let usp_has = rt.create_native_function(
        "has",
        1,
        Rc::new(|rt, this, args| {
            let k = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let ok = this
                .as_object()
                .and_then(|o| rt.get_host_data::<UrlSearchParams>(o))
                .map_or(false, |p| p.has(&k, None));
            Ok(JsValue::Boolean(ok))
        }),
    );
    let usp_delete = rt.create_native_function(
        "delete",
        1,
        Rc::new(|rt, this, args| {
            let k = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            if let Some(o) = this.as_object() {
                if let Some(p) = rt.get_host_data_mut::<UrlSearchParams>(o) {
                    p.delete(&k, None);
                }
            }
            Ok(JsValue::Undefined)
        }),
    );
    let usp_to_string = rt.create_native_function(
        "toString",
        0,
        Rc::new(|rt, this, _| {
            let s = this
                .as_object()
                .and_then(|o| rt.get_host_data::<UrlSearchParams>(o))
                .map(|p| p.serialize())
                .unwrap_or_default();
            Ok(JsValue::String(s))
        }),
    );
    let _ = rt.set_property(usp_proto, "get", JsValue::Object(usp_get));
    let _ = rt.set_property(usp_proto, "set", JsValue::Object(usp_set));
    let _ = rt.set_property(usp_proto, "append", JsValue::Object(usp_append));
    let _ = rt.set_property(usp_proto, "has", JsValue::Object(usp_has));
    let _ = rt.set_property(usp_proto, "delete", JsValue::Object(usp_delete));
    let _ = rt.set_property(usp_proto, "toString", JsValue::Object(usp_to_string));

    let usp_ctor = rt.create_native_constructor(
        "URLSearchParams",
        0,
        usp_proto,
        Rc::new(move |rt, _, args| {
            let init = args
                .first()
                .map(|v| {
                    if v.is_undefined() {
                        Ok(String::new())
                    } else {
                        rt.to_string(v)
                    }
                })
                .transpose()?
                .unwrap_or_default();
            let obj = rt.create_object_with_proto(Some(usp_proto));
            rt.set_host_data(obj, UrlSearchParams::parse(&init));
            Ok(JsValue::Object(obj))
        }),
    );
    let _ = rt.set_property(global, "URLSearchParams", JsValue::Object(usp_ctor));

    let url_to_str = rt.create_native_function(
        "toString",
        0,
        Rc::new(|rt, this, _| {
            let s = this
                .as_object()
                .and_then(|o| rt.get_host_data::<Url>(o))
                .map(|u| u.href())
                .unwrap_or_default();
            Ok(JsValue::String(s))
        }),
    );
    let _ = rt.set_property(url_proto, "toString", JsValue::Object(url_to_str));

    let url_ctor = rt.create_native_constructor(
        "URL",
        1,
        url_proto,
        Rc::new(move |rt, _, args| {
            let input = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let base = args
                .get(1)
                .filter(|v| !v.is_undefined())
                .map(|v| rt.to_string(v))
                .transpose()?;
            let parsed = if let Some(b) = base {
                let bu = Url::parse(&b).map_err(|_| JsError::type_err("Invalid base URL"))?;
                Url::parse_with_base(&input, Some(&bu))
            } else {
                Url::parse(&input)
            }
            .map_err(|_| JsError::type_err(format!("Invalid URL: {}", input)))?;

            let obj = rt.create_object_with_proto(Some(url_proto));
            let _ = rt.set_property(obj, "href", JsValue::String(parsed.href()));
            let _ = rt.set_property(obj, "protocol", JsValue::String(parsed.protocol()));
            let _ = rt.set_property(obj, "username", JsValue::String(parsed.username.clone()));
            let _ = rt.set_property(obj, "password", JsValue::String(parsed.password.clone()));
            let _ = rt.set_property(obj, "host", JsValue::String(parsed.host_str()));
            let _ = rt.set_property(obj, "hostname", JsValue::String(parsed.hostname()));
            let _ = rt.set_property(obj, "port", JsValue::String(parsed.port_str()));
            let _ = rt.set_property(obj, "pathname", JsValue::String(parsed.pathname()));
            let _ = rt.set_property(obj, "search", JsValue::String(parsed.search()));
            let _ = rt.set_property(obj, "hash", JsValue::String(parsed.hash()));
            let _ = rt.set_property(obj, "origin", JsValue::String(parsed.origin()));
            let sp = rt.create_object_with_proto(Some(usp_proto));
            rt.set_host_data(sp, parsed.search_params());
            let _ = rt.set_property(obj, "searchParams", JsValue::Object(sp));
            rt.set_host_data(obj, parsed);
            Ok(JsValue::Object(obj))
        }),
    );
    let _ = rt.set_property(global, "URL", JsValue::Object(url_ctor));
}

fn define_fetch_and_xhr(rt: &mut JsRuntime, dom: SharedDom, global: ObjectId) {
    let d = dom.clone();
    let fetch_fn = rt.create_native_function(
        "fetch",
        1,
        Rc::new(move |rt, _, args| {
            let promise = rt.create_promise();
            let raw_url = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let resolved = {
                let st = d.borrow();
                Url::parse(&st.url)
                    .ok()
                    .and_then(|b| b.join(&raw_url).ok())
                    .or_else(|| Url::parse(&raw_url).ok())
            };
            let Some(url) = resolved else {
                let err = JsValue::Object(rt.create_error_object(
                    peedle_js::ErrorKind::TypeError,
                    "Invalid fetch URL",
                ));
                rt.reject_promise(promise, err);
                return Ok(JsValue::Object(promise));
            };
            let mut req = HttpRequest::get(url);
            if let Some(init) = args.get(1).and_then(|v| v.as_object()) {
                if let Ok(m) = rt.get_property(init, "method") {
                    if !m.is_undefined() {
                        if let Ok(ms) = rt.to_string(&m) {
                            req.method = HttpMethod::from_str_method(&ms);
                        }
                    }
                }
                if let Ok(b) = rt.get_property(init, "body") {
                    if !b.is_nullish() {
                        if let Ok(bs) = rt.to_string(&b) {
                            req.body = Some(bs.into_bytes());
                        }
                    }
                }
            }
            let client = d.borrow().client.clone();
            match client.send(req) {
                Ok(resp) => {
                    let ro = rt.create_object();
                    let _ = rt.set_property(ro, "status", JsValue::Number(resp.status as f64));
                    let _ = rt.set_property(ro, "statusText", JsValue::String(resp.status_text.clone()));
                    let _ = rt.set_property(ro, "ok", JsValue::Boolean((200..300).contains(&resp.status)));
                    let _ = rt.set_property(ro, "url", JsValue::String(resp.url.href()));
                    let body_text = resp.text();
                    let bt1 = body_text.clone();
                    let text_fn = rt.create_native_function(
                        "text",
                        0,
                        Rc::new(move |rt, _, _| {
                            let p = rt.create_promise();
                            rt.resolve_promise(p, JsValue::String(bt1.clone()));
                            Ok(JsValue::Object(p))
                        }),
                    );
                    let bt2 = body_text.clone();
                    let json_fn = rt.create_native_function(
                        "json",
                        0,
                        Rc::new(move |rt, _, _| {
                            let p = rt.create_promise();
                            let parse_fn = rt
                                .get_property(rt.global_object(), "JSON")
                                .ok()
                                .and_then(|j| j.as_object())
                                .and_then(|jo| rt.get_property(jo, "parse").ok());
                            if let Some(pf) = parse_fn {
                                match rt.call_function(pf, JsValue::Undefined, &[JsValue::String(bt2.clone())]) {
                                    Ok(v) => rt.resolve_promise(p, v),
                                    Err(e) => {
                                        let ev = rt.error_to_js_value(e);
                                        rt.reject_promise(p, ev);
                                    }
                                }
                            }
                            Ok(JsValue::Object(p))
                        }),
                    );
                    let _ = rt.set_property(ro, "text", JsValue::Object(text_fn));
                    let _ = rt.set_property(ro, "json", JsValue::Object(json_fn));
                    rt.resolve_promise(promise, JsValue::Object(ro));
                }
                Err(e) => {
                    let err = JsValue::Object(rt.create_error_object(
                        peedle_js::ErrorKind::TypeError,
                        &e.to_string(),
                    ));
                    rt.reject_promise(promise, err);
                }
            }
            Ok(JsValue::Object(promise))
        }),
    );
    let _ = rt.set_property(global, "fetch", JsValue::Object(fetch_fn));

    let ac_proto = rt.create_object();
    let ac_ctor = rt.create_native_constructor(
        "AbortController",
        0,
        ac_proto,
        Rc::new(move |rt, _, _| {
            let ctrl = rt.create_object_with_proto(Some(ac_proto));
            let sig = rt.create_object();
            let _ = rt.set_property(sig, "aborted", JsValue::Boolean(false));
            let _ = rt.set_property(sig, "reason", JsValue::Undefined);
            let noop = rt.create_native_function("noop", 0, Rc::new(|_, _, _| Ok(JsValue::Undefined)));
            let _ = rt.set_property(sig, "addEventListener", JsValue::Object(noop));
            let _ = rt.set_property(sig, "removeEventListener", JsValue::Object(noop));
            let abort_fn = rt.create_native_function(
                "abort",
                1,
                Rc::new(move |rt, _, args| {
                    let _ = rt.set_property(sig, "aborted", JsValue::Boolean(true));
                    let reason = args.first().cloned().unwrap_or(JsValue::String("AbortError".into()));
                    let _ = rt.set_property(sig, "reason", reason);
                    Ok(JsValue::Undefined)
                }),
            );
            let _ = rt.set_property(ctrl, "signal", JsValue::Object(sig));
            let _ = rt.set_property(ctrl, "abort", JsValue::Object(abort_fn));
            Ok(JsValue::Object(ctrl))
        }),
    );
    let _ = rt.set_property(global, "AbortController", JsValue::Object(ac_ctor));
}

fn define_observers_and_misc(rt: &mut JsRuntime, dom: SharedDom, global: ObjectId) {
    let d = dom.clone();
    let gcs = rt.create_native_function(
        "getComputedStyle",
        1,
        Rc::new(move |rt, _, args| {
            let nid = args.first().and_then(|v| node_id_of(rt, v));
            let style = {
                let mut st = d.borrow_mut();
                st.ensure_layout();
                nid.and_then(|id| st.styles.get(id).cloned()).unwrap_or_default()
            };
            let target = rt.create_object();
            let s1 = style.clone();
            let gpv = rt.create_native_function(
                "getPropertyValue",
                1,
                Rc::new(move |rt, _, args| {
                    let prop = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
                    let kebab = crate::style_binding::camel_to_kebab(&prop);
                    Ok(JsValue::String(get_computed_property(&s1, &kebab)))
                }),
            );
            let _ = rt.set_property(target, "getPropertyValue", JsValue::Object(gpv));
            let handler = rt.create_object();
            let get_t = rt.create_native_function(
                "get",
                3,
                Rc::new(move |rt, _, args| {
                    let Some(t_oid) = args.first().and_then(|v| v.as_object()) else {
                        return Ok(JsValue::Undefined);
                    };
                    let prop_v = args.get(1).cloned().unwrap_or(JsValue::Undefined);
                    if matches!(prop_v, JsValue::Symbol(_)) {
                        let key = rt.to_property_key(&prop_v)?;
                        return rt.get_property_key(t_oid, &key);
                    }
                    let prop = rt.to_string(&prop_v)?;
                    if prop == "getPropertyValue" {
                        return rt.get_property(t_oid, &prop);
                    }
                    let kebab = crate::style_binding::camel_to_kebab(&prop);
                    Ok(JsValue::String(get_computed_property(&style, &kebab)))
                }),
            );
            let _ = rt.set_property(handler, "get", JsValue::Object(get_t));
            let proxy_ctor = rt.get_property(rt.global_object(), "Proxy").unwrap_or(JsValue::Undefined);
            if let Ok(p) = rt.construct(proxy_ctor, &[JsValue::Object(target), JsValue::Object(handler)]) {
                Ok(p)
            } else {
                Ok(JsValue::Object(target))
            }
        }),
    );
    let _ = rt.set_property(global, "getComputedStyle", JsValue::Object(gcs));

    let d = dom.clone();
    let match_media = rt.create_native_function(
        "matchMedia",
        1,
        Rc::new(move |rt, _, args| {
            let query = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mql = parse_media_query_list(&query);
            let matches = {
                let st = d.borrow();
                mql.matches(&Viewport {
                    width: st.viewport_w,
                    height: st.viewport_h,
                    dark_mode: st.dark_mode,
                })
            };
            let obj = rt.create_object();
            let _ = rt.set_property(obj, "matches", JsValue::Boolean(matches));
            let _ = rt.set_property(obj, "media", JsValue::String(query));
            let noop = rt.create_native_function("noop", 0, Rc::new(|_, _, _| Ok(JsValue::Undefined)));
            let _ = rt.set_property(obj, "addListener", JsValue::Object(noop));
            let _ = rt.set_property(obj, "removeListener", JsValue::Object(noop));
            let _ = rt.set_property(obj, "addEventListener", JsValue::Object(noop));
            let _ = rt.set_property(obj, "removeEventListener", JsValue::Object(noop));
            Ok(JsValue::Object(obj))
        }),
    );
    let _ = rt.set_property(global, "matchMedia", JsValue::Object(match_media));

    let mo_proto = rt.create_object();
    let d = dom.clone();
    let mo_ctor = rt.create_native_constructor(
        "MutationObserver",
        1,
        mo_proto,
        Rc::new(move |rt, _, args| {
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            let idx = {
                let mut st = d.borrow_mut();
                let i = st.mutation_observers.len();
                st.mutation_observers.push(MutationObserverEntry {
                    callback: cb,
                    targets: Vec::new(),
                });
                i
            };
            let obj = rt.create_object_with_proto(Some(mo_proto));
            let d1 = d.clone();
            let obs_fn = rt.create_native_function(
                "observe",
                2,
                Rc::new(move |rt, _, args| {
                    let Some(nid) = args.first().and_then(|v| node_id_of(rt, v)) else {
                        return Ok(JsValue::Undefined);
                    };
                    let (mut cl, mut at, mut cd, mut sub) = (false, false, false, false);
                    if let Some(opts) = args.get(1).and_then(|v| v.as_object()) {
                        cl = rt.get_property(opts, "childList").map_or(false, |v| rt.to_boolean(&v));
                        at = rt.get_property(opts, "attributes").map_or(false, |v| rt.to_boolean(&v));
                        cd = rt
                            .get_property(opts, "characterData")
                            .map_or(false, |v| rt.to_boolean(&v));
                        sub = rt.get_property(opts, "subtree").map_or(false, |v| rt.to_boolean(&v));
                    }
                    if let Some(entry) = d1.borrow_mut().mutation_observers.get_mut(idx) {
                        entry.targets.push(MutationTarget {
                            node: nid,
                            child_list: cl,
                            attributes: at,
                            character_data: cd,
                            subtree: sub,
                        });
                    }
                    Ok(JsValue::Undefined)
                }),
            );
            let d2 = d.clone();
            let disc_fn = rt.create_native_function(
                "disconnect",
                0,
                Rc::new(move |_, _, _| {
                    if let Some(entry) = d2.borrow_mut().mutation_observers.get_mut(idx) {
                        entry.targets.clear();
                    }
                    Ok(JsValue::Undefined)
                }),
            );
            let take_fn = rt.create_native_function(
                "takeRecords",
                0,
                Rc::new(|rt, _, _| Ok(JsValue::Object(rt.create_array(Vec::new())))),
            );
            let _ = rt.set_property(obj, "observe", JsValue::Object(obs_fn));
            let _ = rt.set_property(obj, "disconnect", JsValue::Object(disc_fn));
            let _ = rt.set_property(obj, "takeRecords", JsValue::Object(take_fn));
            Ok(JsValue::Object(obj))
        }),
    );
    let _ = rt.set_property(global, "MutationObserver", JsValue::Object(mo_ctor));

    for obs_name in ["ResizeObserver", "IntersectionObserver"] {
        let p = rt.create_object();
        let noop = rt.create_native_function("noop", 0, Rc::new(|_, _, _| Ok(JsValue::Undefined)));
        let _ = rt.set_property(p, "observe", JsValue::Object(noop));
        let _ = rt.set_property(p, "unobserve", JsValue::Object(noop));
        let _ = rt.set_property(p, "disconnect", JsValue::Object(noop));
        let ctor = rt.create_native_constructor(
            obs_name,
            1,
            p,
            Rc::new(move |rt, _, _| Ok(JsValue::Object(rt.create_object_with_proto(Some(p))))),
        );
        let _ = rt.set_property(global, obs_name, JsValue::Object(ctor));
    }

    let dp_proto = rt.create_object();
    let d = dom.clone();
    let parse_str = rt.create_native_function(
        "parseFromString",
        2,
        Rc::new(move |rt, _, args| {
            let html = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let parsed = parse_html(&html);
            let frag_id = {
                let mut st = d.borrow_mut();
                let root = st.doc.create_fragment();
                if let Some(body) = parsed.body() {
                    let inner = parsed.inner_html(body);
                    let nodes = peedle_html::parse_fragment_into(
                        &mut st.doc,
                        "body",
                        peedle_html::Namespace::Html,
                        &inner,
                    );
                    for n in nodes {
                        st.doc.append_child(root, n);
                    }
                }
                root
            };
            let wrapper = rt.create_object();
            let body_obj = wrap_node(rt, &d, frag_id);
            let _ = rt.set_property(wrapper, "body", JsValue::Object(body_obj));
            let _ = rt.set_property(wrapper, "documentElement", JsValue::Object(body_obj));
            Ok(JsValue::Object(wrapper))
        }),
    );
    let _ = rt.set_property(dp_proto, "parseFromString", JsValue::Object(parse_str));
    let dp_ctor = rt.create_native_constructor(
        "DOMParser",
        0,
        dp_proto,
        Rc::new(move |rt, _, _| Ok(JsValue::Object(rt.create_object_with_proto(Some(dp_proto))))),
    );
    let _ = rt.set_property(global, "DOMParser", JsValue::Object(dp_ctor));

    let xs_proto = rt.create_object();
    let d = dom.clone();
    let ser_str = rt.create_native_function(
        "serializeToString",
        1,
        Rc::new(move |rt, _, args| {
            let Some(nid) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Ok(JsValue::String(String::new()));
            };
            Ok(JsValue::String(d.borrow().doc.outer_html(nid)))
        }),
    );
    let _ = rt.set_property(xs_proto, "serializeToString", JsValue::Object(ser_str));
    let xs_ctor = rt.create_native_constructor(
        "XMLSerializer",
        0,
        xs_proto,
        Rc::new(move |rt, _, _| Ok(JsValue::Object(rt.create_object_with_proto(Some(xs_proto))))),
    );
    let _ = rt.set_property(global, "XMLSerializer", JsValue::Object(xs_ctor));

    let ce = rt.create_object();
    let d = dom.clone();
    let ce_def = rt.create_native_function(
        "define",
        2,
        Rc::new(move |rt, _, args| {
            let name = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            if let Some(ctor) = args.get(1).and_then(|v| v.as_object()) {
                d.borrow_mut().custom_elements.insert(name, ctor);
            }
            Ok(JsValue::Undefined)
        }),
    );
    let d = dom.clone();
    let ce_get = rt.create_native_function(
        "get",
        1,
        Rc::new(move |rt, _, args| {
            let name = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            Ok(d
                .borrow()
                .custom_elements
                .get(&name)
                .map(|&id| JsValue::Object(id))
                .unwrap_or(JsValue::Undefined))
        }),
    );
    let _ = rt.set_property(ce, "define", JsValue::Object(ce_def));
    let _ = rt.set_property(ce, "get", JsValue::Object(ce_get));
    let _ = rt.set_property(global, "customElements", JsValue::Object(ce));

    let nf = rt.create_object();
    let _ = rt.set_property(nf, "SHOW_ALL", JsValue::Number(0xFFFF_FFFFu32 as f64));
    let _ = rt.set_property(nf, "SHOW_ELEMENT", JsValue::Number(1.0));
    let _ = rt.set_property(nf, "SHOW_TEXT", JsValue::Number(4.0));
    let _ = rt.set_property(nf, "SHOW_COMMENT", JsValue::Number(128.0));
    let _ = rt.set_property(nf, "FILTER_ACCEPT", JsValue::Number(1.0));
    let _ = rt.set_property(nf, "FILTER_REJECT", JsValue::Number(2.0));
    let _ = rt.set_property(nf, "FILTER_SKIP", JsValue::Number(3.0));
    let _ = rt.set_property(global, "NodeFilter", JsValue::Object(nf));
}

fn define_crypto(rt: &mut JsRuntime, global: ObjectId) {
    let crypto_obj = rt.create_object();

    let rand_uuid = rt.create_native_function(
        "randomUUID",
        0,
        Rc::new(|_, _, _| {
            let mut b = [0u8; 16];
            let _ = getrandom::fill(&mut b);
            b[6] = (b[6] & 0x0F) | 0x40;
            b[8] = (b[8] & 0x3F) | 0x80;
            let uuid = format!(
                "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
            );
            Ok(JsValue::String(uuid))
        }),
    );
    let _ = rt.set_property(crypto_obj, "randomUUID", JsValue::Object(rand_uuid));

    let get_rand = rt.create_native_function(
        "getRandomValues",
        1,
        Rc::new(|rt, _, args| {
            let arg = args.first().cloned().unwrap_or(JsValue::Undefined);
            if let Some(oid) = arg.as_object() {
                if let peedle_js::ObjectKind::TypedArray(ref ta) = rt.heap.get_obj(oid).kind {
                    let buf_id = ta.buffer;
                    let off = ta.byte_offset;
                    let len = ta.length * ta.kind.element_size();
                    if let peedle_js::ObjectKind::ArrayBuffer(ref mut ab) =
                        rt.heap.get_obj_mut(buf_id).kind
                    {
                        if off + len <= ab.bytes.len() {
                            let _ = getrandom::fill(&mut ab.bytes[off..off + len]);
                        }
                    }
                }
            }
            Ok(arg)
        }),
    );
    let _ = rt.set_property(crypto_obj, "getRandomValues", JsValue::Object(get_rand));

    let subtle = rt.create_object();
    let digest_fn = rt.create_native_function(
        "digest",
        2,
        Rc::new(|rt, _, args| {
            let p = rt.create_promise();
            let alg = match args.first() {
                Some(JsValue::String(s)) => s.to_ascii_uppercase(),
                Some(JsValue::Object(o)) => rt
                    .get_property(*o, "name")
                    .and_then(|v| rt.to_string(&v))
                    .unwrap_or_default()
                    .to_ascii_uppercase(),
                _ => String::new(),
            };
            let mut input_bytes = Vec::new();
            if let Some(oid) = args.get(1).and_then(|v| v.as_object()) {
                match &rt.heap.get_obj(oid).kind {
                    peedle_js::ObjectKind::ArrayBuffer(ab) => input_bytes = ab.bytes.clone(),
                    peedle_js::ObjectKind::TypedArray(ta) => {
                        let b = ta.buffer;
                        let off = ta.byte_offset;
                        let len = ta.length * ta.kind.element_size();
                        if let peedle_js::ObjectKind::ArrayBuffer(ab) = &rt.heap.get_obj(b).kind {
                            if off + len <= ab.bytes.len() {
                                input_bytes = ab.bytes[off..off + len].to_vec();
                            }
                        }
                    }
                    _ => {}
                }
            }
            let out: Vec<u8> = match alg.as_str() {
                "SHA-1" => sha1::Sha1::digest(&input_bytes).to_vec(),
                "SHA-256" => Sha256::digest(&input_bytes).to_vec(),
                "SHA-384" => Sha384::digest(&input_bytes).to_vec(),
                "SHA-512" => Sha512::digest(&input_bytes).to_vec(),
                _ => {
                    let err = JsValue::Object(rt.create_error_object(
                        peedle_js::ErrorKind::TypeError,
                        "Unsupported digest algorithm",
                    ));
                    rt.reject_promise(p, err);
                    return Ok(JsValue::Object(p));
                }
            };
            let ab_proto = rt.intrinsics.array_buffer_proto;
            let ab_obj = rt.heap.alloc_object(peedle_js::heap::JsObject::new(
                Some(ab_proto),
                peedle_js::ObjectKind::ArrayBuffer(peedle_js::heap::ArrayBufferData {
                    bytes: out,
                    detached: false,
                }),
            ));
            rt.resolve_promise(p, JsValue::Object(ab_obj));
            Ok(JsValue::Object(p))
        }),
    );
    let _ = rt.set_property(subtle, "digest", JsValue::Object(digest_fn));
    let _ = rt.set_property(crypto_obj, "subtle", JsValue::Object(subtle));
    let _ = rt.set_property(global, "crypto", JsValue::Object(crypto_obj));
}
