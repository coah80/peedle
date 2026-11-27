use std::cell::RefCell;
use std::rc::Rc;

use peedle_css::{parse_stylesheet, query_selector, query_selector_all};
use peedle_html::{parse_fragment_into, Namespace, NodeData, NodeId, QuirksMode};
use peedle_js::{JsError, JsRuntime, JsValue, ObjectId};
use peedle_url::Url;

use crate::events::{build_event_object, define_getter_setter};
use crate::nodes::{import_foreign_subtree, node_id_of, wrap_node, wrap_node_list, wrap_node_opt};
use crate::state::{LiveNodeIterator, NodeIteratorHandle, RangeData, RangeHandle, SharedDom};
use crate::traversal::{
    define_full_range_proto, is_valid_ncname, is_valid_xml_name, throw_dom_exception,
    TreeWalkerData,
};

pub fn define_document_proto(
    rt: &mut JsRuntime,
    dom: SharedDom,
    doc_proto: ObjectId,
    text_proto: ObjectId,
    comment_proto: ObjectId,
    fragment_proto: ObjectId,
    doctype_proto: ObjectId,
    range_proto: ObjectId,
) {
    let d = dom.clone();
    let get_dt_name = rt.create_native_function(
        "get name",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::String(String::new()));
            };
            let s = match &d.borrow().doc.node(nid).data {
                NodeData::Doctype { name, .. } => name.clone(),
                _ => String::new(),
            };
            Ok(JsValue::String(s))
        }),
    );
    define_getter_setter(rt, doctype_proto, "name", Some(get_dt_name), None);

    let d = dom.clone();
    let get_dt_pub = rt.create_native_function(
        "get publicId",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::String(String::new()));
            };
            let s = match &d.borrow().doc.node(nid).data {
                NodeData::Doctype { public_id, .. } => public_id.clone(),
                _ => String::new(),
            };
            Ok(JsValue::String(s))
        }),
    );
    define_getter_setter(rt, doctype_proto, "publicId", Some(get_dt_pub), None);

    let d = dom.clone();
    let get_dt_sys = rt.create_native_function(
        "get systemId",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::String(String::new()));
            };
            let s = match &d.borrow().doc.node(nid).data {
                NodeData::Doctype { system_id, .. } => system_id.clone(),
                _ => String::new(),
            };
            Ok(JsValue::String(s))
        }),
    );
    define_getter_setter(rt, doctype_proto, "systemId", Some(get_dt_sys), None);

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
                st.invalidate_style_owner(nid);
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
                    st.notify_pre_remove(nid);
                    st.invalidate_style_owner(nid);
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
        Rc::new(move |rt, this, _| {
            let st = dg.borrow();
            let root = node_id_of(rt, &this).unwrap_or(st.doc.root);
            if let Some(t) = st.title_overrides.get(&root) {
                return Ok(JsValue::String(t.clone()));
            }
            if root == st.doc.root {
                return Ok(JsValue::String(st.doc.title()));
            }
            for id in st.doc.descendants(root) {
                if let Some(el) = st.doc.element(id) {
                    if el.is_html() && el.tag_is("title") {
                        let txt = st
                            .doc
                            .text_content(id)
                            .split_ascii_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ");
                        return Ok(JsValue::String(txt));
                    }
                }
            }
            Ok(JsValue::String(String::new()))
        }),
    );
    let set_title = rt.create_native_function(
        "set title",
        1,
        Rc::new(move |rt, this, args| {
            let s = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mut st = ds.borrow_mut();
            let root = node_id_of(rt, &this).unwrap_or(st.doc.root);
            if root == st.doc.root {
                st.doc.title_override = Some(s.clone());
            }
            st.title_overrides.insert(root, s);
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
        Rc::new(move |rt, this, args| {
            let tag = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            if !is_valid_xml_name(&tag) {
                return Err(throw_dom_exception(
                    rt,
                    &d,
                    5,
                    "InvalidCharacterError",
                    "Invalid tag name",
                ));
            }
            let is_xml = node_id_of(rt, &this)
                .map_or(false, |doc_id| d.borrow().xml_documents.contains(&doc_id));
            let nid = if is_xml {
                let mut st = d.borrow_mut();
                let id = st.doc.create_element_ns(tag, Namespace::Html, Vec::new());
                st.xml_elements.insert(id);
                id
            } else {
                d.borrow_mut().doc.create_element(tag)
            };
            Ok(JsValue::Object(wrap_node(rt, &d, nid)))
        }),
    );
    let _ = rt.set_property(doc_proto, "createElement", JsValue::Object(create_el));

    let d = dom.clone();
    let create_el_ns = rt.create_native_function(
        "createElementNS",
        2,
        Rc::new(move |rt, _, args| {
            let ns_arg = args.first().cloned().unwrap_or(JsValue::Null);
            let ns_uri = if ns_arg.is_nullish() {
                String::new()
            } else {
                rt.to_string(&ns_arg)?
            };
            let tag = args.get(1).map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            if !is_valid_xml_name(&tag) {
                return Err(throw_dom_exception(
                    rt,
                    &d,
                    5,
                    "InvalidCharacterError",
                    "Invalid qualified name",
                ));
            }
            let prefix_opt = if let Some(colon) = tag.find(':') {
                let prefix = &tag[..colon];
                let local = &tag[colon + 1..];
                if !is_valid_ncname(prefix) || !is_valid_ncname(local) {
                    return Err(throw_dom_exception(
                        rt,
                        &d,
                        14,
                        "NamespaceError",
                        "Invalid qualified name",
                    ));
                }
                if ns_uri.is_empty() {
                    return Err(throw_dom_exception(
                        rt,
                        &d,
                        14,
                        "NamespaceError",
                        "Prefix requires non-null namespace",
                    ));
                }
                if prefix == "xml" && ns_uri != "http://www.w3.org/XML/1998/namespace" {
                    return Err(throw_dom_exception(
                        rt,
                        &d,
                        14,
                        "NamespaceError",
                        "xml prefix requires XML namespace",
                    ));
                }
                if prefix == "xmlns" && ns_uri != "http://www.w3.org/2000/xmlns/" {
                    return Err(throw_dom_exception(
                        rt,
                        &d,
                        14,
                        "NamespaceError",
                        "xmlns requires XMLNS namespace",
                    ));
                }
                Some(prefix)
            } else {
                if tag == "xmlns" && ns_uri != "http://www.w3.org/2000/xmlns/" {
                    return Err(throw_dom_exception(
                        rt,
                        &d,
                        14,
                        "NamespaceError",
                        "xmlns requires XMLNS namespace",
                    ));
                }
                None
            };
            if ns_uri == "http://www.w3.org/2000/xmlns/"
                && tag != "xmlns"
                && prefix_opt != Some("xmlns")
            {
                return Err(throw_dom_exception(
                    rt,
                    &d,
                    14,
                    "NamespaceError",
                    "XMLNS namespace requires xmlns prefix or qualified name",
                ));
            }
            let is_html_ns = ns_uri == "http://www.w3.org/1999/xhtml";
            let ns = if ns_uri.is_empty() {
                Namespace::Html
            } else {
                Namespace::from_uri(&ns_uri)
            };
            let mut st = d.borrow_mut();
            let nid = st.doc.create_element_ns(tag, ns, Vec::new());
            if !is_html_ns {
                st.xml_elements.insert(nid);
            }
            st.custom_namespaces.insert(
                nid,
                if ns_uri.is_empty() {
                    None
                } else {
                    Some(ns_uri)
                },
            );
            drop(st);
            Ok(JsValue::Object(wrap_node(rt, &d, nid)))
        }),
    );
    let _ = rt.set_property(doc_proto, "createElementNS", JsValue::Object(create_el_ns));

    let d = dom.clone();
    let create_pi = rt.create_native_function(
        "createProcessingInstruction",
        2,
        Rc::new(move |rt, _, args| {
            let target = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let data = args.get(1).map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            if !is_valid_xml_name(&target) || data.contains("?>") {
                return Err(throw_dom_exception(
                    rt,
                    &d,
                    5,
                    "InvalidCharacterError",
                    "Invalid processing instruction",
                ));
            }
            let nid = d.borrow_mut().doc.create_processing_instruction(target, data);
            Ok(JsValue::Object(wrap_node(rt, &d, nid)))
        }),
    );
    let _ = rt.set_property(
        doc_proto,
        "createProcessingInstruction",
        JsValue::Object(create_pi),
    );

    let d = dom.clone();
    let create_ni = rt.create_native_function(
        "createNodeIterator",
        1,
        Rc::new(move |rt, _, args| {
            let Some(root) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Err(JsError::type_err("createNodeIterator requires root Node"));
            };
            let what_to_show = args
                .get(1)
                .filter(|v| !v.is_undefined())
                .and_then(|v| rt.to_number(v).ok())
                .map(|n| (n as i64) as u32)
                .unwrap_or(0xFFFF_FFFF);
            let filter = args.get(2).cloned().unwrap_or(JsValue::Null);
            let proto = d.borrow().protos.unwrap().node_iterator_proto;
            let obj = rt.create_object_with_proto(Some(proto));
            let live = Rc::new(RefCell::new(LiveNodeIterator {
                root,
                what_to_show,
                filter,
                reference: root,
                pointer_before: true,
                detached: false,
            }));
            d.borrow_mut().live_iterators.push(live.clone());
            rt.set_host_data(obj, NodeIteratorHandle(live));
            Ok(JsValue::Object(obj))
        }),
    );
    let _ = rt.set_property(doc_proto, "createNodeIterator", JsValue::Object(create_ni));

    let d = dom.clone();
    let create_tw = rt.create_native_function(
        "createTreeWalker",
        1,
        Rc::new(move |rt, _, args| {
            let Some(root) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Err(JsError::type_err("createTreeWalker requires root Node"));
            };
            let what_to_show = args
                .get(1)
                .filter(|v| !v.is_undefined())
                .and_then(|v| rt.to_number(v).ok())
                .map(|n| (n as i64) as u32)
                .unwrap_or(0xFFFF_FFFF);
            let filter = args.get(2).cloned().unwrap_or(JsValue::Null);
            let proto = d.borrow().protos.unwrap().tree_walker_proto;
            let obj = rt.create_object_with_proto(Some(proto));
            rt.set_host_data(
                obj,
                TreeWalkerData {
                    root,
                    what_to_show,
                    filter,
                    current: root,
                },
            );
            Ok(JsValue::Object(obj))
        }),
    );
    let _ = rt.set_property(doc_proto, "createTreeWalker", JsValue::Object(create_tw));

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
        Rc::new(move |rt, this, _| {
            let root = node_id_of(rt, &this).unwrap_or_else(|| d.borrow().doc.root);
            let obj = rt.create_object_with_proto(Some(range_proto));
            let live = Rc::new(RefCell::new(RangeData {
                start_container: root,
                start_offset: 0,
                end_container: root,
                end_offset: 0,
            }));
            d.borrow_mut().live_ranges.push(live.clone());
            rt.set_host_data(obj, RangeHandle(live));
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
    let doc_open = rt.create_native_function(
        "open",
        0,
        Rc::new(move |rt, this, _| {
            let doc_id = node_id_of(rt, &this).unwrap_or_else(|| d.borrow().doc.root);
            let mut st = d.borrow_mut();
            let kids = st.doc.children(doc_id);
            for c in kids {
                st.notify_pre_remove(c);
                st.doc.detach(c);
            }
            st.open_doc_buffers.insert(doc_id, String::new());
            st.mark_dirty();
            Ok(this)
        }),
    );
    let _ = rt.set_property(doc_proto, "open", JsValue::Object(doc_open));

    let d = dom.clone();
    let doc_close = rt.create_native_function(
        "close",
        0,
        Rc::new(move |rt, this, _| {
            let doc_id = node_id_of(rt, &this).unwrap_or_else(|| d.borrow().doc.root);
            let buf_opt = d.borrow_mut().open_doc_buffers.remove(&doc_id);
            if let Some(html) = buf_opt {
                let parsed = peedle_html::parse_html(&html);
                let mut st = d.borrow_mut();
                let kids = st.doc.children(doc_id);
                for c in kids {
                    st.notify_pre_remove(c);
                    st.doc.detach(c);
                }
                for src_child in parsed.children(parsed.root) {
                    let imported = import_foreign_subtree(&mut st.doc, &parsed, src_child);
                    st.doc.append_child(doc_id, imported);
                }
                st.mark_dirty();
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(doc_proto, "close", JsValue::Object(doc_close));

    let d = dom.clone();
    let doc_write = rt.create_native_function(
        "write",
        1,
        Rc::new(move |rt, this, args| {
            let mut html = String::new();
            for a in args {
                html.push_str(&rt.to_string(a)?);
            }
            let doc_id = node_id_of(rt, &this).unwrap_or_else(|| d.borrow().doc.root);
            let mut st = d.borrow_mut();
            if let Some(buf) = st.open_doc_buffers.get_mut(&doc_id) {
                buf.push_str(&html);
                return Ok(JsValue::Undefined);
            }
            if doc_id == st.doc.root {
                if let Some(sid) = st.current_script {
                    if let Some(parent) = st.doc.node(sid).parent {
                        let ref_sib = st.doc.node(sid).next_sibling;
                        let nodes = parse_fragment_into(&mut st.doc, "body", Namespace::Html, &html);
                        for n in nodes {
                            st.doc.insert_before(parent, n, ref_sib);
                        }
                        st.mark_dirty();
                        return Ok(JsValue::Undefined);
                    }
                }
            }
            if let Some(body) = st.doc.body_of(doc_id) {
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

    let d = dom.clone();
    let doc_writeln = rt.create_native_function(
        "writeln",
        1,
        Rc::new(move |rt, this, args| {
            let mut html = String::new();
            for a in args {
                html.push_str(&rt.to_string(a)?);
            }
            html.push('\n');
            let doc_id = node_id_of(rt, &this).unwrap_or_else(|| d.borrow().doc.root);
            let mut st = d.borrow_mut();
            if let Some(buf) = st.open_doc_buffers.get_mut(&doc_id) {
                buf.push_str(&html);
                return Ok(JsValue::Undefined);
            }
            if doc_id == st.doc.root {
                if let Some(sid) = st.current_script {
                    if let Some(parent) = st.doc.node(sid).parent {
                        let ref_sib = st.doc.node(sid).next_sibling;
                        let nodes = parse_fragment_into(&mut st.doc, "body", Namespace::Html, &html);
                        for n in nodes {
                            st.doc.insert_before(parent, n, ref_sib);
                        }
                        st.mark_dirty();
                        return Ok(JsValue::Undefined);
                    }
                }
            }
            if let Some(body) = st.doc.body_of(doc_id) {
                let nodes = parse_fragment_into(&mut st.doc, "body", Namespace::Html, &html);
                for n in nodes {
                    st.doc.append_child(body, n);
                }
                st.mark_dirty();
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(doc_proto, "writeln", JsValue::Object(doc_writeln));

    let has_focus = rt.create_native_function("hasFocus", 0, Rc::new(|_, _, _| Ok(JsValue::Boolean(true))));
    let _ = rt.set_property(doc_proto, "hasFocus", JsValue::Object(has_focus));

    let d = dom.clone();
    let get_sheets = rt.create_native_function(
        "get styleSheets",
        0,
        Rc::new(move |rt, this, _| {
            let sheet_nodes: Vec<(NodeId, Option<String>)> = {
                let st = d.borrow();
                let root = node_id_of(rt, &this).unwrap_or(st.doc.root);
                let mut out = Vec::new();
                for nid in st.doc.descendants(root) {
                    if st.external_stylesheets.contains_key(&nid) {
                        let href = st
                            .doc
                            .element(nid)
                            .and_then(|e| e.get_attr("href"))
                            .map(|h| {
                                Url::parse(&st.url)
                                    .ok()
                                    .and_then(|b| b.join(h).ok())
                                    .map(|u| u.href())
                                    .unwrap_or_else(|| h.to_string())
                            });
                        out.push((nid, href));
                    } else if let Some(el) = st.doc.element(nid) {
                        if el.is_html() && el.tag_is("style") {
                            out.push((nid, None));
                        }
                    }
                }
                out
            };
            let mut arr = Vec::with_capacity(sheet_nodes.len());
            for (snid, href_opt) in sheet_nodes {
                let so = rt.create_object();
                let owner = wrap_node(rt, &d, snid);
                let _ = rt.set_property(so, "ownerNode", JsValue::Object(owner));
                let href_val = match href_opt {
                    Some(h) => JsValue::String(h),
                    None => JsValue::Null,
                };
                let _ = rt.set_property(so, "href", href_val);

                let rules_target = rt.create_object();
                let d_item = d.clone();
                let item_fn = rt.create_native_function(
                    "item",
                    1,
                    Rc::new(move |rt, _, args| {
                        let idx = args.first().and_then(|v| rt.to_number(v).ok()).unwrap_or(0.0) as usize;
                        let len = d_item
                            .borrow()
                            .stylesheet_for_node(snid)
                            .map_or(0, |s| s.rules.len());
                        if idx < len {
                            Ok(JsValue::Object(rt.create_object()))
                        } else {
                            Ok(JsValue::Null)
                        }
                    }),
                );
                let _ = rt.set_property(rules_target, "item", JsValue::Object(item_fn));
                let handler = rt.create_object();
                let d_get = d.clone();
                let get_trap = rt.create_native_function(
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
                        if prop == "length" {
                            let len = d_get
                                .borrow()
                                .stylesheet_for_node(snid)
                                .map_or(0, |s| s.rules.len());
                            return Ok(JsValue::Number(len as f64));
                        }
                        if prop == "item" {
                            return rt.get_property(t_oid, &prop);
                        }
                        if let Ok(idx) = prop.parse::<usize>() {
                            let len = d_get
                                .borrow()
                                .stylesheet_for_node(snid)
                                .map_or(0, |s| s.rules.len());
                            if idx < len {
                                return Ok(JsValue::Object(rt.create_object()));
                            }
                            return Ok(JsValue::Undefined);
                        }
                        rt.get_property(t_oid, &prop)
                    }),
                );
                let _ = rt.set_property(handler, "get", JsValue::Object(get_trap));
                let proxy_ctor = rt.get_property(rt.global_object(), "Proxy").unwrap_or(JsValue::Undefined);
                let rules_val = rt
                    .construct(proxy_ctor, &[JsValue::Object(rules_target), JsValue::Object(handler)])
                    .unwrap_or(JsValue::Object(rules_target));
                let _ = rt.set_property(so, "cssRules", rules_val.clone());
                let _ = rt.set_property(so, "rules", rules_val);

                let d_ins = d.clone();
                let ins_rule = rt.create_native_function(
                    "insertRule",
                    2,
                    Rc::new(move |rt, _, args| {
                        let rule_text = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
                        let idx = args.get(1).and_then(|v| rt.to_number(v).ok()).unwrap_or(0.0) as usize;
                        let mut st = d_ins.borrow_mut();
                        let mut sheet = st.stylesheet_for_node(snid).unwrap_or_default();
                        let parsed = parse_stylesheet(&rule_text);
                        let pos = idx.min(sheet.rules.len());
                        for (offset, rule) in parsed.rules.into_iter().enumerate() {
                            sheet.rules.insert(pos + offset, rule);
                        }
                        st.cssom_stylesheets.insert(snid, sheet);
                        st.mark_dirty();
                        Ok(JsValue::Number(pos as f64))
                    }),
                );
                let _ = rt.set_property(so, "insertRule", JsValue::Object(ins_rule));

                let d_del = d.clone();
                let del_rule = rt.create_native_function(
                    "deleteRule",
                    1,
                    Rc::new(move |rt, _, args| {
                        let idx = args.first().and_then(|v| rt.to_number(v).ok()).unwrap_or(0.0) as usize;
                        let mut st = d_del.borrow_mut();
                        let mut sheet = st.stylesheet_for_node(snid).unwrap_or_default();
                        if idx < sheet.rules.len() {
                            sheet.rules.remove(idx);
                            st.cssom_stylesheets.insert(snid, sheet);
                            st.mark_dirty();
                        }
                        Ok(JsValue::Undefined)
                    }),
                );
                let _ = rt.set_property(so, "deleteRule", JsValue::Object(del_rule));
                arr.push(JsValue::Object(so));
            }
            Ok(JsValue::Object(rt.create_array(arr)))
        }),
    );
    define_getter_setter(rt, doc_proto, "styleSheets", Some(get_sheets), None);

    let d = dom.clone();
    let get_doctype = rt.create_native_function(
        "get doctype",
        0,
        Rc::new(move |rt, this, _| {
            let dt = {
                let st = d.borrow();
                let root = node_id_of(rt, &this).unwrap_or(st.doc.root);
                st.doc.children(root).into_iter().find(|&c| {
                    matches!(st.doc.node(c).data, NodeData::Doctype { .. })
                })
            };
            Ok(wrap_node_opt(rt, &d, dt))
        }),
    );
    define_getter_setter(rt, doc_proto, "doctype", Some(get_doctype), None);

    for (prop, tag) in [
        ("forms", "form"),
        ("images", "img"),
        ("scripts", "script"),
    ] {
        let d = dom.clone();
        let gf = rt.create_native_function(
            prop,
            0,
            Rc::new(move |rt, this, _| {
                let (list, named): (Vec<NodeId>, Vec<(String, NodeId)>) = {
                    let st = d.borrow();
                    let root = node_id_of(rt, &this).unwrap_or(st.doc.root);
                    let ids = st.doc.get_elements_by_tag_name(root, tag);
                    let mut names = Vec::new();
                    for &id in &ids {
                        if let Some(el) = st.doc.element(id) {
                            if let Some(id_attr) = el.get_attr("id").filter(|s| !s.is_empty()) {
                                names.push((id_attr.to_string(), id));
                            }
                            if let Some(name_attr) = el.get_attr("name").filter(|s| !s.is_empty()) {
                                names.push((name_attr.to_string(), id));
                            }
                        }
                    }
                    (ids, names)
                };
                let arr = wrap_node_list(rt, &d, list);
                let mut seen = std::collections::HashSet::new();
                for (k, id) in named {
                    if k != "length" && k != "item" && k.parse::<usize>().is_err() && seen.insert(k.clone()) {
                        let obj = wrap_node(rt, &d, id);
                        let _ = rt.set_property(arr, &k, JsValue::Object(obj));
                    }
                }
                Ok(JsValue::Object(arr))
            }),
        );
        define_getter_setter(rt, doc_proto, prop, Some(gf), None);
    }

    let d = dom.clone();
    let get_links = rt.create_native_function(
        "get links",
        0,
        Rc::new(move |rt, this, _| {
            let list = {
                let st = d.borrow();
                let root = node_id_of(rt, &this).unwrap_or(st.doc.root);
                st.doc
                    .descendants(root)
                    .into_iter()
                    .filter(|&id| {
                        st.doc.element(id).map_or(false, |e| {
                            e.is_html()
                                && (e.tag_is("a") || e.tag_is("area"))
                                && e.get_attr("href").is_some()
                        })
                    })
                    .collect()
            };
            Ok(JsValue::Object(wrap_node_list(rt, &d, list)))
        }),
    );
    define_getter_setter(rt, doc_proto, "links", Some(get_links), None);

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

    let d = dom.clone();
    let create_doc = rt.create_native_function(
        "createDocument",
        3,
        Rc::new(move |rt, _, args| {
            let ns_arg = args.first().cloned().unwrap_or(JsValue::Null);
            let ns_uri = if ns_arg.is_nullish() {
                String::new()
            } else {
                rt.to_string(&ns_arg)?
            };
            let qname_arg = args.get(1).cloned().unwrap_or(JsValue::Null);
            let qname = if qname_arg.is_nullish() {
                String::new()
            } else {
                rt.to_string(&qname_arg)?
            };
            let doctype_id = args.get(2).and_then(|v| node_id_of(rt, v));

            if !qname.is_empty() && !is_valid_xml_name(&qname) {
                return Err(throw_dom_exception(
                    rt,
                    &d,
                    5,
                    "InvalidCharacterError",
                    "Invalid qualified name",
                ));
            }
            let sub = {
                let mut st = d.borrow_mut();
                let sub = st.doc.create_sub_document();
                for c in st.doc.children(sub) {
                    st.doc.detach(c);
                }
                st.xml_documents.insert(sub);
                if let Some(dt) = doctype_id {
                    st.doc.append_child(sub, dt);
                }
                if !qname.is_empty() {
                    let is_html_ns = ns_uri == "http://www.w3.org/1999/xhtml";
                    let ns = if ns_uri.is_empty() {
                        Namespace::Html
                    } else {
                        Namespace::from_uri(&ns_uri)
                    };
                    let root_el = st.doc.create_element_ns(qname, ns, Vec::new());
                    if !is_html_ns {
                        st.xml_elements.insert(root_el);
                    }
                    st.doc.append_child(sub, root_el);
                }
                sub
            };
            Ok(JsValue::Object(wrap_node(rt, &d, sub)))
        }),
    );
    let _ = rt.set_property(impl_obj, "createDocument", JsValue::Object(create_doc));

    let d = dom.clone();
    let create_dt = rt.create_native_function(
        "createDocumentType",
        3,
        Rc::new(move |rt, _, args| {
            let qname = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let pub_id = args.get(1).map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let sys_id = args.get(2).map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            if !is_valid_xml_name(&qname) {
                return Err(throw_dom_exception(
                    rt,
                    &d,
                    5,
                    "InvalidCharacterError",
                    "Invalid doctype name",
                ));
            }
            if let Some(colon) = qname.find(':') {
                let prefix = &qname[..colon];
                let local = &qname[colon + 1..];
                if !is_valid_ncname(prefix) || !is_valid_ncname(local) {
                    return Err(throw_dom_exception(
                        rt,
                        &d,
                        14,
                        "NamespaceError",
                        "Malformed qualified doctype name",
                    ));
                }
            }
            let nid = d.borrow_mut().doc.create_doctype(qname, pub_id, sys_id);
            Ok(JsValue::Object(wrap_node(rt, &d, nid)))
        }),
    );
    let _ = rt.set_property(impl_obj, "createDocumentType", JsValue::Object(create_dt));

    let has_feat = rt.create_native_function("hasFeature", 2, Rc::new(|_, _, _| Ok(JsValue::Boolean(true))));
    let _ = rt.set_property(impl_obj, "hasFeature", JsValue::Object(has_feat));

    let _ = rt.set_property(doc_proto, "implementation", JsValue::Object(impl_obj));

    define_full_range_proto(rt, dom, range_proto);
}
