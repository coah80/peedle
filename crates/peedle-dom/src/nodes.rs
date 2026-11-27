use std::rc::Rc;

use peedle_css::{matches_selector, query_selector, query_selector_all, CalcContext};
use peedle_html::{parse_fragment_into, Namespace, NodeData, NodeId, ShadowRootMode};
use peedle_js::{JsError, JsRuntime, JsValue, ObjectId, PropertyKey};

use crate::canvas_binding::ensure_canvas;
use crate::events::{build_event_object, define_getter_setter, dispatch_event_object};
use crate::state::{Canvas2DHandle, ClassListHandle, NodeHandle, SharedDom};
use crate::style_binding::create_style_proxy;
use crate::traversal::throw_dom_exception;

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
            let name = {
                let st = d.borrow();
                match &st.doc.node(nid).data {
                    NodeData::Element(e) => {
                        if e.is_html() && !st.xml_elements.contains(&nid) {
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
                }
            };
            Ok(JsValue::String(name))
        }),
    );
    let _ = rt.set_property(node_proto, "nodeName", JsValue::Undefined);
    define_getter_setter(rt, node_proto, "nodeName", Some(get_node_name), None);

    let get_null = rt.create_native_function("get null", 0, Rc::new(|_, _, _| Ok(JsValue::Null)));
    define_getter_setter(rt, node_proto, "localName", Some(get_null), None);
    define_getter_setter(rt, node_proto, "prefix", Some(get_null), None);
    define_getter_setter(rt, node_proto, "namespaceURI", Some(get_null), None);

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
            let old_kids = st.doc.children(nid);
            for k in old_kids {
                st.notify_pre_remove(k);
            }
            st.doc.set_text_content(nid, s);
            st.invalidate_style_owner(nid);
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
            let owner = {
                let st = d.borrow();
                if matches!(st.doc.node(nid).data, NodeData::Document { .. }) {
                    None
                } else {
                    let mut cur = st.doc.node(nid).parent;
                    let mut found = None;
                    while let Some(pid) = cur {
                        if matches!(st.doc.node(pid).data, NodeData::Document { .. }) {
                            found = Some(pid);
                            break;
                        }
                        cur = st.doc.node(pid).parent;
                    }
                    Some(found.unwrap_or(st.doc.root))
                }
            };
            Ok(wrap_node_opt(rt, &d, owner))
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
            let bad_hierarchy = {
                let st = d.borrow();
                let parent_cannot_have_children = matches!(
                    st.doc.node(parent).data,
                    NodeData::Text(_)
                        | NodeData::Comment(_)
                        | NodeData::Doctype { .. }
                        | NodeData::ProcessingInstruction { .. }
                );
                let child_is_doc = matches!(st.doc.node(child).data, NodeData::Document { .. });
                let parent_is_doc = matches!(st.doc.node(parent).data, NodeData::Document { .. });
                let dup_doc_el = parent_is_doc
                    && matches!(st.doc.node(child).data, NodeData::Element(_))
                    && st.doc.element_children(parent).iter().any(|&c| c != child);
                let text_in_doc = parent_is_doc && matches!(st.doc.node(child).data, NodeData::Text(_));
                parent_cannot_have_children
                    || child_is_doc
                    || dup_doc_el
                    || text_in_doc
                    || child == parent
                    || st.doc.is_ancestor(child, parent)
            };
            if bad_hierarchy {
                return Err(throw_dom_exception(
                    rt,
                    &d,
                    3,
                    "HierarchyRequestError",
                    "Invalid node hierarchy",
                ));
            }
            {
                let mut st = d.borrow_mut();
                let to_insert: Vec<NodeId> = if matches!(st.doc.node(child).data, NodeData::DocumentFragment) {
                    st.doc.children(child)
                } else {
                    vec![child]
                };
                for &c in &to_insert {
                    if st.doc.node(c).parent.is_some() {
                        st.notify_pre_remove(c);
                    }
                }
                let ins_idx = st.doc.children(parent).len();
                st.doc.append_child(parent, child);
                st.notify_post_insert(parent, ins_idx, to_insert.len());
                st.invalidate_style_owner(parent);
                for &c in &to_insert {
                    st.queue_subdoc_loads_in(c);
                }
                st.mark_dirty_for(parent);
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
            let (bad_hierarchy, bad_ref) = {
                let st = d.borrow();
                let parent_cannot = matches!(
                    st.doc.node(parent).data,
                    NodeData::Text(_)
                        | NodeData::Comment(_)
                        | NodeData::Doctype { .. }
                        | NodeData::ProcessingInstruction { .. }
                );
                let child_is_doc = matches!(st.doc.node(child).data, NodeData::Document { .. });
                let parent_is_doc = matches!(st.doc.node(parent).data, NodeData::Document { .. });
                let dup_doc_el = parent_is_doc
                    && matches!(st.doc.node(child).data, NodeData::Element(_))
                    && st.doc.element_children(parent).iter().any(|&c| c != child);
                let text_in_doc = parent_is_doc && matches!(st.doc.node(child).data, NodeData::Text(_));
                let bad_h = parent_cannot
                    || child_is_doc
                    || dup_doc_el
                    || text_in_doc
                    || child == parent
                    || st.doc.is_ancestor(child, parent);
                let bad_r = ref_node.map_or(false, |rn| st.doc.node(rn).parent != Some(parent));
                (bad_h, bad_r)
            };
            if bad_hierarchy {
                return Err(throw_dom_exception(
                    rt,
                    &d,
                    3,
                    "HierarchyRequestError",
                    "Invalid node hierarchy",
                ));
            }
            if bad_ref {
                return Err(throw_dom_exception(
                    rt,
                    &d,
                    8,
                    "NotFoundError",
                    "Reference node is not a child of this node",
                ));
            }
            {
                let mut st = d.borrow_mut();
                let to_insert: Vec<NodeId> = if matches!(st.doc.node(child).data, NodeData::DocumentFragment) {
                    st.doc.children(child)
                } else {
                    vec![child]
                };
                for &c in &to_insert {
                    if st.doc.node(c).parent.is_some() {
                        st.notify_pre_remove(c);
                    }
                }
                let ins_idx = match ref_node {
                    Some(rn) => st
                        .doc
                        .children(parent)
                        .iter()
                        .position(|&x| x == rn)
                        .unwrap_or_else(|| st.doc.children(parent).len()),
                    None => st.doc.children(parent).len(),
                };
                st.doc.insert_before(parent, child, ref_node);
                st.notify_post_insert(parent, ins_idx, to_insert.len());
                st.invalidate_style_owner(parent);
                for &c in &to_insert {
                    st.queue_subdoc_loads_in(c);
                }
                st.mark_dirty_for(parent);
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
            let not_child = d.borrow().doc.node(child).parent != Some(parent);
            if not_child {
                return Err(throw_dom_exception(
                    rt,
                    &d,
                    8,
                    "NotFoundError",
                    "child is not a child of this node",
                ));
            }
            {
                let mut st = d.borrow_mut();
                st.notify_pre_remove(child);
                st.invalidate_style_owner(parent);
                st.doc.detach(child);
                st.mark_dirty_for(parent);
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
                let to_insert: Vec<NodeId> = if matches!(st.doc.node(new_c).data, NodeData::DocumentFragment) {
                    st.doc.children(new_c)
                } else {
                    vec![new_c]
                };
                for &c in &to_insert {
                    if st.doc.node(c).parent.is_some() {
                        st.notify_pre_remove(c);
                    }
                }
                st.notify_pre_remove(old_c);
                st.doc.replace_child(parent, new_c, old_c);
                st.invalidate_style_owner(parent);
                for &c in &to_insert {
                    st.queue_subdoc_loads_in(c);
                }
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
            let s = {
                let st = d.borrow();
                st.doc
                    .element(nid)
                    .map(|e| {
                        if e.is_html() && !st.xml_elements.contains(&nid) {
                            e.tag_name.to_ascii_uppercase()
                        } else {
                            e.tag_name.clone()
                        }
                    })
                    .unwrap_or_default()
            };
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
                return Ok(JsValue::Null);
            };
            let s = d
                .borrow()
                .doc
                .element(nid)
                .map(|e| {
                    e.tag_name
                        .split_once(':')
                        .map(|(_, local)| local.to_string())
                        .unwrap_or_else(|| e.tag_name.clone())
                });
            Ok(s.map(JsValue::String).unwrap_or(JsValue::Null))
        }),
    );
    define_getter_setter(rt, element_proto, "localName", Some(get_local), None);

    let d = dom.clone();
    let get_prefix = rt.create_native_function(
        "get prefix",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let p = d
                .borrow()
                .doc
                .element(nid)
                .and_then(|e| e.tag_name.split_once(':').map(|(pfx, _)| pfx.to_string()));
            Ok(p.map(JsValue::String).unwrap_or(JsValue::Null))
        }),
    );
    define_getter_setter(rt, element_proto, "prefix", Some(get_prefix), None);

    let d = dom.clone();
    let get_ns = rt.create_native_function(
        "get namespaceURI",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let st = d.borrow();
            if let Some(custom) = st.custom_namespaces.get(&nid) {
                return Ok(match custom {
                    Some(u) => JsValue::String(u.clone()),
                    None => JsValue::Null,
                });
            }
            let uri = st
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
    define_attr_prop(rt, &dom, html_element_proto, "httpEquiv", "http-equiv");
    define_attr_prop(rt, &dom, html_element_proto, "content", "content");

    let dg = dom.clone();
    let ds = dom.clone();
    let get_data_prop = rt.create_native_function(
        "get data",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::String(String::new()));
            };
            let val = {
                let st = dg.borrow();
                let raw = st
                    .doc
                    .element(nid)
                    .and_then(|e| e.get_attr("data"))
                    .unwrap_or("")
                    .to_string();
                if raw.is_empty() {
                    raw
                } else {
                    peedle_url::Url::parse(&st.url)
                        .ok()
                        .and_then(|b| b.join(&raw).ok())
                        .map(|u| u.href())
                        .unwrap_or(raw)
                }
            };
            Ok(JsValue::String(val))
        }),
    );
    let set_data_prop = rt.create_native_function(
        "set data",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let s = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mut st = ds.borrow_mut();
            st.doc.set_attribute(nid, "data", s);
            st.sub_documents.remove(&nid);
            if st.doc.element(nid).map_or(false, |e| e.is_html() && e.tag_is("object"))
                && st.doc.is_ancestor(st.doc.root, nid)
                && !st.pending_subdoc_loads.contains(&nid)
            {
                st.pending_subdoc_loads.push(nid);
            }
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, html_element_proto, "data", Some(get_data_prop), Some(set_data_prop));
    define_attr_prop(rt, &dom, html_element_proto, "href", "href");

    let dg_src = dom.clone();
    let ds_src = dom.clone();
    let get_src_prop = rt.create_native_function(
        "get src",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::String(String::new()));
            };
            let val = dg_src
                .borrow()
                .doc
                .element(nid)
                .and_then(|e| e.get_attr("src"))
                .unwrap_or("")
                .to_string();
            Ok(JsValue::String(val))
        }),
    );
    let set_src_prop = rt.create_native_function(
        "set src",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let s = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mut st = ds_src.borrow_mut();
            st.doc.set_attribute(nid, "src", s);
            if st.doc.element(nid).map_or(false, |e| e.is_html() && e.tag_is("iframe")) {
                st.sub_documents.remove(&nid);
                if st.doc.is_ancestor(st.doc.root, nid) && !st.pending_subdoc_loads.contains(&nid) {
                    st.pending_subdoc_loads.push(nid);
                }
            }
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, html_element_proto, "src", Some(get_src_prop), Some(set_src_prop));
    define_attr_prop(rt, &dom, html_element_proto, "alt", "alt");
    define_attr_prop(rt, &dom, html_element_proto, "placeholder", "placeholder");
    define_attr_prop(rt, &dom, html_element_proto, "rel", "rel");
    define_attr_prop(rt, &dom, html_element_proto, "action", "action");
    define_attr_prop(rt, &dom, html_element_proto, "method", "method");
    define_attr_prop(rt, &dom, html_element_proto, "role", "role");
    define_attr_prop(rt, &dom, html_element_proto, "htmlFor", "for");

    let dg = dom.clone();
    let ds = dom.clone();
    let get_type = rt.create_native_function(
        "get type",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::String(String::new()));
            };
            let val = dg
                .borrow()
                .doc
                .element(nid)
                .map(|e| {
                    let raw = e.get_attr("type").unwrap_or("").to_ascii_lowercase();
                    if e.tag_is("button") {
                        match raw.as_str() {
                            "button" | "reset" | "submit" => raw,
                            _ => "submit".to_string(),
                        }
                    } else if e.tag_is("input") {
                        if raw.is_empty() {
                            "text".to_string()
                        } else {
                            raw
                        }
                    } else {
                        e.get_attr("type").unwrap_or("").to_string()
                    }
                })
                .unwrap_or_default();
            Ok(JsValue::String(val))
        }),
    );
    let set_type = rt.create_native_function(
        "set type",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let s = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mut st = ds.borrow_mut();
            st.doc.set_attribute(nid, "type", s);
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, html_element_proto, "type", Some(get_type), Some(set_type));

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
            if b {
                uncheck_radio_siblings(&mut st.doc, nid);
            }
            if let Some(el) = st.doc.element_mut(nid) {
                el.checked = Some(b);
            }
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, html_element_proto, "checked", Some(get_chk), Some(set_chk));

    let dg_dc = dom.clone();
    let ds_dc = dom.clone();
    let get_dchk = rt.create_native_function(
        "get defaultChecked",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Boolean(false));
            };
            let c = dg_dc.borrow().doc.element(nid).map_or(false, |e| e.has_attr("checked"));
            Ok(JsValue::Boolean(c))
        }),
    );
    let set_dchk = rt.create_native_function(
        "set defaultChecked",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let b = args.first().map_or(false, |v| rt.to_boolean(v));
            let mut st = ds_dc.borrow_mut();
            if b {
                st.doc.set_attribute(nid, "checked", "");
            } else {
                st.doc.remove_attribute(nid, "checked");
            }
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, html_element_proto, "defaultChecked", Some(get_dchk), Some(set_dchk));

    let dg_ds = dom.clone();
    let ds_ds = dom.clone();
    let get_dsel = rt.create_native_function(
        "get defaultSelected",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Boolean(false));
            };
            let c = dg_ds.borrow().doc.element(nid).map_or(false, |e| e.has_attr("selected"));
            Ok(JsValue::Boolean(c))
        }),
    );
    let set_dsel = rt.create_native_function(
        "set defaultSelected",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let b = args.first().map_or(false, |v| rt.to_boolean(v));
            let mut st = ds_ds.borrow_mut();
            if b {
                st.doc.set_attribute(nid, "selected", "");
            } else {
                st.doc.remove_attribute(nid, "selected");
            }
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, html_element_proto, "defaultSelected", Some(get_dsel), Some(set_dsel));

    let dg_sel = dom.clone();
    let ds_sel = dom.clone();
    let get_sel = rt.create_native_function(
        "get selected",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Boolean(false));
            };
            let c = dg_sel
                .borrow()
                .doc
                .element(nid)
                .map_or(false, |e| e.checked.unwrap_or_else(|| e.has_attr("selected")));
            Ok(JsValue::Boolean(c))
        }),
    );
    let set_sel = rt.create_native_function(
        "set selected",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let b = args.first().map_or(false, |v| rt.to_boolean(v));
            let mut st = ds_sel.borrow_mut();
            if let Some(el) = st.doc.element_mut(nid) {
                el.checked = Some(b);
            }
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, html_element_proto, "selected", Some(get_sel), Some(set_sel));

    let dg_si = dom.clone();
    let ds_si = dom.clone();
    let get_sel_idx = rt.create_native_function(
        "get selectedIndex",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Number(-1.0));
            };
            let st = dg_si.borrow();
            let opts: Vec<NodeId> = st
                .doc
                .descendants(nid)
                .into_iter()
                .filter(|&id| st.doc.element(id).map_or(false, |e| e.tag_is("option")))
                .collect();
            if opts.is_empty() {
                return Ok(JsValue::Number(-1.0));
            }
            if let Some(explicit) = st.doc.element(nid).and_then(|e| e.selected_index) {
                if explicit < opts.len() {
                    return Ok(JsValue::Number(explicit as f64));
                }
            }
            let mut chosen = 0usize;
            for (i, &oid) in opts.iter().enumerate() {
                if let Some(oe) = st.doc.element(oid) {
                    if oe.checked.unwrap_or_else(|| oe.has_attr("selected")) {
                        chosen = i;
                    }
                }
            }
            Ok(JsValue::Number(chosen as f64))
        }),
    );
    let set_sel_idx = rt.create_native_function(
        "set selectedIndex",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let idx = args
                .first()
                .and_then(|v| rt.to_number(v).ok())
                .map(|n| n as isize)
                .unwrap_or(-1);
            let mut st = ds_si.borrow_mut();
            if let Some(el) = st.doc.element_mut(nid) {
                el.selected_index = if idx >= 0 { Some(idx as usize) } else { None };
            }
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, html_element_proto, "selectedIndex", Some(get_sel_idx), Some(set_sel_idx));

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
            let is_embed_url = st.doc.element(nid).map_or(false, |e| {
                e.is_html()
                    && ((e.tag_is("iframe") && name.eq_ignore_ascii_case("src"))
                        || (e.tag_is("object") && name.eq_ignore_ascii_case("data")))
            });
            st.doc.set_attribute(nid, name, val);
            if is_embed_url {
                st.sub_documents.remove(&nid);
                if st.doc.is_ancestor(st.doc.root, nid) && !st.pending_subdoc_loads.contains(&nid) {
                    st.pending_subdoc_loads.push(nid);
                }
            }
            st.mark_dirty_for(nid);
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
            st.mark_dirty_for(nid);
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
            st.mark_dirty_for(nid);
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
            let (kind, prev_states) = {
                let mut st = d.borrow_mut();
                let info = st.doc.element(nid).map(|e| {
                    let is_html = e.is_html();
                    let tag = e.tag_name.clone();
                    let ty = e.get_attr("type").unwrap_or("").to_ascii_lowercase();
                    let disabled = e.is_disabled();
                    let checked = e.checked;
                    let is_checked = e.is_checked();
                    (is_html, tag, ty, disabled, checked, is_checked)
                });
                match info {
                    Some((true, ref tag, ref ty, false, old_opt, is_chk))
                        if tag == "input" && ty == "checkbox" =>
                    {
                        if let Some(el) = st.doc.element_mut(nid) {
                            el.checked = Some(!is_chk);
                        }
                        st.mark_dirty();
                        (1u8, vec![(nid, old_opt)])
                    }
                    Some((true, ref tag, ref ty, false, _, _))
                        if tag == "input" && ty == "radio" =>
                    {
                        let group = collect_radio_group(&st.doc, nid);
                        let saved: Vec<(NodeId, Option<bool>)> = group
                            .iter()
                            .map(|&id| (id, st.doc.element(id).and_then(|e| e.checked)))
                            .collect();
                        uncheck_radio_siblings(&mut st.doc, nid);
                        if let Some(el) = st.doc.element_mut(nid) {
                            el.checked = Some(true);
                        }
                        st.mark_dirty();
                        (2u8, saved)
                    }
                    Some((true, ref tag, ref ty, false, _, _))
                        if (tag == "input" && ty == "submit")
                            || (tag == "button" && (ty.is_empty() || ty == "submit")) =>
                    {
                        (3u8, Vec::new())
                    }
                    _ => (0u8, Vec::new()),
                }
            };
            let ev_proto = d.borrow().protos.unwrap().event;
            let ev = build_event_object(rt, ev_proto, "click".to_string(), true, true, true);
            let allow_default = dispatch_event_object(rt, &d, Some(nid), ev)?;
            if !allow_default && (kind == 1 || kind == 2) {
                let mut st = d.borrow_mut();
                for (id, old) in prev_states {
                    if let Some(el) = st.doc.element_mut(id) {
                        el.checked = old;
                    }
                }
                st.mark_dirty();
            } else if allow_default && kind == 3 {
                let form_id = {
                    let st = d.borrow();
                    find_owner_form(&st.doc, nid)
                };
                if let Some(fid) = form_id {
                    let sub_ev = build_event_object(rt, ev_proto, "submit".to_string(), true, true, true);
                    let _ = dispatch_event_object(rt, &d, Some(fid), sub_ev)?;
                }
            }
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
            let old_kids = st.doc.children(nid);
            for k in old_kids {
                st.notify_pre_remove(k);
            }
            st.doc.set_text_content(nid, s);
            st.invalidate_style_owner(nid);
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, html_element_proto, "innerText", Some(get_it), Some(set_it));

    define_html_table_form_embed_apis(rt, dom, html_element_proto);
}

