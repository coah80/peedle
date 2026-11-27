use std::rc::Rc;

use peedle_css::{query_selector, query_selector_all};
use peedle_html::{parse_fragment_into, Namespace, NodeData, QuirksMode};
use peedle_js::{JsError, JsRuntime, JsValue, ObjectId};
use peedle_url::Url;

use crate::events::{build_event_object, define_getter_setter};
use crate::nodes::{node_id_of, wrap_node, wrap_node_list, wrap_node_opt};
use crate::state::{RangeData, SharedDom};

pub fn define_document_proto(
    rt: &mut JsRuntime,
    dom: SharedDom,
    doc_proto: ObjectId,
    text_proto: ObjectId,
    comment_proto: ObjectId,
    fragment_proto: ObjectId,
    range_proto: ObjectId,
) {
    for proto in [text_proto, comment_proto] {
        let dg = dom.clone();
        let ds = dom.clone();
        let get_data = rt.create_native_function(
            "get data",
            0,
            Rc::new(move |rt, this, _| {
                let Some(nid) = node_id_of(rt, &this) else {
                    return Ok(JsValue::String(String::new()));
                };
                let s = match &dg.borrow().doc.node(nid).data {
                    NodeData::Text(s) | NodeData::Comment(s) => s.clone(),
                    _ => String::new(),
                };
                Ok(JsValue::String(s))
            }),
        );
        let set_data = rt.create_native_function(
            "set data",
            1,
            Rc::new(move |rt, this, args| {
                let Some(nid) = node_id_of(rt, &this) else {
                    return Ok(JsValue::Undefined);
                };
                let s = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
                let mut st = ds.borrow_mut();
                st.doc.set_character_data(nid, s);
                st.mark_dirty();
                Ok(JsValue::Undefined)
            }),
        );
        define_getter_setter(rt, proto, "data", Some(get_data), Some(set_data));

        let d = dom.clone();
        let get_len = rt.create_native_function(
            "get length",
            0,
            Rc::new(move |rt, this, _| {
                let Some(nid) = node_id_of(rt, &this) else {
                    return Ok(JsValue::Number(0.0));
                };
                let len = match &d.borrow().doc.node(nid).data {
                    NodeData::Text(s) | NodeData::Comment(s) => s.encode_utf16().count(),
                    _ => 0,
                };
                Ok(JsValue::Number(len as f64))
            }),
        );
        define_getter_setter(rt, proto, "length", Some(get_len), None);

        let d = dom.clone();
        let rem = rt.create_native_function(
            "remove",
            0,
            Rc::new(move |rt, this, _| {
                if let Some(nid) = node_id_of(rt, &this) {
                    let mut st = d.borrow_mut();
                    st.doc.detach(nid);
                    st.mark_dirty();
                }
                Ok(JsValue::Undefined)
            }),
        );
        let _ = rt.set_property(proto, "remove", JsValue::Object(rem));
    }

    let d = dom.clone();
    let frag_qs = rt.create_native_function(
        "querySelector",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let sel = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let hit = query_selector(&d.borrow().doc, nid, &sel);
            Ok(wrap_node_opt(rt, &d, hit))
        }),
    );
    let _ = rt.set_property(fragment_proto, "querySelector", JsValue::Object(frag_qs));

    let d = dom.clone();
    let frag_qsa = rt.create_native_function(
        "querySelectorAll",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Object(rt.create_array(Vec::new())));
            };
            let sel = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let hits = query_selector_all(&d.borrow().doc, nid, &sel);
            Ok(JsValue::Object(wrap_node_list(rt, &d, hits)))
        }),
    );
    let _ = rt.set_property(fragment_proto, "querySelectorAll", JsValue::Object(frag_qsa));

    let d = dom.clone();
    let get_de = rt.create_native_function(
        "get documentElement",
        0,
        Rc::new(move |rt, this, _| {
            let id = {
                let st = d.borrow();
                let root = node_id_of(rt, &this).unwrap_or(st.doc.root);
                st.doc.document_element_of(root)
            };
            Ok(wrap_node_opt(rt, &d, id))
        }),
    );
    define_getter_setter(rt, doc_proto, "documentElement", Some(get_de), None);

    let d = dom.clone();
    let get_head = rt.create_native_function(
        "get head",
        0,
        Rc::new(move |rt, this, _| {
            let id = {
                let st = d.borrow();
                let root = node_id_of(rt, &this).unwrap_or(st.doc.root);
                st.doc.head_of(root)
            };
            Ok(wrap_node_opt(rt, &d, id))
        }),
    );
    define_getter_setter(rt, doc_proto, "head", Some(get_head), None);

    let d = dom.clone();
    let get_body = rt.create_native_function(
        "get body",
        0,
        Rc::new(move |rt, this, _| {
            let id = {
                let st = d.borrow();
                let root = node_id_of(rt, &this).unwrap_or(st.doc.root);
                st.doc.body_of(root)
            };
            Ok(wrap_node_opt(rt, &d, id))
        }),
    );
    define_getter_setter(rt, doc_proto, "body", Some(get_body), None);

    let dg = dom.clone();
    let ds = dom.clone();
    let get_title = rt.create_native_function(
        "get title",
        0,
        Rc::new(move |_, _, _| Ok(JsValue::String(dg.borrow().doc.title()))),
    );
    let set_title = rt.create_native_function(
        "set title",
        1,
        Rc::new(move |rt, _, args| {
            let s = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            ds.borrow_mut().doc.title_override = Some(s);
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, doc_proto, "title", Some(get_title), Some(set_title));

    let d = dom.clone();
    let get_rs = rt.create_native_function(
        "get readyState",
        0,
        Rc::new(move |_, _, _| Ok(JsValue::String(d.borrow().ready_state.clone()))),
    );
    define_getter_setter(rt, doc_proto, "readyState", Some(get_rs), None);

    let d = dom.clone();
    let get_cm = rt.create_native_function(
        "get compatMode",
        0,
        Rc::new(move |_, _, _| {
            let s = if d.borrow().doc.quirks_mode() == QuirksMode::Quirks {
                "BackCompat"
            } else {
                "CSS1Compat"
            };
            Ok(JsValue::String(s.to_string()))
        }),
    );
    define_getter_setter(rt, doc_proto, "compatMode", Some(get_cm), None);

    let d = dom.clone();
    let get_url = rt.create_native_function(
        "get URL",
        0,
        Rc::new(move |_, _, _| Ok(JsValue::String(d.borrow().url.clone()))),
    );
    define_getter_setter(rt, doc_proto, "URL", Some(get_url), None);
    define_getter_setter(rt, doc_proto, "documentURI", Some(get_url), None);

    let d = dom.clone();
    let get_active = rt.create_native_function(
        "get activeElement",
        0,
        Rc::new(move |rt, _, _| {
            let id = d.borrow().focused_node.or_else(|| d.borrow().doc.body());
            Ok(wrap_node_opt(rt, &d, id))
        }),
    );
    define_getter_setter(rt, doc_proto, "activeElement", Some(get_active), None);

    let get_dv = rt.create_native_function(
        "get defaultView",
        0,
        Rc::new(|rt, _, _| Ok(JsValue::Object(rt.global_object()))),
    );
    define_getter_setter(rt, doc_proto, "defaultView", Some(get_dv), None);

    let dg = dom.clone();
    let ds = dom.clone();
    let get_cookie = rt.create_native_function(
        "get cookie",
        0,
        Rc::new(move |_, _, _| {
            let st = dg.borrow();
            let s = Url::parse(&st.url)
                .map(|u| st.client.cookies.lock().unwrap().cookie_header_for(&u, false))
                .unwrap_or_default();
            Ok(JsValue::String(s))
        }),
    );
    let set_cookie = rt.create_native_function(
        "set cookie",
        1,
        Rc::new(move |rt, _, args| {
            let s = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let st = ds.borrow();
            if let Ok(u) = Url::parse(&st.url) {
                st.client.cookies.lock().unwrap().set_from_header(&u, &s);
            }
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, doc_proto, "cookie", Some(get_cookie), Some(set_cookie));

    let _ = rt.set_property(doc_proto, "characterSet", JsValue::String("UTF-8".into()));
    let _ = rt.set_property(doc_proto, "charset", JsValue::String("UTF-8".into()));
    let _ = rt.set_property(doc_proto, "contentType", JsValue::String("text/html".into()));

    let d = dom.clone();
    let create_el = rt.create_native_function(
        "createElement",
        1,
        Rc::new(move |rt, _, args| {
            let tag = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let nid = d.borrow_mut().doc.create_element(tag);
            Ok(JsValue::Object(wrap_node(rt, &d, nid)))
        }),
    );
    let _ = rt.set_property(doc_proto, "createElement", JsValue::Object(create_el));

    let d = dom.clone();
    let create_el_ns = rt.create_native_function(
        "createElementNS",
        2,
        Rc::new(move |rt, _, args| {
            let ns_uri = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let tag = args.get(1).map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let ns = Namespace::from_uri(&ns_uri);
            let final_tag = if ns == Namespace::Html {
                tag.to_ascii_lowercase()
            } else {
                tag
            };
            let nid = d.borrow_mut().doc.create_element_ns(final_tag, ns, Vec::new());
            Ok(JsValue::Object(wrap_node(rt, &d, nid)))
        }),
    );
    let _ = rt.set_property(doc_proto, "createElementNS", JsValue::Object(create_el_ns));

    let d = dom.clone();
    let create_text = rt.create_native_function(
        "createTextNode",
        1,
        Rc::new(move |rt, _, args| {
            let text = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let nid = d.borrow_mut().doc.create_text(text);
            Ok(JsValue::Object(wrap_node(rt, &d, nid)))
        }),
    );
    let _ = rt.set_property(doc_proto, "createTextNode", JsValue::Object(create_text));

    let d = dom.clone();
    let create_comment = rt.create_native_function(
        "createComment",
        1,
        Rc::new(move |rt, _, args| {
            let text = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let nid = d.borrow_mut().doc.create_comment(text);
            Ok(JsValue::Object(wrap_node(rt, &d, nid)))
        }),
    );
    let _ = rt.set_property(doc_proto, "createComment", JsValue::Object(create_comment));

    let d = dom.clone();
    let create_frag = rt.create_native_function(
        "createDocumentFragment",
        0,
        Rc::new(move |rt, _, _| {
            let nid = d.borrow_mut().doc.create_fragment();
            Ok(JsValue::Object(wrap_node(rt, &d, nid)))
        }),
    );
    let _ = rt.set_property(doc_proto, "createDocumentFragment", JsValue::Object(create_frag));

    let d = dom.clone();
    let create_ev = rt.create_native_function(
        "createEvent",
        1,
        Rc::new(move |rt, _, _| {
            let ev_proto = d.borrow().protos.unwrap().event;
            let obj = build_event_object(rt, ev_proto, String::new(), false, false, false);
            Ok(JsValue::Object(obj))
        }),
    );
    let _ = rt.set_property(doc_proto, "createEvent", JsValue::Object(create_ev));

    let d = dom.clone();
    let create_range = rt.create_native_function(
        "createRange",
        0,
        Rc::new(move |rt, _, _| {
            let root = d.borrow().doc.root;
            let obj = rt.create_object_with_proto(Some(range_proto));
            rt.set_host_data(
                obj,
                RangeData {
                    start_container: root,
                    start_offset: 0,
                    end_container: root,
                    end_offset: 0,
                },
            );
            Ok(JsValue::Object(obj))
        }),
    );
    let _ = rt.set_property(doc_proto, "createRange", JsValue::Object(create_range));

    let d = dom.clone();
    let get_by_id = rt.create_native_function(
        "getElementById",
        1,
        Rc::new(move |rt, this, args| {
            let id = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let hit = {
                let st = d.borrow();
                let root = node_id_of(rt, &this).unwrap_or(st.doc.root);
                st.doc.get_element_by_id_in(root, &id)
            };
            Ok(wrap_node_opt(rt, &d, hit))
        }),
    );
    let _ = rt.set_property(doc_proto, "getElementById", JsValue::Object(get_by_id));

    let d = dom.clone();
    let gebtn = rt.create_native_function(
        "getElementsByTagName",
        1,
        Rc::new(move |rt, this, args| {
            let tag = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let list = {
                let st = d.borrow();
                let root = node_id_of(rt, &this).unwrap_or(st.doc.root);
                st.doc.get_elements_by_tag_name(root, &tag)
            };
            Ok(JsValue::Object(wrap_node_list(rt, &d, list)))
        }),
    );
    let _ = rt.set_property(doc_proto, "getElementsByTagName", JsValue::Object(gebtn));

    let d = dom.clone();
    let gebcn = rt.create_native_function(
        "getElementsByClassName",
        1,
        Rc::new(move |rt, this, args| {
            let cls = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let list = {
                let st = d.borrow();
                let root = node_id_of(rt, &this).unwrap_or(st.doc.root);
                st.doc.get_elements_by_class_name(root, &cls)
            };
            Ok(JsValue::Object(wrap_node_list(rt, &d, list)))
        }),
    );
    let _ = rt.set_property(doc_proto, "getElementsByClassName", JsValue::Object(gebcn));

    let d = dom.clone();
    let qs = rt.create_native_function(
        "querySelector",
        1,
        Rc::new(move |rt, this, args| {
            let sel = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let hit = {
                let st = d.borrow();
                let root = node_id_of(rt, &this).unwrap_or(st.doc.root);
                query_selector(&st.doc, root, &sel)
            };
            Ok(wrap_node_opt(rt, &d, hit))
        }),
    );
    let _ = rt.set_property(doc_proto, "querySelector", JsValue::Object(qs));

    let d = dom.clone();
    let qsa = rt.create_native_function(
        "querySelectorAll",
        1,
        Rc::new(move |rt, this, args| {
            let sel = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let hits = {
                let st = d.borrow();
                let root = node_id_of(rt, &this).unwrap_or(st.doc.root);
                query_selector_all(&st.doc, root, &sel)
            };
            Ok(JsValue::Object(wrap_node_list(rt, &d, hits)))
        }),
    );
    let _ = rt.set_property(doc_proto, "querySelectorAll", JsValue::Object(qsa));

    let d = dom.clone();
    let import_node = rt.create_native_function(
        "importNode",
        2,
        Rc::new(move |rt, _, args| {
            let Some(nid) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Err(JsError::type_err("importNode requires Node"));
            };
            let deep = args.get(1).map_or(false, |v| rt.to_boolean(v));
            let cloned = d.borrow_mut().doc.clone_node(nid, deep);
            Ok(JsValue::Object(wrap_node(rt, &d, cloned)))
        }),
    );
    let _ = rt.set_property(doc_proto, "importNode", JsValue::Object(import_node));

    let d = dom.clone();
    let doc_write = rt.create_native_function(
        "write",
        1,
        Rc::new(move |rt, _, args| {
            let mut html = String::new();
            for a in args {
                html.push_str(&rt.to_string(a)?);
            }
            let mut st = d.borrow_mut();
            if let Some(body) = st.doc.body() {
                let nodes = parse_fragment_into(&mut st.doc, "body", Namespace::Html, &html);
                for n in nodes {
                    st.doc.append_child(body, n);
                }
                st.mark_dirty();
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(doc_proto, "write", JsValue::Object(doc_write));
    let _ = rt.set_property(doc_proto, "writeln", JsValue::Object(doc_write));

    let has_focus = rt.create_native_function("hasFocus", 0, Rc::new(|_, _, _| Ok(JsValue::Boolean(true))));
    let _ = rt.set_property(doc_proto, "hasFocus", JsValue::Object(has_focus));

    let d = dom.clone();
    let get_sheets = rt.create_native_function(
        "get styleSheets",
        0,
        Rc::new(move |rt, _, _| {
            let count = d.borrow().stylesheets.len();
            let mut arr = Vec::with_capacity(count);
            for _ in 0..count {
                let so = rt.create_object();
                let rules = rt.create_array(Vec::new());
                let _ = rt.set_property(so, "cssRules", JsValue::Object(rules));
                arr.push(JsValue::Object(so));
            }
            Ok(JsValue::Object(rt.create_array(arr)))
        }),
    );
    define_getter_setter(rt, doc_proto, "styleSheets", Some(get_sheets), None);

    let d = dom.clone();
    let impl_obj = rt.create_object();
    let create_html_doc = rt.create_native_function(
        "createHTMLDocument",
        1,
        Rc::new(move |rt, _, args| {
            let title_text = args
                .first()
                .filter(|v| !v.is_undefined())
                .map(|v| rt.to_string(v))
                .transpose()?;
            let sub = {
                let mut st = d.borrow_mut();
                let sub = st.doc.create_sub_document();
                if let Some(t) = title_text {
                    if let Some(head) = st.doc.head_of(sub) {
                        let title_el = st.doc.create_element("title");
                        let txt = st.doc.create_text(t);
                        st.doc.append_child(title_el, txt);
                        st.doc.append_child(head, title_el);
                    }
                }
                sub
            };
            Ok(JsValue::Object(wrap_node(rt, &d, sub)))
        }),
    );
    let _ = rt.set_property(impl_obj, "createHTMLDocument", JsValue::Object(create_html_doc));
    let _ = rt.set_property(doc_proto, "implementation", JsValue::Object(impl_obj));

    define_range_proto(rt, dom, range_proto);
}

fn define_range_proto(rt: &mut JsRuntime, dom: SharedDom, range_proto: ObjectId) {
    let set_start = rt.create_native_function(
        "setStart",
        2,
        Rc::new(|rt, this, args| {
            let Some(oid) = this.as_object() else {
                return Ok(JsValue::Undefined);
            };
            let Some(nid) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Ok(JsValue::Undefined);
            };
            let off = args.get(1).and_then(|v| rt.to_number(v).ok()).unwrap_or(0.0) as usize;
            if let Some(r) = rt.get_host_data_mut::<RangeData>(oid) {
                r.start_container = nid;
                r.start_offset = off;
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(range_proto, "setStart", JsValue::Object(set_start));

    let set_end = rt.create_native_function(
        "setEnd",
        2,
        Rc::new(|rt, this, args| {
            let Some(oid) = this.as_object() else {
                return Ok(JsValue::Undefined);
            };
            let Some(nid) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Ok(JsValue::Undefined);
            };
            let off = args.get(1).and_then(|v| rt.to_number(v).ok()).unwrap_or(0.0) as usize;
            if let Some(r) = rt.get_host_data_mut::<RangeData>(oid) {
                r.end_container = nid;
                r.end_offset = off;
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(range_proto, "setEnd", JsValue::Object(set_end));

    let d = dom.clone();
    let create_ctx_frag = rt.create_native_function(
        "createContextualFragment",
        1,
        Rc::new(move |rt, _, args| {
            let html = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let frag = {
                let mut st = d.borrow_mut();
                let f = st.doc.create_fragment();
                let nodes = parse_fragment_into(&mut st.doc, "div", Namespace::Html, &html);
                for n in nodes {
                    st.doc.append_child(f, n);
                }
                f
            };
            Ok(JsValue::Object(wrap_node(rt, &d, frag)))
        }),
    );
    let _ = rt.set_property(range_proto, "createContextualFragment", JsValue::Object(create_ctx_frag));
}
