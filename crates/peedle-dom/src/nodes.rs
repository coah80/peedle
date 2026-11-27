use std::rc::Rc;

use peedle_css::{matches_selector, query_selector, query_selector_all};
use peedle_html::{parse_fragment_into, Namespace, NodeData, NodeId, ShadowRootMode};
use peedle_js::{JsError, JsRuntime, JsValue, ObjectId, PropertyKey};

use crate::canvas_binding::ensure_canvas;
use crate::events::{build_event_object, define_getter_setter, dispatch_event_object};
use crate::state::{Canvas2DHandle, ClassListHandle, NodeHandle, SharedDom};
use crate::style_binding::create_style_proxy;

pub fn node_id_of(rt: &JsRuntime, val: &JsValue) -> Option<NodeId> {
    let oid = val.as_object()?;
    rt.get_host_data::<NodeHandle>(oid).map(|h| h.0)
}

pub fn wrap_node(rt: &mut JsRuntime, dom: &SharedDom, nid: NodeId) -> ObjectId {
    if let Some(&existing) = dom.borrow().node_wrappers.get(&nid) {
        return existing;
    }
    let protos = dom.borrow().protos.unwrap();
    let proto = match &dom.borrow().doc.node(nid).data {
        NodeData::Document { .. } => protos.document,
        NodeData::Element(e) => match e.namespace {
            Namespace::Html => protos.html_element,
            Namespace::Svg => protos.svg_element,
            Namespace::MathMl => protos.element,
        },
        NodeData::Text(_) => protos.text,
        NodeData::Comment(_) => protos.comment,
        NodeData::DocumentFragment => protos.fragment,
        NodeData::ShadowRoot { .. } => protos.shadow_root,
        NodeData::Doctype { .. } => protos.doctype,
        NodeData::ProcessingInstruction { .. } => protos.node,
    };
    let obj = rt.create_object_with_proto(Some(proto));
    rt.set_host_data(obj, NodeHandle(nid));
    dom.borrow_mut().node_wrappers.insert(nid, obj);
    obj
}

pub fn wrap_node_opt(rt: &mut JsRuntime, dom: &SharedDom, opt: Option<NodeId>) -> JsValue {
    match opt {
        Some(nid) => JsValue::Object(wrap_node(rt, dom, nid)),
        None => JsValue::Null,
    }
}

pub fn wrap_node_list(rt: &mut JsRuntime, dom: &SharedDom, ids: Vec<NodeId>) -> ObjectId {
    let items: Vec<JsValue> = ids
        .into_iter()
        .map(|nid| JsValue::Object(wrap_node(rt, dom, nid)))
        .collect();
    let arr = rt.create_array(items);
    let item_fn = rt.create_native_function(
        "item",
        1,
        Rc::new(|rt, this, args| {
            let Some(oid) = this.as_object() else {
                return Ok(JsValue::Null);
            };
            let idx = args.first().and_then(|v| rt.to_number(v).ok()).unwrap_or(0.0) as usize;
            let val = rt.get_property(oid, &idx.to_string())?;
            Ok(if val.is_undefined() { JsValue::Null } else { val })
        }),
    );
    let _ = rt.set_property(arr, "item", JsValue::Object(item_fn));
    arr
}