pub fn ensure_subdocument(dom: &SharedDom, host_id: NodeId) -> Option<NodeId> {
    if let Some(&existing) = dom.borrow().sub_documents.get(&host_id) {
        return Some(existing);
    }
    let _ = load_subdocument_with_scripts(dom, host_id);
    dom.borrow().sub_documents.get(&host_id).copied()
}

pub fn load_subdocument_with_scripts(dom: &SharedDom, host_id: NodeId) -> Vec<String> {
    let (tag, url_attr, base_url, client) = {
        let st = dom.borrow();
        let Some(el) = st.doc.element(host_id) else {
            return Vec::new();
        };
        let tag = el.tag_name.clone();
        let attr = if tag == "iframe" {
            el.get_attr("src").map(|s| s.to_string())
        } else if tag == "object" {
            el.get_attr("data").map(|s| s.to_string())
        } else {
            return Vec::new();
        };
        (tag, attr, st.url.clone(), st.client.clone())
    };

    enum SubdocPayload {
        Blank,
        PlainText(String),
        Html(String),
        Xml(Vec<u8>),
        Skip,
    }

    let payload = if let Some(ref raw) = url_attr {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed == "about:blank" {
            SubdocPayload::Blank
        } else {
            let resolved = peedle_url::Url::parse(&base_url)
                .ok()
                .and_then(|b| b.join(trimmed).ok())
                .or_else(|| peedle_url::Url::parse(trimmed).ok())
                .map(|u| u.href());
            if let Some(u) = resolved {
                match client.get(&u) {
                    Ok(resp) if resp.status < 400 => {
                        let ct = resp.content_type().to_ascii_lowercase();
                        let path_part = u.split('?').next().unwrap_or(&u).to_ascii_lowercase();
                        let is_img_bin = (ct.starts_with("image/") && !ct.contains("svg"))
                            || (ct.contains("octet-stream")
                                && (path_part.ends_with(".png")
                                    || path_part.ends_with(".gif")
                                    || path_part.ends_with(".jpg")
                                    || path_part.ends_with(".jpeg")
                                    || path_part.ends_with(".webp")));
                        let is_plain = ct.starts_with("text/plain")
                            || (ct.contains("octet-stream") && path_part.ends_with(".txt"));
                        let is_xml = ct.contains("xml")
                            || ct.contains("svg")
                            || path_part.ends_with(".xml")
                            || path_part.ends_with(".svg")
                            || path_part.ends_with("/xhtml.1")
                            || path_part.ends_with("/xhtml.2")
                            || path_part.ends_with("/xhtml.3");
                        if is_img_bin {
                            SubdocPayload::Skip
                        } else if is_plain {
                            SubdocPayload::PlainText(resp.text())
                        } else if is_xml {
                            SubdocPayload::Xml(resp.body)
                        } else {
                            SubdocPayload::Html(resp.text())
                        }
                    }
                    _ => SubdocPayload::Skip,
                }
            } else {
                SubdocPayload::Skip
            }
        }
    } else if tag == "iframe" {
        SubdocPayload::Blank
    } else {
        SubdocPayload::Skip
    };

    let mut st = dom.borrow_mut();
    match payload {
        SubdocPayload::Skip => {
            st.sub_documents.remove(&host_id);
            Vec::new()
        }
        SubdocPayload::Blank => {
            let sub = st.doc.create_sub_document();
            st.sub_documents.insert(host_id, sub);
            Vec::new()
        }
        SubdocPayload::PlainText(txt) => {
            let sub = st.doc.create_sub_document();
            if let Some(body) = st.doc.body_of(sub) {
                let pre = st.doc.create_element("pre");
                let tn = st.doc.create_text(txt);
                st.doc.append_child(pre, tn);
                st.doc.append_child(body, pre);
            }
            st.sub_documents.insert(host_id, sub);
            Vec::new()
        }
        SubdocPayload::Xml(bytes) => {
            let sub = st.doc.create_sub_document();
            for c in st.doc.children(sub) {
                st.doc.detach(c);
            }
            st.xml_documents.insert(sub);
            match crate::xml_parser::parse_xml_into(&mut st, &bytes, sub) {
                Ok(scripts) => {
                    st.sub_documents.insert(host_id, sub);
                    scripts
                }
                Err(()) => {
                    st.sub_documents.remove(&host_id);
                    Vec::new()
                }
            }
        }
        SubdocPayload::Html(content) => {
            let sub = st.doc.create_sub_document();
            if !content.trim().is_empty() {
                let parsed = peedle_html::parse_html(&content);
                for c in st.doc.children(sub) {
                    st.doc.detach(c);
                }
                for src_child in parsed.children(parsed.root) {
                    let imported = import_foreign_subtree(&mut st.doc, &parsed, src_child);
                    st.doc.append_child(sub, imported);
                }
            }
            st.sub_documents.insert(host_id, sub);
            Vec::new()
        }
    }
}