pub fn define_node_proto(rt: &mut JsRuntime, dom: SharedDom, node_proto: ObjectId) {
    for (name, code) in [
        ("ELEMENT_NODE", 1.0),
        ("ATTRIBUTE_NODE", 2.0),
        ("TEXT_NODE", 3.0),
        ("CDATA_SECTION_NODE", 4.0),
        ("PROCESSING_INSTRUCTION_NODE", 7.0),
        ("COMMENT_NODE", 8.0),
        ("DOCUMENT_NODE", 9.0),
        ("DOCUMENT_TYPE_NODE", 10.0),
        ("DOCUMENT_FRAGMENT_NODE", 11.0),
    ] {
        let _ = rt.set_property(node_proto, name, JsValue::Number(code));
    }

    let d = dom.clone();
    let get_node_type = rt.create_native_function(
        "get nodeType",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Number(0.0));
            };
            let code = match d.borrow().doc.node(nid).data {
                NodeData::Element(_) => 1.0,
                NodeData::Text(_) => 3.0,
                NodeData::ProcessingInstruction { .. } => 7.0,
                NodeData::Comment(_) => 8.0,
                NodeData::Document { .. } => 9.0,
                NodeData::Doctype { .. } => 10.0,
                NodeData::DocumentFragment | NodeData::ShadowRoot { .. } => 11.0,
            };
            Ok(JsValue::Number(code))
        }),
    );
    define_getter_setter(rt, node_proto, "nodeType", Some(get_node_type), None);

    let d = dom.clone();
    let get_node_name = rt.create_native_function(
        "get nodeName",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::String(String::new()));
            };
            let name = match &d.borrow().doc.node(nid).data {
                NodeData::Element(e) => {
                    if e.is_html() {
                        e.tag_name.to_ascii_uppercase()
                    } else {
                        e.tag_name.clone()
                    }
                }
                NodeData::Text(_) => "#text".to_string(),
                NodeData::Comment(_) => "#comment".to_string(),
                NodeData::Document { .. } => "#document".to_string(),
                NodeData::DocumentFragment | NodeData::ShadowRoot { .. } => {
                    "#document-fragment".to_string()
                }
                NodeData::Doctype { name, .. } => name.clone(),
                NodeData::ProcessingInstruction { target, .. } => target.clone(),
            };
            Ok(JsValue::String(name))
        }),
    );
    define_getter_setter(rt, node_proto, "nodeName", Some(get_node_name), None);

    let dg = dom.clone();
    let ds = dom.clone();
    let get_nv = rt.create_native_function(
        "get nodeValue",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let v = match &dg.borrow().doc.node(nid).data {
                NodeData::Text(s) | NodeData::Comment(s) => JsValue::String(s.clone()),
                NodeData::ProcessingInstruction { data, .. } => JsValue::String(data.clone()),
                _ => JsValue::Null,
            };
            Ok(v)
        }),
    );
    let set_nv = rt.create_native_function(
        "set nodeValue",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let s = match args.first() {
                Some(v) if !v.is_nullish() => rt.to_string(v)?,
                _ => String::new(),
            };
            let mut st = ds.borrow_mut();
            st.doc.set_character_data(nid, s);
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, node_proto, "nodeValue", Some(get_nv), Some(set_nv));

    let dg = dom.clone();
    let ds = dom.clone();
    let get_tc = rt.create_native_function(
        "get textContent",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            if matches!(
                dg.borrow().doc.node(nid).data,
                NodeData::Document { .. } | NodeData::Doctype { .. }
            ) {
                return Ok(JsValue::Null);
            }
            let s = dg.borrow().doc.text_content(nid);
            Ok(JsValue::String(s))
        }),
    );
    let set_tc = rt.create_native_function(
        "set textContent",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let s = match args.first() {
                Some(v) if !v.is_nullish() => rt.to_string(v)?,
                _ => String::new(),
            };
            let mut st = ds.borrow_mut();
            st.doc.set_text_content(nid, s);
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, node_proto, "textContent", Some(get_tc), Some(set_tc));

    let d = dom.clone();
    let get_parent = rt.create_native_function(
        "get parentNode",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let p = d.borrow().doc.node(nid).parent;
            Ok(wrap_node_opt(rt, &d, p))
        }),
    );
    define_getter_setter(rt, node_proto, "parentNode", Some(get_parent), None);

    let d = dom.clone();
    let get_pe = rt.create_native_function(
        "get parentElement",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let p = d.borrow().doc.node(nid).parent.filter(|&pid| {
                matches!(d.borrow().doc.node(pid).data, NodeData::Element(_))
            });
            Ok(wrap_node_opt(rt, &d, p))
        }),
    );
    define_getter_setter(rt, node_proto, "parentElement", Some(get_pe), None);

    let d = dom.clone();
    let get_fc = rt.create_native_function(
        "get firstChild",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let c = d.borrow().doc.node(nid).first_child;
            Ok(wrap_node_opt(rt, &d, c))
        }),
    );
    define_getter_setter(rt, node_proto, "firstChild", Some(get_fc), None);

    let d = dom.clone();
    let get_lc = rt.create_native_function(
        "get lastChild",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let c = d.borrow().doc.node(nid).last_child;
            Ok(wrap_node_opt(rt, &d, c))
        }),
    );
    define_getter_setter(rt, node_proto, "lastChild", Some(get_lc), None);

    let d = dom.clone();
    let get_ps = rt.create_native_function(
        "get previousSibling",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let c = d.borrow().doc.node(nid).prev_sibling;
            Ok(wrap_node_opt(rt, &d, c))
        }),
    );
    define_getter_setter(rt, node_proto, "previousSibling", Some(get_ps), None);

    let d = dom.clone();
    let get_ns = rt.create_native_function(
        "get nextSibling",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let c = d.borrow().doc.node(nid).next_sibling;
            Ok(wrap_node_opt(rt, &d, c))
        }),
    );
    define_getter_setter(rt, node_proto, "nextSibling", Some(get_ns), None);

    let d = dom.clone();
    let get_cn = rt.create_native_function(
        "get childNodes",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Object(rt.create_array(Vec::new())));
            };
            let kids = d.borrow().doc.children(nid);
            Ok(JsValue::Object(wrap_node_list(rt, &d, kids)))
        }),
    );
    define_getter_setter(rt, node_proto, "childNodes", Some(get_cn), None);

    let d = dom.clone();
    let get_od = rt.create_native_function(
        "get ownerDocument",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            if nid == d.borrow().doc.root {
                return Ok(JsValue::Null);
            }
            let root = d.borrow().doc.root;
            Ok(JsValue::Object(wrap_node(rt, &d, root)))
        }),
    );
    define_getter_setter(rt, node_proto, "ownerDocument", Some(get_od), None);

    let d = dom.clone();
    let get_conn = rt.create_native_function(
        "get isConnected",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Boolean(false));
            };
            let st = d.borrow();
            let conn = nid == st.doc.root || st.doc.is_ancestor(st.doc.root, nid);
            Ok(JsValue::Boolean(conn))
        }),
    );
    define_getter_setter(rt, node_proto, "isConnected", Some(get_conn), None);

    let d = dom.clone();
    let append_child = rt.create_native_function(
        "appendChild",
        1,
        Rc::new(move |rt, this, args| {
            let Some(parent) = node_id_of(rt, &this) else {
                return Err(JsError::type_err("appendChild requires a Node"));
            };
            let Some(child) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Err(JsError::type_err("appendChild argument must be a Node"));
            };
            {
                let mut st = d.borrow_mut();
                st.doc.append_child(parent, child);
                st.mark_dirty();
            }
            Ok(JsValue::Object(wrap_node(rt, &d, child)))
        }),
    );
    let _ = rt.set_property(node_proto, "appendChild", JsValue::Object(append_child));

    let d = dom.clone();
    let insert_before = rt.create_native_function(
        "insertBefore",
        2,
        Rc::new(move |rt, this, args| {
            let Some(parent) = node_id_of(rt, &this) else {
                return Err(JsError::type_err("insertBefore requires a Node"));
            };
            let Some(child) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Err(JsError::type_err("insertBefore argument must be a Node"));
            };
            let ref_node = args.get(1).and_then(|v| node_id_of(rt, v));
            {
                let mut st = d.borrow_mut();
                st.doc.insert_before(parent, child, ref_node);
                st.mark_dirty();
            }
            Ok(JsValue::Object(wrap_node(rt, &d, child)))
        }),
    );
    let _ = rt.set_property(node_proto, "insertBefore", JsValue::Object(insert_before));

    let d = dom.clone();
    let remove_child = rt.create_native_function(
        "removeChild",
        1,
        Rc::new(move |rt, this, args| {
            let Some(parent) = node_id_of(rt, &this) else {
                return Err(JsError::type_err("removeChild requires a Node"));
            };
            let Some(child) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Err(JsError::type_err("removeChild argument must be a Node"));
            };
            {
                let mut st = d.borrow_mut();
                if st.doc.node(child).parent != Some(parent) {
                    return Err(JsError::error("NotFoundError: child is not a child of this node"));
                }
                st.doc.detach(child);
                st.mark_dirty();
            }
            Ok(JsValue::Object(wrap_node(rt, &d, child)))
        }),
    );
    let _ = rt.set_property(node_proto, "removeChild", JsValue::Object(remove_child));

    let d = dom.clone();
    let replace_child = rt.create_native_function(
        "replaceChild",
        2,
        Rc::new(move |rt, this, args| {
            let Some(parent) = node_id_of(rt, &this) else {
                return Err(JsError::type_err("replaceChild requires a Node"));
            };
            let Some(new_c) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Err(JsError::type_err("replaceChild requires newChild"));
            };
            let Some(old_c) = args.get(1).and_then(|v| node_id_of(rt, v)) else {
                return Err(JsError::type_err("replaceChild requires oldChild"));
            };
            {
                let mut st = d.borrow_mut();
                st.doc.replace_child(parent, new_c, old_c);
                st.mark_dirty();
            }
            Ok(JsValue::Object(wrap_node(rt, &d, old_c)))
        }),
    );
    let _ = rt.set_property(node_proto, "replaceChild", JsValue::Object(replace_child));

    let d = dom.clone();
    let clone_node = rt.create_native_function(
        "cloneNode",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Err(JsError::type_err("cloneNode requires a Node"));
            };
            let deep = args.first().map_or(false, |v| rt.to_boolean(v));
            let cloned = d.borrow_mut().doc.clone_node(nid, deep);
            Ok(JsValue::Object(wrap_node(rt, &d, cloned)))
        }),
    );
    let _ = rt.set_property(node_proto, "cloneNode", JsValue::Object(clone_node));

    let d = dom.clone();
    let contains = rt.create_native_function(
        "contains",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Boolean(false));
            };
            let Some(other) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Ok(JsValue::Boolean(false));
            };
            let ok = nid == other || d.borrow().doc.is_ancestor(nid, other);
            Ok(JsValue::Boolean(ok))
        }),
    );
    let _ = rt.set_property(node_proto, "contains", JsValue::Object(contains));

    let d = dom.clone();
    let has_child = rt.create_native_function(
        "hasChildNodes",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Boolean(false));
            };
            Ok(JsValue::Boolean(d.borrow().doc.node(nid).first_child.is_some()))
        }),
    );
    let _ = rt.set_property(node_proto, "hasChildNodes", JsValue::Object(has_child));

    let d = dom.clone();
    let get_root = rt.create_native_function(
        "getRootNode",
        0,
        Rc::new(move |rt, this, _| {
            let Some(mut cur) = node_id_of(rt, &this) else {
                return Ok(this);
            };
            loop {
                let p = d.borrow().doc.node(cur).parent;
                match p {
                    Some(pid) => cur = pid,
                    None => break,
                }
            }
            Ok(JsValue::Object(wrap_node(rt, &d, cur)))
        }),
    );
    let _ = rt.set_property(node_proto, "getRootNode", JsValue::Object(get_root));

    let d = dom.clone();
    let compare_pos = rt.create_native_function(
        "compareDocumentPosition",
        1,
        Rc::new(move |rt, this, args| {
            let (Some(a), Some(b)) = (node_id_of(rt, &this), args.first().and_then(|v| node_id_of(rt, v))) else {
                return Ok(JsValue::Number(1.0));
            };
            if a == b {
                return Ok(JsValue::Number(0.0));
            }
            let st = d.borrow();
            if st.doc.is_ancestor(a, b) {
                return Ok(JsValue::Number(20.0));
            }
            if st.doc.is_ancestor(b, a) {
                return Ok(JsValue::Number(10.0));
            }
            let desc = st.doc.descendants(st.doc.root);
            let pa = desc.iter().position(|&id| id == a);
            let pb = desc.iter().position(|&id| id == b);
            match (pa, pb) {
                (Some(ia), Some(ib)) => Ok(JsValue::Number(if ia < ib { 4.0 } else { 2.0 })),
                _ => Ok(JsValue::Number(1.0)),
            }
        }),
    );
    let _ = rt.set_property(node_proto, "compareDocumentPosition", JsValue::Object(compare_pos));

    let d = dom.clone();
    let normalize = rt.create_native_function(
        "normalize",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let mut st = d.borrow_mut();
            let kids = st.doc.children(nid);
            let mut prev_text: Option<NodeId> = None;
            for k in kids {
                let text_opt = match &st.doc.node(k).data {
                    NodeData::Text(s) => Some(s.clone()),
                    _ => None,
                };
                if let Some(s) = text_opt {
                    if s.is_empty() {
                        st.doc.detach(k);
                    } else if let Some(pt) = prev_text {
                        if let NodeData::Text(ref mut prev_s) = st.doc.node_mut(pt).data {
                            prev_s.push_str(&s);
                        }
                        st.doc.detach(k);
                    } else {
                        prev_text = Some(k);
                    }
                } else {
                    prev_text = None;
                }
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(node_proto, "normalize", JsValue::Object(normalize));

    let is_same = rt.create_native_function(
        "isSameNode",
        1,
        Rc::new(|rt, this, args| {
            let ok = node_id_of(rt, &this) == args.first().and_then(|v| node_id_of(rt, v));
            Ok(JsValue::Boolean(ok))
        }),
    );
    let _ = rt.set_property(node_proto, "isSameNode", JsValue::Object(is_same));
}

pub fn define_element_proto(
    rt: &mut JsRuntime,
    dom: SharedDom,
    element_proto: ObjectId,
    html_element_proto: ObjectId,
) {
    let d = dom.clone();
    let get_tag = rt.create_native_function(
        "get tagName",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::String(String::new()));
            };
            let s = d
                .borrow()
                .doc
                .element(nid)
                .map(|e| {
                    if e.is_html() {
                        e.tag_name.to_ascii_uppercase()
                    } else {
                        e.tag_name.clone()
                    }
                })
                .unwrap_or_default();
            Ok(JsValue::String(s))
        }),
    );
    define_getter_setter(rt, element_proto, "tagName", Some(get_tag), None);

    let d = dom.clone();
    let get_local = rt.create_native_function(
        "get localName",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::String(String::new()));
            };
            let s = d
                .borrow()
                .doc
                .element(nid)
                .map(|e| e.tag_name.clone())
                .unwrap_or_default();
            Ok(JsValue::String(s))
        }),
    );
    define_getter_setter(rt, element_proto, "localName", Some(get_local), None);

    let d = dom.clone();
    let get_ns = rt.create_native_function(
        "get namespaceURI",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let uri = d
                .borrow()
                .doc
                .element(nid)
                .map(|e| JsValue::String(e.namespace.uri().to_string()))
                .unwrap_or(JsValue::Null);
            Ok(uri)
        }),
    );
    define_getter_setter(rt, element_proto, "namespaceURI", Some(get_ns), None);

    define_attr_prop(rt, &dom, element_proto, "id", "id");
    define_attr_prop(rt, &dom, element_proto, "className", "class");
    define_attr_prop(rt, &dom, html_element_proto, "title", "title");
    define_attr_prop(rt, &dom, html_element_proto, "name", "name");
    define_attr_prop(rt, &dom, html_element_proto, "type", "type");
    define_attr_prop(rt, &dom, html_element_proto, "href", "href");
    define_attr_prop(rt, &dom, html_element_proto, "src", "src");
    define_attr_prop(rt, &dom, html_element_proto, "alt", "alt");
    define_attr_prop(rt, &dom, html_element_proto, "placeholder", "placeholder");
    define_attr_prop(rt, &dom, html_element_proto, "rel", "rel");
    define_attr_prop(rt, &dom, html_element_proto, "action", "action");
    define_attr_prop(rt, &dom, html_element_proto, "method", "method");
    define_attr_prop(rt, &dom, html_element_proto, "role", "role");
    define_attr_prop(rt, &dom, html_element_proto, "htmlFor", "for");

    let dg = dom.clone();
    let ds = dom.clone();
    let get_val = rt.create_native_function(
        "get value",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::String(String::new()));
            };
            let s = dg
                .borrow()
                .doc
                .element(nid)
                .map(|e| e.current_value())
                .unwrap_or_default();
            Ok(JsValue::String(s))
        }),
    );
    let set_val = rt.create_native_function(
        "set value",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let s = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mut st = ds.borrow_mut();
            if let Some(el) = st.doc.element_mut(nid) {
                el.input_value = Some(s);
            }
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, html_element_proto, "value", Some(get_val), Some(set_val));

    let dg = dom.clone();
    let ds = dom.clone();
    let get_chk = rt.create_native_function(
        "get checked",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Boolean(false));
            };
            let c = dg.borrow().doc.element(nid).map_or(false, |e| e.is_checked());
            Ok(JsValue::Boolean(c))
        }),
    );
    let set_chk = rt.create_native_function(
        "set checked",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let b = args.first().map_or(false, |v| rt.to_boolean(v));
            let mut st = ds.borrow_mut();
            if let Some(el) = st.doc.element_mut(nid) {
                el.checked = Some(b);
            }
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, html_element_proto, "checked", Some(get_chk), Some(set_chk));

    let dg = dom.clone();
    let ds = dom.clone();
    let get_dis = rt.create_native_function(
        "get disabled",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Boolean(false));
            };
            Ok(JsValue::Boolean(dg.borrow().doc.element(nid).map_or(false, |e| e.is_disabled())))
        }),
    );
    let set_dis = rt.create_native_function(
        "set disabled",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let b = args.first().map_or(false, |v| rt.to_boolean(v));
            let mut st = ds.borrow_mut();
            if b {
                st.doc.set_attribute(nid, "disabled", "");
            } else {
                st.doc.remove_attribute(nid, "disabled");
            }
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, html_element_proto, "disabled", Some(get_dis), Some(set_dis));

    let d = dom.clone();
    let get_style = rt.create_native_function(
        "get style",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Object(rt.create_object()));
            };
            let proto = d.borrow().protos.unwrap().css_style_decl;
            let proxy = create_style_proxy(rt, &d, nid, proto);
            Ok(JsValue::Object(proxy))
        }),
    );
    define_getter_setter(rt, html_element_proto, "style", Some(get_style), None);

    let d = dom.clone();
    let get_cl = rt.create_native_function(
        "get classList",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Object(rt.create_object()));
            };
            let proto = d.borrow().protos.unwrap().dom_token_list;
            let obj = rt.create_object_with_proto(Some(proto));
            rt.set_host_data(obj, ClassListHandle(nid));
            Ok(JsValue::Object(obj))
        }),
    );
    define_getter_setter(rt, element_proto, "classList", Some(get_cl), None);

    let d = dom.clone();
    let get_dataset = rt.create_native_function(
        "get dataset",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Object(rt.create_object()));
            };
            let target = rt.create_object();
            if let Some(el) = d.borrow().doc.element(nid) {
                for a in &el.attributes {
                    if let Some(rest) = a.name.strip_prefix("data-") {
                        let camel = kebab_to_camel(rest);
                        let _ = rt.set_property(target, &camel, JsValue::String(a.value.clone()));
                    }
                }
            }
            let handler = rt.create_object();
            let ds = d.clone();
            let set_t = rt.create_native_function(
                "set",
                4,
                Rc::new(move |rt, _, args| {
                    let key = args.get(1).map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
                    let val = args.get(2).map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
                    let attr = format!("data-{}", crate::style_binding::camel_to_kebab(&key));
                    ds.borrow_mut().doc.set_attribute(nid, attr, val.clone());
                    if let Some(t) = args.first().and_then(|v| v.as_object()) {
                        let _ = rt.set_property(t, &key, JsValue::String(val));
                    }
                    Ok(JsValue::Boolean(true))
                }),
            );
            let _ = rt.set_property(handler, "set", JsValue::Object(set_t));
            let proxy_ctor = rt.get_property(rt.global_object(), "Proxy").unwrap_or(JsValue::Undefined);
            if let Ok(p) = rt.construct(proxy_ctor, &[JsValue::Object(target), JsValue::Object(handler)]) {
                Ok(p)
            } else {
                Ok(JsValue::Object(target))
            }
        }),
    );
    define_getter_setter(rt, html_element_proto, "dataset", Some(get_dataset), None);

    let d = dom.clone();
    let get_attr = rt.create_native_function(
        "getAttribute",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let name = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let val = d
                .borrow()
                .doc
                .element(nid)
                .and_then(|e| e.get_attr(&name).map(|s| JsValue::String(s.to_string())))
                .unwrap_or(JsValue::Null);
            Ok(val)
        }),
    );
    let _ = rt.set_property(element_proto, "getAttribute", JsValue::Object(get_attr));

    let d = dom.clone();
    let set_attr = rt.create_native_function(
        "setAttribute",
        2,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let name = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let val = args.get(1).map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mut st = d.borrow_mut();
            st.doc.set_attribute(nid, name, val);
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(element_proto, "setAttribute", JsValue::Object(set_attr));

    let d = dom.clone();
    let rem_attr = rt.create_native_function(
        "removeAttribute",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let name = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mut st = d.borrow_mut();
            st.doc.remove_attribute(nid, &name);
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(element_proto, "removeAttribute", JsValue::Object(rem_attr));

    let d = dom.clone();
    let has_attr = rt.create_native_function(
        "hasAttribute",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Boolean(false));
            };
            let name = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            Ok(JsValue::Boolean(
                d.borrow().doc.element(nid).map_or(false, |e| e.has_attr(&name)),
            ))
        }),
    );
    let _ = rt.set_property(element_proto, "hasAttribute", JsValue::Object(has_attr));

    let d = dom.clone();
    let set_attr_ns = rt.create_native_function(
        "setAttributeNS",
        3,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let name = args.get(1).map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let val = args.get(2).map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mut st = d.borrow_mut();
            st.doc.set_attribute(nid, name, val);
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(element_proto, "setAttributeNS", JsValue::Object(set_attr_ns));

    let d = dom.clone();
    let rem_attr_ns = rt.create_native_function(
        "removeAttributeNS",
        2,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let name = args.get(1).map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mut st = d.borrow_mut();
            st.doc.remove_attribute(nid, &name);
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(element_proto, "removeAttributeNS", JsValue::Object(rem_attr_ns));

    let d = dom.clone();
    let get_attrs = rt.create_native_function(
        "get attributes",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Object(rt.create_array(Vec::new())));
            };
            let attrs = d
                .borrow()
                .doc
                .element(nid)
                .map(|e| e.attributes.clone())
                .unwrap_or_default();
            let mut items = Vec::with_capacity(attrs.len());
            for a in attrs {
                let ao = rt.create_object();
                let _ = rt.set_property(ao, "name", JsValue::String(a.name));
                let _ = rt.set_property(ao, "value", JsValue::String(a.value));
                items.push(JsValue::Object(ao));
            }
            Ok(JsValue::Object(rt.create_array(items)))
        }),
    );
    define_getter_setter(rt, element_proto, "attributes", Some(get_attrs), None);

    let dg = dom.clone();
    let ds = dom.clone();
    let get_ih = rt.create_native_function(
        "get innerHTML",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::String(String::new()));
            };
            Ok(JsValue::String(dg.borrow().doc.inner_html(nid)))
        }),
    );
    let set_ih = rt.create_native_function(
        "set innerHTML",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let html = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mut st = ds.borrow_mut();
            let (tag, ns, target) = match st.doc.element(nid) {
                Some(e) => (
                    e.tag_name.clone(),
                    e.namespace,
                    e.template_contents.unwrap_or(nid),
                ),
                None => ("div".to_string(), Namespace::Html, nid),
            };
            let old_kids = st.doc.children(target);
            for k in old_kids {
                st.doc.detach(k);
            }
            if !html.is_empty() {
                let new_nodes = parse_fragment_into(&mut st.doc, &tag, ns, &html);
                for n in new_nodes {
                    st.doc.append_child(target, n);
                }
            }
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, element_proto, "innerHTML", Some(get_ih), Some(set_ih));

    let dg = dom.clone();
    let ds = dom.clone();
    let get_oh = rt.create_native_function(
        "get outerHTML",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::String(String::new()));
            };
            Ok(JsValue::String(dg.borrow().doc.outer_html(nid)))
        }),
    );
    let set_oh = rt.create_native_function(
        "set outerHTML",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let html = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mut st = ds.borrow_mut();
            let Some(parent) = st.doc.node(nid).parent else {
                return Ok(JsValue::Undefined);
            };
            let (ptag, pns) = st
                .doc
                .element(parent)
                .map(|e| (e.tag_name.clone(), e.namespace))
                .unwrap_or_else(|| ("div".to_string(), Namespace::Html));
            let new_nodes = parse_fragment_into(&mut st.doc, &ptag, pns, &html);
            for n in new_nodes {
                st.doc.insert_before(parent, n, Some(nid));
            }
            st.doc.detach(nid);
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, element_proto, "outerHTML", Some(get_oh), Some(set_oh));

    let d = dom.clone();
    let insert_adj_html = rt.create_native_function(
        "insertAdjacentHTML",
        2,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let pos = args
                .first()
                .map(|v| rt.to_string(v))
                .transpose()?
                .unwrap_or_default()
                .to_ascii_lowercase();
            let html = args.get(1).map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mut st = d.borrow_mut();
            let (tag, ns) = st
                .doc
                .element(nid)
                .map(|e| (e.tag_name.clone(), e.namespace))
                .unwrap_or_else(|| ("div".to_string(), Namespace::Html));
            let nodes = parse_fragment_into(&mut st.doc, &tag, ns, &html);
            match pos.as_str() {
                "beforebegin" => {
                    if let Some(p) = st.doc.node(nid).parent {
                        for n in nodes {
                            st.doc.insert_before(p, n, Some(nid));
                        }
                    }
                }
                "afterbegin" => {
                    let fc = st.doc.node(nid).first_child;
                    for n in nodes {
                        st.doc.insert_before(nid, n, fc);
                    }
                }
                "beforeend" => {
                    for n in nodes {
                        st.doc.append_child(nid, n);
                    }
                }
                "afterend" => {
                    if let Some(p) = st.doc.node(nid).parent {
                        let nsib = st.doc.node(nid).next_sibling;
                        for n in nodes {
                            st.doc.insert_before(p, n, nsib);
                        }
                    }
                }
                _ => {}
            }
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(element_proto, "insertAdjacentHTML", JsValue::Object(insert_adj_html));

    let d = dom.clone();
    let get_children = rt.create_native_function(
        "get children",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Object(rt.create_array(Vec::new())));
            };
            let kids = d.borrow().doc.element_children(nid);
            Ok(JsValue::Object(wrap_node_list(rt, &d, kids)))
        }),
    );
    define_getter_setter(rt, element_proto, "children", Some(get_children), None);

    let d = dom.clone();
    let get_fec = rt.create_native_function(
        "get firstElementChild",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let first = d.borrow().doc.element_children(nid).into_iter().next();
            Ok(wrap_node_opt(rt, &d, first))
        }),
    );
    define_getter_setter(rt, element_proto, "firstElementChild", Some(get_fec), None);

    let d = dom.clone();
    let get_lec = rt.create_native_function(
        "get lastElementChild",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let last = d.borrow().doc.element_children(nid).into_iter().last();
            Ok(wrap_node_opt(rt, &d, last))
        }),
    );
    define_getter_setter(rt, element_proto, "lastElementChild", Some(get_lec), None);

    let d = dom.clone();
    let get_nes = rt.create_native_function(
        "get nextElementSibling",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let mut cur = d.borrow().doc.node(nid).next_sibling;
            while let Some(c) = cur {
                if matches!(d.borrow().doc.node(c).data, NodeData::Element(_)) {
                    return Ok(JsValue::Object(wrap_node(rt, &d, c)));
                }
                cur = d.borrow().doc.node(c).next_sibling;
            }
            Ok(JsValue::Null)
        }),
    );
    define_getter_setter(rt, element_proto, "nextElementSibling", Some(get_nes), None);

    let d = dom.clone();
    let get_pes = rt.create_native_function(
        "get previousElementSibling",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let mut cur = d.borrow().doc.node(nid).prev_sibling;
            while let Some(c) = cur {
                if matches!(d.borrow().doc.node(c).data, NodeData::Element(_)) {
                    return Ok(JsValue::Object(wrap_node(rt, &d, c)));
                }
                cur = d.borrow().doc.node(c).prev_sibling;
            }
            Ok(JsValue::Null)
        }),
    );
    define_getter_setter(rt, element_proto, "previousElementSibling", Some(get_pes), None);

    let d = dom.clone();
    let get_cec = rt.create_native_function(
        "get childElementCount",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Number(0.0));
            };
            let len = d.borrow().doc.element_children(nid).len();
            Ok(JsValue::Number(len as f64))
        }),
    );
    define_getter_setter(rt, element_proto, "childElementCount", Some(get_cec), None);

    let d = dom.clone();
    let qs = rt.create_native_function(
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
    let _ = rt.set_property(element_proto, "querySelector", JsValue::Object(qs));

    let d = dom.clone();
    let qsa = rt.create_native_function(
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
    let _ = rt.set_property(element_proto, "querySelectorAll", JsValue::Object(qsa));

    let d = dom.clone();
    let matches_fn = rt.create_native_function(
        "matches",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Boolean(false));
            };
            let sel = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let ok = matches_selector(&d.borrow().doc, nid, &sel);
            Ok(JsValue::Boolean(ok))
        }),
    );
    let _ = rt.set_property(element_proto, "matches", JsValue::Object(matches_fn));

    let d = dom.clone();
    let closest_fn = rt.create_native_function(
        "closest",
        1,
        Rc::new(move |rt, this, args| {
            let Some(mut cur) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let sel = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            loop {
                if matches_selector(&d.borrow().doc, cur, &sel) {
                    return Ok(JsValue::Object(wrap_node(rt, &d, cur)));
                }
                let p = d.borrow().doc.node(cur).parent;
                match p {
                    Some(pid) if matches!(d.borrow().doc.node(pid).data, NodeData::Element(_)) => {
                        cur = pid;
                    }
                    _ => return Ok(JsValue::Null),
                }
            }
        }),
    );
    let _ = rt.set_property(element_proto, "closest", JsValue::Object(closest_fn));

    let d = dom.clone();
    let gebtn = rt.create_native_function(
        "getElementsByTagName",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Object(rt.create_array(Vec::new())));
            };
            let tag = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let list = d.borrow().doc.get_elements_by_tag_name(nid, &tag);
            Ok(JsValue::Object(wrap_node_list(rt, &d, list)))
        }),
    );
    let _ = rt.set_property(element_proto, "getElementsByTagName", JsValue::Object(gebtn));

    let d = dom.clone();
    let gebcn = rt.create_native_function(
        "getElementsByClassName",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Object(rt.create_array(Vec::new())));
            };
            let cls = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let list = d.borrow().doc.get_elements_by_class_name(nid, &cls);
            Ok(JsValue::Object(wrap_node_list(rt, &d, list)))
        }),
    );
    let _ = rt.set_property(element_proto, "getElementsByClassName", JsValue::Object(gebcn));

    let d = dom.clone();
    let remove_fn = rt.create_native_function(
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
    let _ = rt.set_property(element_proto, "remove", JsValue::Object(remove_fn));

    let d = dom.clone();
    let append_fn = rt.create_native_function(
        "append",
        1,
        Rc::new(move |rt, this, args| {
            let Some(parent) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            for arg in args {
                let child = if let Some(cid) = node_id_of(rt, arg) {
                    cid
                } else {
                    let s = rt.to_string(arg)?;
                    d.borrow_mut().doc.create_text(s)
                };
                let mut st = d.borrow_mut();
                st.doc.append_child(parent, child);
                st.mark_dirty();
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(element_proto, "append", JsValue::Object(append_fn));

    let d = dom.clone();
    let prepend_fn = rt.create_native_function(
        "prepend",
        1,
        Rc::new(move |rt, this, args| {
            let Some(parent) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let first = d.borrow().doc.node(parent).first_child;
            for arg in args {
                let child = if let Some(cid) = node_id_of(rt, arg) {
                    cid
                } else {
                    let s = rt.to_string(arg)?;
                    d.borrow_mut().doc.create_text(s)
                };
                let mut st = d.borrow_mut();
                st.doc.insert_before(parent, child, first);
                st.mark_dirty();
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(element_proto, "prepend", JsValue::Object(prepend_fn));

    let d = dom.clone();
    let attach_shadow = rt.create_native_function(
        "attachShadow",
        1,
        Rc::new(move |rt, this, args| {
            let Some(host) = node_id_of(rt, &this) else {
                return Err(JsError::type_err("attachShadow requires Element"));
            };
            let mode = args
                .first()
                .and_then(|v| v.as_object())
                .and_then(|o| rt.get_property(o, "mode").ok())
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "open".to_string());
            let smode = if mode == "closed" {
                ShadowRootMode::Closed
            } else {
                ShadowRootMode::Open
            };
            let sr = d.borrow_mut().doc.create_shadow_root(host, smode);
            Ok(JsValue::Object(wrap_node(rt, &d, sr)))
        }),
    );
    let _ = rt.set_property(element_proto, "attachShadow", JsValue::Object(attach_shadow));

    let d = dom.clone();
    let get_sr = rt.create_native_function(
        "get shadowRoot",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let sr = d.borrow().doc.element(nid).and_then(|e| e.shadow_root);
            if let Some(srid) = sr {
                let is_open = matches!(
                    d.borrow().doc.node(srid).data,
                    NodeData::ShadowRoot {
                        mode: ShadowRootMode::Open,
                        ..
                    }
                );
                if is_open {
                    return Ok(JsValue::Object(wrap_node(rt, &d, srid)));
                }
            }
            Ok(JsValue::Null)
        }),
    );
    define_getter_setter(rt, element_proto, "shadowRoot", Some(get_sr), None);

    let d = dom.clone();
    let get_tpl_content = rt.create_native_function(
        "get content",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let tc = d.borrow().doc.element(nid).and_then(|e| e.template_contents);
            Ok(match tc {
                Some(id) => JsValue::Object(wrap_node(rt, &d, id)),
                None => JsValue::Undefined,
            })
        }),
    );
    define_getter_setter(rt, html_element_proto, "content", Some(get_tpl_content), None);

    let d = dom.clone();
    let gbcr = rt.create_native_function(
        "getBoundingClientRect",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Object(make_dom_rect(rt, 0.0, 0.0, 0.0, 0.0)));
            };
            let rect = {
                let mut st = d.borrow_mut();
                st.ensure_layout();
                st.layout.bounding_rect(nid).unwrap_or_default()
            };
            Ok(JsValue::Object(make_dom_rect(
                rt,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
            )))
        }),
    );
    let _ = rt.set_property(element_proto, "getBoundingClientRect", JsValue::Object(gbcr));

    for (prop, field) in [
        ("offsetWidth", 0u8),
        ("offsetHeight", 1),
        ("offsetLeft", 2),
        ("offsetTop", 3),
        ("clientWidth", 0),
        ("clientHeight", 1),
        ("scrollWidth", 0),
        ("scrollHeight", 1),
    ] {
        let dg = dom.clone();
        let gf = rt.create_native_function(
            prop,
            0,
            Rc::new(move |rt, this, _| {
                let Some(nid) = node_id_of(rt, &this) else {
                    return Ok(JsValue::Number(0.0));
                };
                let r = {
                    let mut st = dg.borrow_mut();
                    st.ensure_layout();
                    st.layout.bounding_rect(nid).unwrap_or_default()
                };
                let val = match field {
                    0 => r.width,
                    1 => r.height,
                    2 => r.x,
                    _ => r.y,
                };
                Ok(JsValue::Number(val as f64))
            }),
        );
        define_getter_setter(rt, element_proto, prop, Some(gf), None);
    }

    let d = dom.clone();
    let click_fn = rt.create_native_function(
        "click",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let ev_proto = d.borrow().protos.unwrap().event;
            let ev = build_event_object(rt, ev_proto, "click".to_string(), true, true, true);
            let _ = dispatch_event_object(rt, &d, Some(nid), ev)?;
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(html_element_proto, "click", JsValue::Object(click_fn));

    let d = dom.clone();
    let focus_fn = rt.create_native_function(
        "focus",
        0,
        Rc::new(move |rt, this, _| {
            if let Some(nid) = node_id_of(rt, &this) {
                let mut st = d.borrow_mut();
                if let Some(prev) = st.focused_node {
                    if let Some(el) = st.doc.element_mut(prev) {
                        el.focused = false;
                    }
                }
                if let Some(el) = st.doc.element_mut(nid) {
                    el.focused = true;
                }
                st.focused_node = Some(nid);
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(html_element_proto, "focus", JsValue::Object(focus_fn));

    let d = dom.clone();
    let blur_fn = rt.create_native_function(
        "blur",
        0,
        Rc::new(move |rt, this, _| {
            if let Some(nid) = node_id_of(rt, &this) {
                let mut st = d.borrow_mut();
                if let Some(el) = st.doc.element_mut(nid) {
                    el.focused = false;
                }
                if st.focused_node == Some(nid) {
                    st.focused_node = None;
                }
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(html_element_proto, "blur", JsValue::Object(blur_fn));

    let d = dom.clone();
    let get_ctx = rt.create_native_function(
        "getContext",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let kind = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            if kind != "2d" {
                return Ok(JsValue::Null);
            }
            ensure_canvas(&d, nid);
            let proto = d.borrow().protos.unwrap().canvas2d;
            let obj = rt.create_object_with_proto(Some(proto));
            rt.set_host_data(obj, Canvas2DHandle(nid));
            let _ = rt.set_property(obj, "canvas", this);
            Ok(JsValue::Object(obj))
        }),
    );
    let _ = rt.set_property(html_element_proto, "getContext", JsValue::Object(get_ctx));

    let dg = dom.clone();
    let ds = dom.clone();
    let get_it = rt.create_native_function(
        "get innerText",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::String(String::new()));
            };
            Ok(JsValue::String(dg.borrow().doc.text_content(nid)))
        }),
    );
    let set_it = rt.create_native_function(
        "set innerText",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let s = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mut st = ds.borrow_mut();
            st.doc.set_text_content(nid, s);
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, html_element_proto, "innerText", Some(get_it), Some(set_it));
}

fn define_attr_prop(
    rt: &mut JsRuntime,
    dom: &SharedDom,
    proto: ObjectId,
    prop_name: &str,
    attr_name: &'static str,
) {
    let dg = dom.clone();
    let ds = dom.clone();
    let getter = rt.create_native_function(
        prop_name,
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::String(String::new()));
            };
            let val = dg
                .borrow()
                .doc
                .element(nid)
                .and_then(|e| e.get_attr(attr_name))
                .unwrap_or("")
                .to_string();
            Ok(JsValue::String(val))
        }),
    );
    let setter = rt.create_native_function(
        prop_name,
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let s = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mut st = ds.borrow_mut();
            st.doc.set_attribute(nid, attr_name, s);
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.define_property(
        proto,
        PropertyKey::from(prop_name),
        peedle_js::PropertyDescriptor {
            value: None,
            writable: None,
            get: Some(JsValue::Object(getter)),
            set: Some(JsValue::Object(setter)),
            enumerable: Some(true),
            configurable: Some(true),
        },
    );
}

fn make_dom_rect(rt: &mut JsRuntime, x: f32, y: f32, w: f32, h: f32) -> ObjectId {
    let obj = rt.create_object();
    let _ = rt.set_property(obj, "x", JsValue::Number(x as f64));
    let _ = rt.set_property(obj, "y", JsValue::Number(y as f64));
    let _ = rt.set_property(obj, "left", JsValue::Number(x as f64));
    let _ = rt.set_property(obj, "top", JsValue::Number(y as f64));
    let _ = rt.set_property(obj, "width", JsValue::Number(w as f64));
    let _ = rt.set_property(obj, "height", JsValue::Number(h as f64));
    let _ = rt.set_property(obj, "right", JsValue::Number((x + w) as f64));
    let _ = rt.set_property(obj, "bottom", JsValue::Number((y + h) as f64));
    obj
}

fn kebab_to_camel(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut cap = false;
    for c in s.chars() {
        if c == '-' {
            cap = true;
        } else if cap {
            out.push(c.to_ascii_uppercase());
            cap = false;
        } else {
            out.push(c);
        }
    }
    out
}