pub fn import_foreign_subtree(
    dst: &mut peedle_html::Document,
    src: &peedle_html::Document,
    src_id: NodeId,
) -> NodeId {
    let new_id = match &src.node(src_id).data {
        NodeData::Element(e) => {
            dst.create_element_ns(e.tag_name.clone(), e.namespace, e.attributes.clone())
        }
        NodeData::Text(s) => dst.create_text(s.clone()),
        NodeData::Comment(s) => dst.create_comment(s.clone()),
        NodeData::Doctype {
            name,
            public_id,
            system_id,
        } => dst.create_doctype(name.clone(), public_id.clone(), system_id.clone()),
        NodeData::ProcessingInstruction { target, data } => {
            dst.create_processing_instruction(target.clone(), data.clone())
        }
        _ => dst.create_fragment(),
    };
    for c in src.children(src_id) {
        let cid = import_foreign_subtree(dst, src, c);
        dst.append_child(new_id, cid);
    }
    new_id
}

fn define_html_table_form_embed_apis(
    rt: &mut JsRuntime,
    dom: SharedDom,
    html_element_proto: ObjectId,
) {
    let d = dom.clone();
    let get_cd = rt.create_native_function(
        "get contentDocument",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let sub = ensure_subdocument(&d, nid);
            Ok(wrap_node_opt(rt, &d, sub))
        }),
    );
    define_getter_setter(rt, html_element_proto, "contentDocument", Some(get_cd), None);
    let _ = rt.set_property(html_element_proto, "getSVGDocument", JsValue::Object(get_cd));

    let d = dom.clone();
    let get_row_index = rt.create_native_function(
        "get rowIndex",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Number(-1.0));
            };
            let st = d.borrow();
            let mut cur = st.doc.node(nid).parent;
            let mut table_id = None;
            while let Some(p) = cur {
                if st.doc.element(p).map_or(false, |e| e.tag_is("table")) {
                    table_id = Some(p);
                    break;
                }
                cur = st.doc.node(p).parent;
            }
            let idx = table_id
                .and_then(|tid| collect_table_rows(&st.doc, tid).iter().position(|&r| r == nid))
                .map(|i| i as f64)
                .unwrap_or(-1.0);
            Ok(JsValue::Number(idx))
        }),
    );
    define_getter_setter(rt, html_element_proto, "rowIndex", Some(get_row_index), None);

    let d = dom.clone();
    let get_sec_row_index = rt.create_native_function(
        "get sectionRowIndex",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Number(-1.0));
            };
            let st = d.borrow();
            let idx = st
                .doc
                .node(nid)
                .parent
                .and_then(|p| {
                    st.doc
                        .element_children(p)
                        .into_iter()
                        .filter(|&c| st.doc.element(c).map_or(false, |e| e.tag_is("tr")))
                        .position(|r| r == nid)
                })
                .map(|i| i as f64)
                .unwrap_or(-1.0);
            Ok(JsValue::Number(idx))
        }),
    );
    define_getter_setter(
        rt,
        html_element_proto,
        "sectionRowIndex",
        Some(get_sec_row_index),
        None,
    );

    let svg_proto = dom.borrow().protos.unwrap().svg_element;
    for attr in ["x", "y", "width", "height", "rx", "ry"] {
        let d = dom.clone();
        let gf = rt.create_native_function(
            attr,
            0,
            Rc::new(move |rt, this, _| {
                let Some(nid) = node_id_of(rt, &this) else {
                    return Ok(JsValue::Undefined);
                };
                let v: f64 = d
                    .borrow()
                    .doc
                    .element(nid)
                    .and_then(|e| e.get_attr(attr))
                    .and_then(|s| s.trim_end_matches("px").parse::<f64>().ok())
                    .unwrap_or(0.0);
                let anim = rt.create_object();
                let bv = rt.create_object();
                let _ = rt.set_property(bv, "value", JsValue::Number(v));
                let av = rt.create_object();
                let _ = rt.set_property(av, "value", JsValue::Number(v));
                let _ = rt.set_property(anim, "baseVal", JsValue::Object(bv));
                let _ = rt.set_property(anim, "animVal", JsValue::Object(av));
                Ok(JsValue::Object(anim))
            }),
        );
        define_getter_setter(rt, svg_proto, attr, Some(gf), None);
    }

    let d = dom.clone();
    let get_num_chars = rt.create_native_function(
        "getNumberOfChars",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Number(0.0));
            };
            let n = d.borrow().doc.text_content(nid).encode_utf16().count();
            Ok(JsValue::Number(n as f64))
        }),
    );
    let _ = rt.set_property(svg_proto, "getNumberOfChars", JsValue::Object(get_num_chars));

    let get_cw = rt.create_native_function(
        "get contentWindow",
        0,
        Rc::new(|rt, _, _| Ok(JsValue::Object(rt.global_object()))),
    );
    define_getter_setter(rt, html_element_proto, "contentWindow", Some(get_cw), None);

    let d = dom.clone();
    let get_tbodies = rt.create_native_function(
        "get tBodies",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Object(rt.create_array(Vec::new())));
            };
            let list: Vec<NodeId> = {
                let st = d.borrow();
                st.doc
                    .element_children(nid)
                    .into_iter()
                    .filter(|&c| st.doc.element(c).map_or(false, |e| e.tag_is("tbody")))
                    .collect()
            };
            Ok(JsValue::Object(wrap_node_list(rt, &d, list)))
        }),
    );
    define_getter_setter(rt, html_element_proto, "tBodies", Some(get_tbodies), None);

    for (prop, tag) in [("tHead", "thead"), ("tFoot", "tfoot"), ("caption", "caption")] {
        let dg = dom.clone();
        let gf = rt.create_native_function(
            prop,
            0,
            Rc::new(move |rt, this, _| {
                let Some(nid) = node_id_of(rt, &this) else {
                    return Ok(JsValue::Null);
                };
                let hit = {
                    let st = dg.borrow();
                    st.doc
                        .element_children(nid)
                        .into_iter()
                        .find(|&c| st.doc.element(c).map_or(false, |e| e.tag_is(tag)))
                };
                Ok(wrap_node_opt(rt, &dg, hit))
            }),
        );
        define_getter_setter(rt, html_element_proto, prop, Some(gf), None);
    }

    for (mname, tag) in [
        ("createTHead", "thead"),
        ("createTFoot", "tfoot"),
        ("createCaption", "caption"),
    ] {
        let d = dom.clone();
        let cf = rt.create_native_function(
            mname,
            0,
            Rc::new(move |rt, this, _| {
                let Some(nid) = node_id_of(rt, &this) else {
                    return Ok(JsValue::Null);
                };
                let res = {
                    let mut st = d.borrow_mut();
                    if let Some(existing) = st
                        .doc
                        .element_children(nid)
                        .into_iter()
                        .find(|&c| st.doc.element(c).map_or(false, |e| e.tag_is(tag)))
                    {
                        existing
                    } else {
                        let created = st.doc.create_element(tag);
                        st.doc.append_child(nid, created);
                        st.mark_dirty();
                        created
                    }
                };
                Ok(JsValue::Object(wrap_node(rt, &d, res)))
            }),
        );
        let _ = rt.set_property(html_element_proto, mname, JsValue::Object(cf));
    }

    let d = dom.clone();
    let create_tbody = rt.create_native_function(
        "createTBody",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let tb = {
                let mut st = d.borrow_mut();
                let tb = st.doc.create_element("tbody");
                st.doc.append_child(nid, tb);
                st.mark_dirty();
                tb
            };
            Ok(JsValue::Object(wrap_node(rt, &d, tb)))
        }),
    );
    let _ = rt.set_property(html_element_proto, "createTBody", JsValue::Object(create_tbody));

    for (mname, tag) in [
        ("deleteTHead", "thead"),
        ("deleteTFoot", "tfoot"),
        ("deleteCaption", "caption"),
    ] {
        let d = dom.clone();
        let df = rt.create_native_function(
            mname,
            0,
            Rc::new(move |rt, this, _| {
                let Some(nid) = node_id_of(rt, &this) else {
                    return Ok(JsValue::Undefined);
                };
                let mut st = d.borrow_mut();
                if let Some(existing) = st
                    .doc
                    .element_children(nid)
                    .into_iter()
                    .find(|&c| st.doc.element(c).map_or(false, |e| e.tag_is(tag)))
                {
                    st.doc.detach(existing);
                    st.mark_dirty();
                }
                Ok(JsValue::Undefined)
            }),
        );
        let _ = rt.set_property(html_element_proto, mname, JsValue::Object(df));
    }

    let d = dom.clone();
    let get_rows = rt.create_native_function(
        "get rows",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Object(rt.create_array(Vec::new())));
            };
            let rows = collect_table_rows(&d.borrow().doc, nid);
            Ok(JsValue::Object(wrap_node_list(rt, &d, rows)))
        }),
    );
    define_getter_setter(rt, html_element_proto, "rows", Some(get_rows), None);

    let d = dom.clone();
    let insert_row = rt.create_native_function(
        "insertRow",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let idx = args
                .first()
                .filter(|v| !v.is_undefined())
                .and_then(|v| rt.to_number(v).ok())
                .map(|n| n as isize)
                .unwrap_or(-1);
            let tr = {
                let mut st = d.borrow_mut();
                let rows = collect_table_rows(&st.doc, nid);
                let is_table = st.doc.element(nid).map_or(false, |e| e.tag_is("table"));
                if idx < -1 || idx as usize > rows.len() {
                    drop(st);
                    return Err(throw_dom_exception(rt, &d, 1, "IndexSizeError", "row index out of bounds"));
                }
                let tr = st.doc.create_element("tr");
                if idx == -1 || idx as usize == rows.len() {
                    let parent = if is_table {
                        st.doc
                            .element_children(nid)
                            .into_iter()
                            .rfind(|&c| st.doc.element(c).map_or(false, |e| e.tag_is("tbody")))
                            .unwrap_or_else(|| {
                                let tb = st.doc.create_element("tbody");
                                st.doc.append_child(nid, tb);
                                tb
                            })
                    } else {
                        nid
                    };
                    st.doc.append_child(parent, tr);
                } else {
                    let ref_tr = rows[idx as usize];
                    let parent = st.doc.node(ref_tr).parent.unwrap_or(nid);
                    st.doc.insert_before(parent, tr, Some(ref_tr));
                }
                st.mark_dirty();
                tr
            };
            Ok(JsValue::Object(wrap_node(rt, &d, tr)))
        }),
    );
    let _ = rt.set_property(html_element_proto, "insertRow", JsValue::Object(insert_row));

    let d = dom.clone();
    let delete_row = rt.create_native_function(
        "deleteRow",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let idx = args
                .first()
                .and_then(|v| rt.to_number(v).ok())
                .map(|n| n as isize)
                .unwrap_or(-1);
            let rows = collect_table_rows(&d.borrow().doc, nid);
            let target = if idx == -1 {
                rows.last().copied()
            } else if idx >= 0 && (idx as usize) < rows.len() {
                Some(rows[idx as usize])
            } else {
                return Err(throw_dom_exception(rt, &d, 1, "IndexSizeError", "row index out of bounds"));
            };
            if let Some(tr) = target {
                let mut st = d.borrow_mut();
                st.doc.detach(tr);
                st.mark_dirty();
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(html_element_proto, "deleteRow", JsValue::Object(delete_row));

    let d = dom.clone();
    let get_cells = rt.create_native_function(
        "get cells",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Object(rt.create_array(Vec::new())));
            };
            let cells: Vec<NodeId> = {
                let st = d.borrow();
                st.doc
                    .element_children(nid)
                    .into_iter()
                    .filter(|&c| {
                        st.doc
                            .element(c)
                            .map_or(false, |e| e.tag_is("td") || e.tag_is("th"))
                    })
                    .collect()
            };
            Ok(JsValue::Object(wrap_node_list(rt, &d, cells)))
        }),
    );
    define_getter_setter(rt, html_element_proto, "cells", Some(get_cells), None);

    let d = dom.clone();
    let insert_cell = rt.create_native_function(
        "insertCell",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let idx = args
                .first()
                .filter(|v| !v.is_undefined())
                .and_then(|v| rt.to_number(v).ok())
                .map(|n| n as isize)
                .unwrap_or(-1);
            let td = {
                let mut st = d.borrow_mut();
                let cells: Vec<NodeId> = st
                    .doc
                    .element_children(nid)
                    .into_iter()
                    .filter(|&c| {
                        st.doc
                            .element(c)
                            .map_or(false, |e| e.tag_is("td") || e.tag_is("th"))
                    })
                    .collect();
                if idx < -1 || idx as usize > cells.len() {
                    drop(st);
                    return Err(throw_dom_exception(rt, &d, 1, "IndexSizeError", "cell index out of bounds"));
                }
                let td = st.doc.create_element("td");
                if idx == -1 || idx as usize == cells.len() {
                    st.doc.append_child(nid, td);
                } else {
                    st.doc.insert_before(nid, td, Some(cells[idx as usize]));
                }
                st.mark_dirty();
                td
            };
            Ok(JsValue::Object(wrap_node(rt, &d, td)))
        }),
    );
    let _ = rt.set_property(html_element_proto, "insertCell", JsValue::Object(insert_cell));

    let d = dom.clone();
    let delete_cell = rt.create_native_function(
        "deleteCell",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let idx = args
                .first()
                .and_then(|v| rt.to_number(v).ok())
                .map(|n| n as isize)
                .unwrap_or(-1);
            let cells: Vec<NodeId> = {
                let st = d.borrow();
                st.doc
                    .element_children(nid)
                    .into_iter()
                    .filter(|&c| {
                        st.doc
                            .element(c)
                            .map_or(false, |e| e.tag_is("td") || e.tag_is("th"))
                    })
                    .collect()
            };
            let target = if idx == -1 {
                cells.last().copied()
            } else if idx >= 0 && (idx as usize) < cells.len() {
                Some(cells[idx as usize])
            } else {
                return Err(throw_dom_exception(rt, &d, 1, "IndexSizeError", "cell index out of bounds"));
            };
            if let Some(td) = target {
                let mut st = d.borrow_mut();
                st.doc.detach(td);
                st.mark_dirty();
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(html_element_proto, "deleteCell", JsValue::Object(delete_cell));

    let d = dom.clone();
    let get_elements = rt.create_native_function(
        "get elements",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Object(rt.create_array(Vec::new())));
            };
            let controls: Vec<(NodeId, Option<String>, Option<String>)> = {
                let st = d.borrow();
                st.doc
                    .descendants(nid)
                    .into_iter()
                    .filter_map(|id| {
                        let e = st.doc.element(id)?;
                        if e.is_html()
                            && matches!(
                                e.tag_name.as_str(),
                                "input" | "button" | "select" | "textarea" | "fieldset" | "output" | "object"
                            )
                            && !(e.tag_name == "input"
                                && e.get_attr("type")
                                    .map_or(false, |t| t.eq_ignore_ascii_case("image")))
                        {
                            Some((
                                id,
                                e.get_attr("name").map(|s| s.to_string()),
                                e.get_attr("id").map(|s| s.to_string()),
                            ))
                        } else {
                            None
                        }
                    })
                    .collect()
            };
            let ids: Vec<NodeId> = controls.iter().map(|(id, _, _)| *id).collect();
            let list = wrap_node_list(rt, &d, ids);
            let mut by_name: std::collections::HashMap<String, Vec<NodeId>> =
                std::collections::HashMap::new();
            for (id, name_opt, id_opt) in controls {
                if let Some(n) = name_opt {
                    if !n.is_empty() {
                        by_name.entry(n).or_default().push(id);
                    }
                }
                if let Some(i) = id_opt {
                    if !i.is_empty() {
                        let entry = by_name.entry(i).or_default();
                        if !entry.contains(&id) {
                            entry.push(id);
                        }
                    }
                }
            }
            for (k, v) in by_name {
                if v.len() == 1 {
                    let obj = wrap_node(rt, &d, v[0]);
                    let _ = rt.set_property(list, &k, JsValue::Object(obj));
                } else {
                    let sub = wrap_node_list(rt, &d, v);
                    let _ = rt.set_property(list, &k, JsValue::Object(sub));
                }
            }
            Ok(JsValue::Object(list))
        }),
    );
    define_getter_setter(rt, html_element_proto, "elements", Some(get_elements), None);

    let d = dom.clone();
    let get_options = rt.create_native_function(
        "get options",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Object(rt.create_array(Vec::new())));
            };
            let opts: Vec<NodeId> = {
                let st = d.borrow();
                st.doc
                    .descendants(nid)
                    .into_iter()
                    .filter(|&id| st.doc.element(id).map_or(false, |e| e.tag_is("option")))
                    .collect()
            };
            Ok(JsValue::Object(wrap_node_list(rt, &d, opts)))
        }),
    );
    define_getter_setter(rt, html_element_proto, "options", Some(get_options), None);

    let d = dom.clone();
    let select_add = rt.create_native_function(
        "add",
        2,
        Rc::new(move |rt, this, args| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let Some(opt) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Ok(JsValue::Undefined);
            };
            let before = args.get(1).and_then(|v| node_id_of(rt, v));
            let mut st = d.borrow_mut();
            st.doc.insert_before(nid, opt, before);
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(html_element_proto, "add", JsValue::Object(select_add));

    let d = dom.clone();
    let get_form_or_select_len = rt.create_native_function(
        "get length",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = node_id_of(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let st = d.borrow();
            let Some(el) = st.doc.element(nid) else {
                return Ok(JsValue::Undefined);
            };
            if el.is_html() && el.tag_is("form") {
                let count = st
                    .doc
                    .descendants(nid)
                    .into_iter()
                    .filter(|&id| {
                        st.doc.element(id).map_or(false, |e| {
                            e.is_html()
                                && matches!(
                                    e.tag_name.as_str(),
                                    "input" | "button" | "select" | "textarea" | "fieldset" | "output" | "object"
                                )
                                && !(e.tag_name == "input"
                                    && e.get_attr("type")
                                        .map_or(false, |t| t.eq_ignore_ascii_case("image")))
                        })
                    })
                    .count();
                return Ok(JsValue::Number(count as f64));
            }
            if el.is_html() && el.tag_is("select") {
                let count = st
                    .doc
                    .descendants(nid)
                    .into_iter()
                    .filter(|&id| st.doc.element(id).map_or(false, |e| e.tag_is("option")))
                    .count();
                return Ok(JsValue::Number(count as f64));
            }
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, html_element_proto, "length", Some(get_form_or_select_len), None);

    for (dim_prop, is_width) in [("width", true), ("height", false)] {
        let dg = dom.clone();
        let ds = dom.clone();
        let getter = rt.create_native_function(
            dim_prop,
            0,
            Rc::new(move |rt, this, _| {
                let Some(nid) = node_id_of(rt, &this) else {
                    return Ok(JsValue::Undefined);
                };
                let mut st = dg.borrow_mut();
                let Some(tag) = st.doc.element(nid).map(|e| e.tag_name.clone()) else {
                    return Ok(JsValue::Undefined);
                };
                if tag == "img" {
                    let mut root = nid;
                    while let Some(p) = st.doc.node(root).parent {
                        root = p;
                    }
                    let (style, vp) = if root == st.doc.root {
                        st.ensure_layout();
                        let vp = peedle_css::Viewport {
                            width: st.viewport_w,
                            height: st.viewport_h,
                            dark_mode: st.dark_mode,
                        };
                        (st.styles.get(nid).cloned().unwrap_or_default(), vp)
                    } else {
                        let sheets = st.collect_stylesheets_for_root(root);
                        let vp = st.resolve_iframe_viewport(root);
                        let smap = peedle_css::compute_styles_for_root(&st.doc, root, &sheets, vp);
                        (smap.get(nid).cloned().unwrap_or_default(), vp)
                    };
                    let ctx = CalcContext {
                        font_size: style.font_size,
                        root_font_size: 16.0,
                        viewport_width: vp.width,
                        viewport_height: vp.height,
                        percentage_base: if is_width { vp.width } else { vp.height },
                    };
                    let css_val = if is_width {
                        style.width.resolve(&ctx)
                    } else {
                        style.height.resolve(&ctx)
                    };
                    if let Some(px) = css_val {
                        return Ok(JsValue::Number(px.round() as f64));
                    }
                    let attr_v = st
                        .doc
                        .element(nid)
                        .and_then(|e| e.get_attr(dim_prop))
                        .and_then(|s| s.trim_end_matches("px").parse::<f64>().ok());
                    if let Some(av) = attr_v {
                        return Ok(JsValue::Number(av));
                    }
                    if let Some(src) = st.doc.element(nid).and_then(|e| e.get_attr("src")).map(|s| s.to_string()) {
                        if let Some(&(iw, ih)) = st.image_sizes.get(&src) {
                            return Ok(JsValue::Number(if is_width { iw as f64 } else { ih as f64 }));
                        }
                    }
                    return Ok(JsValue::Number(0.0));
                }
                if tag == "canvas" {
                    let def = if is_width { 300.0 } else { 150.0 };
                    let v = st
                        .doc
                        .element(nid)
                        .and_then(|e| e.get_attr(dim_prop))
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(def);
                    return Ok(JsValue::Number(v));
                }
                if matches!(tag.as_str(), "iframe" | "object" | "embed" | "video") {
                    let s = st
                        .doc
                        .element(nid)
                        .and_then(|e| e.get_attr(dim_prop))
                        .unwrap_or("")
                        .to_string();
                    return Ok(JsValue::String(s));
                }
                Ok(JsValue::Undefined)
            }),
        );
        let setter = rt.create_native_function(
            dim_prop,
            1,
            Rc::new(move |rt, this, args| {
                let Some(nid) = node_id_of(rt, &this) else {
                    return Ok(JsValue::Undefined);
                };
                let s = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
                let mut st = ds.borrow_mut();
                st.doc.set_attribute(nid, dim_prop, s);
                st.mark_dirty();
                Ok(JsValue::Undefined)
            }),
        );
        define_getter_setter(rt, html_element_proto, dim_prop, Some(getter), Some(setter));
    }
}

fn find_owner_form(doc: &peedle_html::Document, nid: NodeId) -> Option<NodeId> {
    let mut cur = doc.node(nid).parent;
    while let Some(p) = cur {
        if doc.element(p).map_or(false, |e| e.is_html() && e.tag_is("form")) {
            return Some(p);
        }
        cur = doc.node(p).parent;
    }
    None
}

fn collect_radio_group(doc: &peedle_html::Document, nid: NodeId) -> Vec<NodeId> {
    let Some(el) = doc.element(nid) else {
        return Vec::new();
    };
    if !el.is_html() || !el.tag_is("input") {
        return Vec::new();
    }
    if !el.get_attr("type").map_or(false, |t| t.eq_ignore_ascii_case("radio")) {
        return Vec::new();
    }
    let Some(name) = el.get_attr("name").filter(|n| !n.is_empty()) else {
        return vec![nid];
    };
    let form_owner = find_owner_form(doc, nid);
    let search_root = if let Some(fid) = form_owner {
        fid
    } else {
        let mut r = nid;
        while let Some(p) = doc.node(r).parent {
            r = p;
        }
        r
    };
    doc.descendants(search_root)
        .into_iter()
        .filter(|&id| {
            let Some(other) = doc.element(id) else {
                return false;
            };
            if !other.is_html() || !other.tag_is("input") {
                return false;
            }
            if !other.get_attr("type").map_or(false, |t| t.eq_ignore_ascii_case("radio")) {
                return false;
            }
            if other.get_attr("name") != Some(name) {
                return false;
            }
            find_owner_form(doc, id) == form_owner
        })
        .collect()
}

fn uncheck_radio_siblings(doc: &mut peedle_html::Document, nid: NodeId) {
    let group = collect_radio_group(doc, nid);
    for other_id in group {
        if other_id != nid {
            if let Some(other_el) = doc.element_mut(other_id) {
                other_el.checked = Some(false);
            }
        }
    }
}

fn collect_table_rows(doc: &peedle_html::Document, container: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let is_table = doc.element(container).map_or(false, |e| e.tag_is("table"));
    if !is_table {
        for c in doc.element_children(container) {
            if doc.element(c).map_or(false, |e| e.tag_is("tr")) {
                out.push(c);
            }
        }
        return out;
    }
    let mut head_rows = Vec::new();
    let mut body_rows = Vec::new();
    let mut foot_rows = Vec::new();
    for c in doc.element_children(container) {
        let Some(el) = doc.element(c) else {
            continue;
        };
        match el.tag_name.as_str() {
            "thead" => {
                for r in doc.element_children(c) {
                    if doc.element(r).map_or(false, |e| e.tag_is("tr")) {
                        head_rows.push(r);
                    }
                }
            }
            "tbody" => {
                for r in doc.element_children(c) {
                    if doc.element(r).map_or(false, |e| e.tag_is("tr")) {
                        body_rows.push(r);
                    }
                }
            }
            "tfoot" => {
                for r in doc.element_children(c) {
                    if doc.element(r).map_or(false, |e| e.tag_is("tr")) {
                        foot_rows.push(r);
                    }
                }
            }
            "tr" => body_rows.push(c),
            _ => {}
        }
    }
    out.extend(head_rows);
    out.extend(body_rows);
    out.extend(foot_rows);
    out
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
            st.mark_dirty_for(nid);
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
