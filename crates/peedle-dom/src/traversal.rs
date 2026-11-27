use std::cell::RefCell;
use std::rc::Rc;

use peedle_html::{Document, Namespace, NodeData, NodeId};
use peedle_js::{JsError, JsRuntime, JsValue, ObjectId};

use crate::events::define_getter_setter;
use crate::nodes::{node_id_of, wrap_node, wrap_node_opt};
use crate::state::{DomState, NodeIteratorHandle, RangeData, RangeHandle, SharedDom};

pub const DOM_EXCEPTION_CONSTANTS: &[(&str, u16)] = &[
    ("INDEX_SIZE_ERR", 1),
    ("DOMSTRING_SIZE_ERR", 2),
    ("HIERARCHY_REQUEST_ERR", 3),
    ("WRONG_DOCUMENT_ERR", 4),
    ("INVALID_CHARACTER_ERR", 5),
    ("NO_DATA_ALLOWED_ERR", 6),
    ("NO_MODIFICATION_ALLOWED_ERR", 7),
    ("NOT_FOUND_ERR", 8),
    ("NOT_SUPPORTED_ERR", 9),
    ("INUSE_ATTRIBUTE_ERR", 10),
    ("INVALID_STATE_ERR", 11),
    ("SYNTAX_ERR", 12),
    ("INVALID_MODIFICATION_ERR", 13),
    ("NAMESPACE_ERR", 14),
    ("INVALID_ACCESS_ERR", 15),
    ("VALIDATION_ERR", 16),
    ("TYPE_MISMATCH_ERR", 17),
    ("SECURITY_ERR", 18),
    ("NETWORK_ERR", 19),
    ("ABORT_ERR", 20),
    ("URL_MISMATCH_ERR", 21),
    ("QUOTA_EXCEEDED_ERR", 22),
    ("TIMEOUT_ERR", 23),
    ("INVALID_NODE_TYPE_ERR", 24),
    ("DATA_CLONE_ERR", 25),
];

pub fn throw_dom_exception(
    rt: &mut JsRuntime,
    dom: &SharedDom,
    code: u16,
    name: &str,
    message: &str,
) -> JsError {
    let proto = dom.borrow().protos.unwrap().dom_exception_proto;
    let obj = rt.create_object_with_proto(Some(proto));
    let _ = rt.set_property(obj, "code", JsValue::Number(code as f64));
    let _ = rt.set_property(obj, "name", JsValue::String(name.to_string()));
    let _ = rt.set_property(obj, "message", JsValue::String(message.to_string()));
    JsError::thrown(JsValue::Object(obj))
}

pub fn is_valid_xml_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let start_ok = first.is_ascii_alphabetic()
        || first == '_'
        || first == ':'
        || (first as u32 >= 0xC0 && first != '\u{D7}' && first != '\u{F7}');
    if !start_ok {
        return false;
    }
    for c in chars {
        let ok = c.is_ascii_alphanumeric()
            || matches!(c, '_' | ':' | '-' | '.')
            || (c as u32 >= 0xB7 && c != '\u{D7}' && c != '\u{F7}');
        if !ok {
            return false;
        }
    }
    true
}

pub fn is_valid_ncname(name: &str) -> bool {
    if name.is_empty() || name.contains(':') {
        return false;
    }
    is_valid_xml_name(name)
}

pub fn define_dom_exception(rt: &mut JsRuntime, dom_exception_proto: ObjectId) {
    let err_proto = rt.intrinsics.error_proto;
    let _ = rt.set_prototype_of(dom_exception_proto, Some(err_proto));
    for &(name, code) in DOM_EXCEPTION_CONSTANTS {
        let _ = rt.set_property(dom_exception_proto, name, JsValue::Number(code as f64));
    }
    let ctor = rt.create_native_constructor(
        "DOMException",
        0,
        dom_exception_proto,
        Rc::new(move |rt, _, args| {
            let msg = args
                .first()
                .filter(|v| !v.is_undefined())
                .map(|v| rt.to_string(v))
                .transpose()?
                .unwrap_or_default();
            let name = args
                .get(1)
                .filter(|v| !v.is_undefined())
                .map(|v| rt.to_string(v))
                .transpose()?
                .unwrap_or_else(|| "Error".to_string());
            let obj = rt.create_object_with_proto(Some(dom_exception_proto));
            let _ = rt.set_property(obj, "name", JsValue::String(name));
            let _ = rt.set_property(obj, "message", JsValue::String(msg));
            let _ = rt.set_property(obj, "code", JsValue::Number(0.0));
            Ok(JsValue::Object(obj))
        }),
    );
    for &(name, code) in DOM_EXCEPTION_CONSTANTS {
        let _ = rt.set_property(ctor, name, JsValue::Number(code as f64));
    }
    let global = rt.global_object();
    let _ = rt.set_property(global, "DOMException", JsValue::Object(ctor));
}

pub struct TreeWalkerData {
    pub root: NodeId,
    pub what_to_show: u32,
    pub filter: JsValue,
    pub current: NodeId,
}

fn node_show_mask(doc: &Document, nid: NodeId) -> u32 {
    match doc.node(nid).data {
        NodeData::Element(_) => 0x1,
        NodeData::Text(_) => 0x4,
        NodeData::ProcessingInstruction { .. } => 0x40,
        NodeData::Comment(_) => 0x80,
        NodeData::Document { .. } => 0x100,
        NodeData::Doctype { .. } => 0x200,
        NodeData::DocumentFragment | NodeData::ShadowRoot { .. } => 0x400,
    }
}

fn apply_node_filter(
    rt: &mut JsRuntime,
    dom: &SharedDom,
    what_to_show: u32,
    filter: &JsValue,
    nid: NodeId,
) -> Result<u8, JsError> {
    let mask = node_show_mask(&dom.borrow().doc, nid);
    if (what_to_show & mask) == 0 {
        return Ok(3);
    }
    if filter.is_nullish() {
        return Ok(1);
    }
    let wrapped = JsValue::Object(wrap_node(rt, dom, nid));
    let res = if rt.is_callable(filter) {
        rt.call_function(filter.clone(), JsValue::Undefined, &[wrapped])?
    } else if let Some(f_oid) = filter.as_object() {
        let accept = rt.get_property(f_oid, "acceptNode")?;
        if rt.is_callable(&accept) {
            rt.call_function(accept, filter.clone(), &[wrapped])?
        } else {
            return Ok(1);
        }
    } else {
        return Ok(1);
    };
    let n = rt.to_number(&res)? as u8;
    Ok(n)
}

fn collect_subtree_preorder(doc: &Document, root: NodeId, out: &mut Vec<NodeId>) {
    out.push(root);
    for c in doc.children(root) {
        collect_subtree_preorder(doc, c, out);
    }
}

fn next_in_preorder(doc: &Document, root: NodeId, node: NodeId) -> Option<NodeId> {
    if let Some(fc) = doc.node(node).first_child {
        return Some(fc);
    }
    let mut cur = node;
    loop {
        if cur == root {
            return None;
        }
        if let Some(ns) = doc.node(cur).next_sibling {
            return Some(ns);
        }
        cur = doc.node(cur).parent?;
    }
}

fn prev_in_preorder(doc: &Document, root: NodeId, node: NodeId) -> Option<NodeId> {
    if node == root {
        return None;
    }
    if let Some(mut ps) = doc.node(node).prev_sibling {
        while let Some(lc) = doc.node(ps).last_child {
            ps = lc;
        }
        return Some(ps);
    }
    doc.node(node).parent
}

pub fn define_traversal_protos(
    rt: &mut JsRuntime,
    dom: SharedDom,
    iter_proto: ObjectId,
    walker_proto: ObjectId,
) {
    let d = dom.clone();
    let get_root = rt.create_native_function(
        "get root",
        0,
        Rc::new(move |rt, this, _| {
            let Some(oid) = this.as_object() else {
                return Ok(JsValue::Null);
            };
            let rid = rt
                .get_host_data::<NodeIteratorHandle>(oid)
                .map(|x| x.0.borrow().root)
                .or_else(|| rt.get_host_data::<TreeWalkerData>(oid).map(|x| x.root));
            Ok(wrap_node_opt(rt, &d, rid))
        }),
    );
    define_getter_setter(rt, iter_proto, "root", Some(get_root), None);
    define_getter_setter(rt, walker_proto, "root", Some(get_root), None);

    let get_wts = rt.create_native_function(
        "get whatToShow",
        0,
        Rc::new(|rt, this, _| {
            let Some(oid) = this.as_object() else {
                return Ok(JsValue::Number(0.0));
            };
            let wts = rt
                .get_host_data::<NodeIteratorHandle>(oid)
                .map(|x| x.0.borrow().what_to_show)
                .or_else(|| rt.get_host_data::<TreeWalkerData>(oid).map(|x| x.what_to_show))
                .unwrap_or(0);
            Ok(JsValue::Number(wts as f64))
        }),
    );
    define_getter_setter(rt, iter_proto, "whatToShow", Some(get_wts), None);
    define_getter_setter(rt, walker_proto, "whatToShow", Some(get_wts), None);

    let get_flt = rt.create_native_function(
        "get filter",
        0,
        Rc::new(|rt, this, _| {
            let Some(oid) = this.as_object() else {
                return Ok(JsValue::Null);
            };
            let f = rt
                .get_host_data::<NodeIteratorHandle>(oid)
                .map(|x| x.0.borrow().filter.clone())
                .or_else(|| rt.get_host_data::<TreeWalkerData>(oid).map(|x| x.filter.clone()))
                .unwrap_or(JsValue::Null);
            Ok(f)
        }),
    );
    define_getter_setter(rt, iter_proto, "filter", Some(get_flt), None);
    define_getter_setter(rt, walker_proto, "filter", Some(get_flt), None);

    let d = dom.clone();
    let get_ref = rt.create_native_function(
        "get referenceNode",
        0,
        Rc::new(move |rt, this, _| {
            let rid = this
                .as_object()
                .and_then(|o| rt.get_host_data::<NodeIteratorHandle>(o))
                .map(|x| x.0.borrow().reference);
            Ok(wrap_node_opt(rt, &d, rid))
        }),
    );
    define_getter_setter(rt, iter_proto, "referenceNode", Some(get_ref), None);

    let get_pbr = rt.create_native_function(
        "get pointerBeforeReferenceNode",
        0,
        Rc::new(|rt, this, _| {
            let b = this
                .as_object()
                .and_then(|o| rt.get_host_data::<NodeIteratorHandle>(o))
                .map_or(true, |x| x.0.borrow().pointer_before);
            Ok(JsValue::Boolean(b))
        }),
    );
    define_getter_setter(rt, iter_proto, "pointerBeforeReferenceNode", Some(get_pbr), None);

    let d = dom.clone();
    let next_node = rt.create_native_function(
        "nextNode",
        0,
        Rc::new(move |rt, this, _| {
            let Some(oid) = this.as_object() else {
                return Ok(JsValue::Null);
            };
            let Some(handle) = rt.get_host_data::<NodeIteratorHandle>(oid).map(|h| h.0.clone()) else {
                return Ok(JsValue::Null);
            };
            loop {
                let (root, wts, filter, cur, before) = {
                    let it = handle.borrow();
                    if it.detached {
                        return Ok(JsValue::Null);
                    }
                    (it.root, it.what_to_show, it.filter.clone(), it.reference, it.pointer_before)
                };
                let (next_cur, next_before) = if before {
                    (cur, false)
                } else {
                    let Some(n) = next_in_preorder(&d.borrow().doc, root, cur) else {
                        return Ok(JsValue::Null);
                    };
                    (n, false)
                };
                {
                    let mut it = handle.borrow_mut();
                    it.reference = next_cur;
                    it.pointer_before = next_before;
                }
                match apply_node_filter(rt, &d, wts, &filter, next_cur) {
                    Ok(1) => return Ok(JsValue::Object(wrap_node(rt, &d, next_cur))),
                    Ok(_) => continue,
                    Err(e) => {
                        let mut it = handle.borrow_mut();
                        it.reference = cur;
                        it.pointer_before = before;
                        return Err(e);
                    }
                }
            }
        }),
    );
    let _ = rt.set_property(iter_proto, "nextNode", JsValue::Object(next_node));

    let d = dom.clone();
    let prev_node = rt.create_native_function(
        "previousNode",
        0,
        Rc::new(move |rt, this, _| {
            let Some(oid) = this.as_object() else {
                return Ok(JsValue::Null);
            };
            let Some(handle) = rt.get_host_data::<NodeIteratorHandle>(oid).map(|h| h.0.clone()) else {
                return Ok(JsValue::Null);
            };
            loop {
                let (root, wts, filter, cur, before) = {
                    let it = handle.borrow();
                    if it.detached {
                        return Ok(JsValue::Null);
                    }
                    (it.root, it.what_to_show, it.filter.clone(), it.reference, it.pointer_before)
                };
                let (prev_cur, prev_before) = if !before {
                    (cur, true)
                } else {
                    let Some(p) = prev_in_preorder(&d.borrow().doc, root, cur) else {
                        return Ok(JsValue::Null);
                    };
                    (p, true)
                };
                {
                    let mut it = handle.borrow_mut();
                    it.reference = prev_cur;
                    it.pointer_before = prev_before;
                }
                match apply_node_filter(rt, &d, wts, &filter, prev_cur) {
                    Ok(1) => return Ok(JsValue::Object(wrap_node(rt, &d, prev_cur))),
                    Ok(_) => continue,
                    Err(e) => {
                        let mut it = handle.borrow_mut();
                        it.reference = cur;
                        it.pointer_before = before;
                        return Err(e);
                    }
                }
            }
        }),
    );
    let _ = rt.set_property(iter_proto, "previousNode", JsValue::Object(prev_node));

    let detach = rt.create_native_function(
        "detach",
        0,
        Rc::new(|rt, this, _| {
            if let Some(oid) = this.as_object() {
                if let Some(it) = rt.get_host_data::<NodeIteratorHandle>(oid) {
                    it.0.borrow_mut().detached = true;
                }
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(iter_proto, "detach", JsValue::Object(detach));

    let d = dom.clone();
    let get_cur = rt.create_native_function(
        "get currentNode",
        0,
        Rc::new(move |rt, this, _| {
            let cid = this
                .as_object()
                .and_then(|o| rt.get_host_data::<TreeWalkerData>(o))
                .map(|w| w.current);
            Ok(wrap_node_opt(rt, &d, cid))
        }),
    );
    let set_cur = rt.create_native_function(
        "set currentNode",
        1,
        Rc::new(|rt, this, args| {
            let Some(oid) = this.as_object() else {
                return Ok(JsValue::Undefined);
            };
            let Some(nid) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Err(JsError::type_err("currentNode must be a Node"));
            };
            if let Some(w) = rt.get_host_data_mut::<TreeWalkerData>(oid) {
                w.current = nid;
            }
            Ok(JsValue::Undefined)
        }),
    );
    define_getter_setter(rt, walker_proto, "currentNode", Some(get_cur), Some(set_cur));

    let d = dom.clone();
    let w_parent = rt.create_native_function(
        "parentNode",
        0,
        Rc::new(move |rt, this, _| {
            let Some(oid) = this.as_object() else {
                return Ok(JsValue::Null);
            };
            let (root, wts, filter, mut cur) = {
                let Some(w) = rt.get_host_data::<TreeWalkerData>(oid) else {
                    return Ok(JsValue::Null);
                };
                (w.root, w.what_to_show, w.filter.clone(), w.current)
            };
            while cur != root {
                let p = d.borrow().doc.node(cur).parent;
                let Some(pid) = p else {
                    break;
                };
                cur = pid;
                if apply_node_filter(rt, &d, wts, &filter, cur)? == 1 {
                    if let Some(w) = rt.get_host_data_mut::<TreeWalkerData>(oid) {
                        w.current = cur;
                    }
                    return Ok(JsValue::Object(wrap_node(rt, &d, cur)));
                }
            }
            Ok(JsValue::Null)
        }),
    );
    let _ = rt.set_property(walker_proto, "parentNode", JsValue::Object(w_parent));

    let d = dom.clone();
    let w_first = rt.create_native_function(
        "firstChild",
        0,
        Rc::new(move |rt, this, _| walker_traverse_children(rt, &d, this, true)),
    );
    let _ = rt.set_property(walker_proto, "firstChild", JsValue::Object(w_first));

    let d = dom.clone();
    let w_last = rt.create_native_function(
        "lastChild",
        0,
        Rc::new(move |rt, this, _| walker_traverse_children(rt, &d, this, false)),
    );
    let _ = rt.set_property(walker_proto, "lastChild", JsValue::Object(w_last));

    let d = dom.clone();
    let w_next_sib = rt.create_native_function(
        "nextSibling",
        0,
        Rc::new(move |rt, this, _| walker_traverse_siblings(rt, &d, this, true)),
    );
    let _ = rt.set_property(walker_proto, "nextSibling", JsValue::Object(w_next_sib));

    let d = dom.clone();
    let w_prev_sib = rt.create_native_function(
        "previousSibling",
        0,
        Rc::new(move |rt, this, _| walker_traverse_siblings(rt, &d, this, false)),
    );
    let _ = rt.set_property(walker_proto, "previousSibling", JsValue::Object(w_prev_sib));

    let d = dom.clone();
    let w_next = rt.create_native_function(
        "nextNode",
        0,
        Rc::new(move |rt, this, _| {
            let Some(oid) = this.as_object() else {
                return Ok(JsValue::Null);
            };
            let (root, wts, filter, mut cur) = {
                let Some(w) = rt.get_host_data::<TreeWalkerData>(oid) else {
                    return Ok(JsValue::Null);
                };
                (w.root, w.what_to_show, w.filter.clone(), w.current)
            };
            let mut result = 1u8;
            loop {
                while result != 2 {
                    let fc = d.borrow().doc.node(cur).first_child;
                    let Some(c) = fc else {
                        break;
                    };
                    cur = c;
                    result = apply_node_filter(rt, &d, wts, &filter, cur)?;
                    if result == 1 {
                        if let Some(w) = rt.get_host_data_mut::<TreeWalkerData>(oid) {
                            w.current = cur;
                        }
                        return Ok(JsValue::Object(wrap_node(rt, &d, cur)));
                    }
                }
                let mut temp = Some(cur);
                while let Some(t) = temp {
                    if t == root {
                        return Ok(JsValue::Null);
                    }
                    let ns = d.borrow().doc.node(t).next_sibling;
                    if let Some(s) = ns {
                        cur = s;
                        break;
                    }
                    temp = d.borrow().doc.node(t).parent;
                }
                if temp.is_none() {
                    return Ok(JsValue::Null);
                }
                result = apply_node_filter(rt, &d, wts, &filter, cur)?;
                if result == 1 {
                    if let Some(w) = rt.get_host_data_mut::<TreeWalkerData>(oid) {
                        w.current = cur;
                    }
                    return Ok(JsValue::Object(wrap_node(rt, &d, cur)));
                }
            }
        }),
    );
    let _ = rt.set_property(walker_proto, "nextNode", JsValue::Object(w_next));

    let d = dom.clone();
    let w_prev = rt.create_native_function(
        "previousNode",
        0,
        Rc::new(move |rt, this, _| {
            let Some(oid) = this.as_object() else {
                return Ok(JsValue::Null);
            };
            let (root, wts, filter, mut cur) = {
                let Some(w) = rt.get_host_data::<TreeWalkerData>(oid) else {
                    return Ok(JsValue::Null);
                };
                (w.root, w.what_to_show, w.filter.clone(), w.current)
            };
            while cur != root {
                let mut sib = d.borrow().doc.node(cur).prev_sibling;
                while let Some(mut s) = sib {
                    let mut res = apply_node_filter(rt, &d, wts, &filter, s)?;
                    while res != 2 {
                        let lc = d.borrow().doc.node(s).last_child;
                        let Some(c) = lc else {
                            break;
                        };
                        s = c;
                        res = apply_node_filter(rt, &d, wts, &filter, s)?;
                    }
                    if res == 1 {
                        if let Some(w) = rt.get_host_data_mut::<TreeWalkerData>(oid) {
                            w.current = s;
                        }
                        return Ok(JsValue::Object(wrap_node(rt, &d, s)));
                    }
                    sib = d.borrow().doc.node(s).prev_sibling;
                }
                let p = d.borrow().doc.node(cur).parent;
                let Some(pid) = p else {
                    return Ok(JsValue::Null);
                };
                cur = pid;
                if apply_node_filter(rt, &d, wts, &filter, cur)? == 1 {
                    if let Some(w) = rt.get_host_data_mut::<TreeWalkerData>(oid) {
                        w.current = cur;
                    }
                    return Ok(JsValue::Object(wrap_node(rt, &d, cur)));
                }
            }
            Ok(JsValue::Null)
        }),
    );
    let _ = rt.set_property(walker_proto, "previousNode", JsValue::Object(w_prev));
}

fn walker_traverse_children(
    rt: &mut JsRuntime,
    dom: &SharedDom,
    this: JsValue,
    first: bool,
) -> Result<JsValue, JsError> {
    let Some(oid) = this.as_object() else {
        return Ok(JsValue::Null);
    };
    let (wts, filter, start) = {
        let Some(w) = rt.get_host_data::<TreeWalkerData>(oid) else {
            return Ok(JsValue::Null);
        };
        (w.what_to_show, w.filter.clone(), w.current)
    };
    let mut node_opt = if first {
        dom.borrow().doc.node(start).first_child
    } else {
        dom.borrow().doc.node(start).last_child
    };
    while let Some(node) = node_opt {
        let res = apply_node_filter(rt, dom, wts, &filter, node)?;
        if res == 1 {
            if let Some(w) = rt.get_host_data_mut::<TreeWalkerData>(oid) {
                w.current = node;
            }
            return Ok(JsValue::Object(wrap_node(rt, dom, node)));
        }
        if res == 3 {
            let child = if first {
                dom.borrow().doc.node(node).first_child
            } else {
                dom.borrow().doc.node(node).last_child
            };
            if child.is_some() {
                node_opt = child;
                continue;
            }
        }
        let mut cur = node;
        loop {
            let sib = if first {
                dom.borrow().doc.node(cur).next_sibling
            } else {
                dom.borrow().doc.node(cur).prev_sibling
            };
            if sib.is_some() {
                node_opt = sib;
                break;
            }
            let p = dom.borrow().doc.node(cur).parent;
            match p {
                Some(pid) if pid != start => cur = pid,
                _ => return Ok(JsValue::Null),
            }
        }
    }
    Ok(JsValue::Null)
}

fn walker_traverse_siblings(
    rt: &mut JsRuntime,
    dom: &SharedDom,
    this: JsValue,
    next: bool,
) -> Result<JsValue, JsError> {
    let Some(oid) = this.as_object() else {
        return Ok(JsValue::Null);
    };
    let (root, wts, filter, mut node) = {
        let Some(w) = rt.get_host_data::<TreeWalkerData>(oid) else {
            return Ok(JsValue::Null);
        };
        (w.root, w.what_to_show, w.filter.clone(), w.current)
    };
    if node == root {
        return Ok(JsValue::Null);
    }
    loop {
        let mut sib = if next {
            dom.borrow().doc.node(node).next_sibling
        } else {
            dom.borrow().doc.node(node).prev_sibling
        };
        while let Some(s) = sib {
            node = s;
            let res = apply_node_filter(rt, dom, wts, &filter, node)?;
            if res == 1 {
                if let Some(w) = rt.get_host_data_mut::<TreeWalkerData>(oid) {
                    w.current = node;
                }
                return Ok(JsValue::Object(wrap_node(rt, dom, node)));
            }
            let child = if res != 2 {
                if next {
                    dom.borrow().doc.node(node).first_child
                } else {
                    dom.borrow().doc.node(node).last_child
                }
            } else {
                None
            };
            sib = child.or_else(|| {
                if next {
                    dom.borrow().doc.node(node).next_sibling
                } else {
                    dom.borrow().doc.node(node).prev_sibling
                }
            });
        }
        let p = dom.borrow().doc.node(node).parent;
        let Some(pid) = p else {
            return Ok(JsValue::Null);
        };
        if pid == root {
            return Ok(JsValue::Null);
        }
        node = pid;
        if apply_node_filter(rt, dom, wts, &filter, node)? == 1 {
            return Ok(JsValue::Null);
        }
    }
}

fn node_length(doc: &Document, nid: NodeId) -> usize {
    match &doc.node(nid).data {
        NodeData::Text(s) | NodeData::Comment(s) => s.encode_utf16().count(),
        NodeData::ProcessingInstruction { data, .. } => data.encode_utf16().count(),
        _ => doc.children(nid).len(),
    }
}

fn child_index(doc: &Document, nid: NodeId) -> Option<(NodeId, usize)> {
    let parent = doc.node(nid).parent?;
    let idx = doc.children(parent).iter().position(|&c| c == nid)?;
    Some((parent, idx))
}

fn compare_bp(doc: &Document, a_node: NodeId, a_off: usize, b_node: NodeId, b_off: usize) -> i32 {
    if a_node == b_node {
        return if a_off < b_off {
            -1
        } else if a_off > b_off {
            1
        } else {
            0
        };
    }
    if doc.is_ancestor(a_node, b_node) {
        let mut cur = b_node;
        while let Some(p) = doc.node(cur).parent {
            if p == a_node {
                let idx = doc.children(a_node).iter().position(|&c| c == cur).unwrap_or(0);
                return if a_off <= idx { -1 } else { 1 };
            }
            cur = p;
        }
    }
    if doc.is_ancestor(b_node, a_node) {
        return -compare_bp(doc, b_node, b_off, a_node, a_off);
    }
    let mut root = a_node;
    while let Some(p) = doc.node(root).parent {
        root = p;
    }
    let mut order = Vec::new();
    collect_subtree_preorder(doc, root, &mut order);
    let pa = order.iter().position(|&id| id == a_node);
    let pb = order.iter().position(|&id| id == b_node);
    match (pa, pb) {
        (Some(ia), Some(ib)) => {
            if ia < ib {
                -1
            } else {
                1
            }
        }
        _ => 0,
    }
}

fn common_ancestor(doc: &Document, a: NodeId, b: NodeId) -> NodeId {
    let mut anc = Vec::new();
    let mut cur = Some(a);
    while let Some(id) = cur {
        anc.push(id);
        cur = doc.node(id).parent;
    }
    let mut cur_b = Some(b);
    while let Some(id) = cur_b {
        if anc.contains(&id) {
            return id;
        }
        cur_b = doc.node(id).parent;
    }
    a
}

fn is_inclusive_ancestor(doc: &Document, anc: NodeId, node: NodeId) -> bool {
    anc == node || doc.is_ancestor(anc, node)
}

fn is_character_data(doc: &Document, nid: NodeId) -> bool {
    matches!(
        doc.node(nid).data,
        NodeData::Text(_) | NodeData::Comment(_) | NodeData::ProcessingInstruction { .. }
    )
}

fn get_range_handle(rt: &JsRuntime, val: &JsValue) -> Option<Rc<RefCell<RangeData>>> {
    let oid = val.as_object()?;
    rt.get_host_data::<RangeHandle>(oid).map(|h| h.0.clone())
}

pub fn define_full_range_proto(rt: &mut JsRuntime, dom: SharedDom, range_proto: ObjectId) {
    for (name, val) in [
        ("START_TO_START", 0.0),
        ("START_TO_END", 1.0),
        ("END_TO_END", 2.0),
        ("END_TO_START", 3.0),
    ] {
        let _ = rt.set_property(range_proto, name, JsValue::Number(val));
    }

    let d = dom.clone();
    let get_sc = rt.create_native_function(
        "get startContainer",
        0,
        Rc::new(move |rt, this, _| {
            let id = get_range_handle(rt, &this).map(|r| r.borrow().start_container);
            Ok(wrap_node_opt(rt, &d, id))
        }),
    );
    define_getter_setter(rt, range_proto, "startContainer", Some(get_sc), None);

    let get_so = rt.create_native_function(
        "get startOffset",
        0,
        Rc::new(|rt, this, _| {
            let o = get_range_handle(rt, &this).map_or(0, |r| r.borrow().start_offset);
            Ok(JsValue::Number(o as f64))
        }),
    );
    define_getter_setter(rt, range_proto, "startOffset", Some(get_so), None);

    let d = dom.clone();
    let get_ec = rt.create_native_function(
        "get endContainer",
        0,
        Rc::new(move |rt, this, _| {
            let id = get_range_handle(rt, &this).map(|r| r.borrow().end_container);
            Ok(wrap_node_opt(rt, &d, id))
        }),
    );
    define_getter_setter(rt, range_proto, "endContainer", Some(get_ec), None);

    let get_eo = rt.create_native_function(
        "get endOffset",
        0,
        Rc::new(|rt, this, _| {
            let o = get_range_handle(rt, &this).map_or(0, |r| r.borrow().end_offset);
            Ok(JsValue::Number(o as f64))
        }),
    );
    define_getter_setter(rt, range_proto, "endOffset", Some(get_eo), None);

    let get_col = rt.create_native_function(
        "get collapsed",
        0,
        Rc::new(|rt, this, _| {
            let c = get_range_handle(rt, &this).map_or(true, |r| {
                let r = r.borrow();
                r.start_container == r.end_container && r.start_offset == r.end_offset
            });
            Ok(JsValue::Boolean(c))
        }),
    );
    define_getter_setter(rt, range_proto, "collapsed", Some(get_col), None);

    let d = dom.clone();
    let get_cac = rt.create_native_function(
        "get commonAncestorContainer",
        0,
        Rc::new(move |rt, this, _| {
            let pair = get_range_handle(rt, &this).map(|r| {
                let r = r.borrow();
                (r.start_container, r.end_container)
            });
            let Some((a, b)) = pair else {
                return Ok(JsValue::Null);
            };
            let anc = common_ancestor(&d.borrow().doc, a, b);
            Ok(JsValue::Object(wrap_node(rt, &d, anc)))
        }),
    );
    define_getter_setter(rt, range_proto, "commonAncestorContainer", Some(get_cac), None);

    let d = dom.clone();
    let set_start = rt.create_native_function(
        "setStart",
        2,
        Rc::new(move |rt, this, args| {
            let Some(rh) = get_range_handle(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let Some(nid) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Err(JsError::type_err("setStart requires Node"));
            };
            let off_n = args.get(1).and_then(|v| rt.to_number(v).ok()).unwrap_or(0.0);
            let max_len = node_length(&d.borrow().doc, nid);
            if off_n < 0.0 || (off_n as usize) > max_len {
                return Err(throw_dom_exception(rt, &d, 1, "IndexSizeError", "offset out of range"));
            }
            let off = off_n as usize;
            let mut r = rh.borrow_mut();
            let collapse = compare_bp(&d.borrow().doc, nid, off, r.end_container, r.end_offset) > 0;
            r.start_container = nid;
            r.start_offset = off;
            if collapse {
                r.end_container = nid;
                r.end_offset = off;
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(range_proto, "setStart", JsValue::Object(set_start));

    let d = dom.clone();
    let set_end = rt.create_native_function(
        "setEnd",
        2,
        Rc::new(move |rt, this, args| {
            let Some(rh) = get_range_handle(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let Some(nid) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Err(JsError::type_err("setEnd requires Node"));
            };
            let off_n = args.get(1).and_then(|v| rt.to_number(v).ok()).unwrap_or(0.0);
            let max_len = node_length(&d.borrow().doc, nid);
            if off_n < 0.0 || (off_n as usize) > max_len {
                return Err(throw_dom_exception(rt, &d, 1, "IndexSizeError", "offset out of range"));
            }
            let off = off_n as usize;
            let mut r = rh.borrow_mut();
            let collapse = compare_bp(&d.borrow().doc, r.start_container, r.start_offset, nid, off) > 0;
            r.end_container = nid;
            r.end_offset = off;
            if collapse {
                r.start_container = nid;
                r.start_offset = off;
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(range_proto, "setEnd", JsValue::Object(set_end));

    for (name, is_start, delta) in [
        ("setStartBefore", true, 0usize),
        ("setStartAfter", true, 1usize),
        ("setEndBefore", false, 0usize),
        ("setEndAfter", false, 1usize),
    ] {
        let d = dom.clone();
        let f = rt.create_native_function(
            name,
            1,
            Rc::new(move |rt, this, args| {
                let Some(rh) = get_range_handle(rt, &this) else {
                    return Ok(JsValue::Undefined);
                };
                let Some(nid) = args.first().and_then(|v| node_id_of(rt, v)) else {
                    return Err(JsError::type_err("requires Node"));
                };
                let Some((parent, idx)) = child_index(&d.borrow().doc, nid) else {
                    return Err(throw_dom_exception(rt, &d, 24, "InvalidNodeTypeError", "node has no parent"));
                };
                let off = idx + delta;
                let mut r = rh.borrow_mut();
                let collapse = if is_start {
                    compare_bp(&d.borrow().doc, parent, off, r.end_container, r.end_offset) > 0
                } else {
                    compare_bp(&d.borrow().doc, r.start_container, r.start_offset, parent, off) > 0
                };
                if is_start {
                    r.start_container = parent;
                    r.start_offset = off;
                    if collapse {
                        r.end_container = parent;
                        r.end_offset = off;
                    }
                } else {
                    r.end_container = parent;
                    r.end_offset = off;
                    if collapse {
                        r.start_container = parent;
                        r.start_offset = off;
                    }
                }
                Ok(JsValue::Undefined)
            }),
        );
        let _ = rt.set_property(range_proto, name, JsValue::Object(f));
    }

    let collapse_fn = rt.create_native_function(
        "collapse",
        1,
        Rc::new(|rt, this, args| {
            let to_start = args.first().map_or(false, |v| rt.to_boolean(v));
            if let Some(rh) = get_range_handle(rt, &this) {
                let mut r = rh.borrow_mut();
                if to_start {
                    r.end_container = r.start_container;
                    r.end_offset = r.start_offset;
                } else {
                    r.start_container = r.end_container;
                    r.start_offset = r.end_offset;
                }
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(range_proto, "collapse", JsValue::Object(collapse_fn));

    let d = dom.clone();
    let sel_node = rt.create_native_function(
        "selectNode",
        1,
        Rc::new(move |rt, this, args| {
            let Some(rh) = get_range_handle(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let Some(nid) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Err(JsError::type_err("selectNode requires Node"));
            };
            let Some((parent, idx)) = child_index(&d.borrow().doc, nid) else {
                return Err(throw_dom_exception(rt, &d, 24, "InvalidNodeTypeError", "node has no parent"));
            };
            let mut r = rh.borrow_mut();
            r.start_container = parent;
            r.start_offset = idx;
            r.end_container = parent;
            r.end_offset = idx + 1;
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(range_proto, "selectNode", JsValue::Object(sel_node));

    let d = dom.clone();
    let sel_contents = rt.create_native_function(
        "selectNodeContents",
        1,
        Rc::new(move |rt, this, args| {
            let Some(rh) = get_range_handle(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let Some(nid) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Err(JsError::type_err("selectNodeContents requires Node"));
            };
            let len = node_length(&d.borrow().doc, nid);
            let mut r = rh.borrow_mut();
            r.start_container = nid;
            r.start_offset = 0;
            r.end_container = nid;
            r.end_offset = len;
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(range_proto, "selectNodeContents", JsValue::Object(sel_contents));

    let d = dom.clone();
    let cmp_bp = rt.create_native_function(
        "compareBoundaryPoints",
        2,
        Rc::new(move |rt, this, args| {
            let how = args.first().and_then(|v| rt.to_number(v).ok()).unwrap_or(0.0) as u32;
            let Some(other_val) = args.get(1) else {
                return Err(JsError::type_err("compareBoundaryPoints requires Range"));
            };
            let (Some(ah), Some(bh)) = (get_range_handle(rt, &this), get_range_handle(rt, other_val)) else {
                return Ok(JsValue::Number(0.0));
            };
            let a = *ah.borrow();
            let b = *bh.borrow();
            let (an, ao, bn, bo) = match how {
                0 => (a.start_container, a.start_offset, b.start_container, b.start_offset),
                1 => (a.end_container, a.end_offset, b.start_container, b.start_offset),
                2 => (a.end_container, a.end_offset, b.end_container, b.end_offset),
                3 => (a.start_container, a.start_offset, b.end_container, b.end_offset),
                _ => return Err(throw_dom_exception(rt, &d, 9, "NotSupportedError", "invalid how")),
            };
            let res = compare_bp(&d.borrow().doc, an, ao, bn, bo);
            Ok(JsValue::Number(res as f64))
        }),
    );
    let _ = rt.set_property(range_proto, "compareBoundaryPoints", JsValue::Object(cmp_bp));

    let d = dom.clone();
    let del_contents = rt.create_native_function(
        "deleteContents",
        0,
        Rc::new(move |rt, this, _| {
            let Some(rh) = get_range_handle(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let r = *rh.borrow();
            let (_, new_pos) = {
                let mut st = d.borrow_mut();
                let res = range_extract_or_delete(&mut st, r, false, false);
                st.mark_dirty();
                res
            };
            if let Some((nc, no)) = new_pos {
                let mut rm = rh.borrow_mut();
                rm.start_container = nc;
                rm.start_offset = no;
                rm.end_container = nc;
                rm.end_offset = no;
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(range_proto, "deleteContents", JsValue::Object(del_contents));

    let d = dom.clone();
    let ext_contents = rt.create_native_function(
        "extractContents",
        0,
        Rc::new(move |rt, this, _| {
            let Some(rh) = get_range_handle(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let r = *rh.borrow();
            let (frag, new_pos) = {
                let mut st = d.borrow_mut();
                let res = range_extract_or_delete(&mut st, r, true, false);
                st.mark_dirty();
                res
            };
            if let Some((nc, no)) = new_pos {
                let mut rm = rh.borrow_mut();
                rm.start_container = nc;
                rm.start_offset = no;
                rm.end_container = nc;
                rm.end_offset = no;
            }
            Ok(JsValue::Object(wrap_node(rt, &d, frag)))
        }),
    );
    let _ = rt.set_property(range_proto, "extractContents", JsValue::Object(ext_contents));

    let d = dom.clone();
    let clone_contents = rt.create_native_function(
        "cloneContents",
        0,
        Rc::new(move |rt, this, _| {
            let Some(rh) = get_range_handle(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let r = *rh.borrow();
            let (frag, _) = {
                let mut st = d.borrow_mut();
                range_extract_or_delete(&mut st, r, true, true)
            };
            Ok(JsValue::Object(wrap_node(rt, &d, frag)))
        }),
    );
    let _ = rt.set_property(range_proto, "cloneContents", JsValue::Object(clone_contents));

    let d = dom.clone();
    let insert_node = rt.create_native_function(
        "insertNode",
        1,
        Rc::new(move |rt, this, args| {
            let Some(rh) = get_range_handle(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let Some(new_node) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Err(JsError::type_err("insertNode requires Node"));
            };
            let r = *rh.borrow();
            let mut st = d.borrow_mut();
            let is_text = matches!(st.doc.node(r.start_container).data, NodeData::Text(_));
            if is_text {
                if let Some(parent) = st.doc.node(r.start_container).parent {
                    let tail = if let NodeData::Text(ref mut s) = st.doc.node_mut(r.start_container).data {
                        let split_at = utf16_offset_to_byte(s, r.start_offset);
                        s.split_off(split_at)
                    } else {
                        String::new()
                    };
                    let after_text = st.doc.create_text(tail);
                    let next_sib = st.doc.node(r.start_container).next_sibling;
                    st.doc.insert_before(parent, after_text, next_sib);
                    if st.doc.node(new_node).parent.is_some() {
                        st.notify_pre_remove(new_node);
                    }
                    st.doc.insert_before(parent, new_node, Some(after_text));
                    let new_idx = child_index(&st.doc, new_node).map(|(_, i)| i).unwrap_or(0);
                    let mut rm = rh.borrow_mut();
                    rm.start_container = parent;
                    rm.start_offset = new_idx;
                    if r.end_container == r.start_container {
                        rm.end_container = after_text;
                        rm.end_offset = r.end_offset.saturating_sub(r.start_offset);
                    }
                }
            } else {
                let kids = st.doc.children(r.start_container);
                let ref_node = kids.get(r.start_offset).copied();
                if st.doc.node(new_node).parent.is_some() {
                    st.notify_pre_remove(new_node);
                }
                st.doc.insert_before(r.start_container, new_node, ref_node);
            }
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(range_proto, "insertNode", JsValue::Object(insert_node));

    let d = dom.clone();
    let surround = rt.create_native_function(
        "surroundContents",
        1,
        Rc::new(move |rt, this, args| {
            let Some(rh) = get_range_handle(rt, &this) else {
                return Ok(JsValue::Undefined);
            };
            let Some(wrapper) = args.first().and_then(|v| node_id_of(rt, v)) else {
                return Err(JsError::type_err("surroundContents requires Node"));
            };
            let r = *rh.borrow();
            let err_kind = {
                let st = d.borrow();
                let mut partially_non_text = false;
                let anc = common_ancestor(&st.doc, r.start_container, r.end_container);
                if r.start_container != r.end_container {
                    let mut cur = r.start_container;
                    while cur != anc {
                        if !matches!(st.doc.node(cur).data, NodeData::Text(_)) {
                            partially_non_text = true;
                            break;
                        }
                        match st.doc.node(cur).parent {
                            Some(p) => cur = p,
                            None => break,
                        }
                    }
                    if !partially_non_text {
                        let mut cur = r.end_container;
                        while cur != anc {
                            if !matches!(st.doc.node(cur).data, NodeData::Text(_)) {
                                partially_non_text = true;
                                break;
                            }
                            match st.doc.node(cur).parent {
                                Some(p) => cur = p,
                                None => break,
                            }
                        }
                    }
                }
                if partially_non_text {
                    Some((11u16, "InvalidStateError", "Partially selects a non-Text node"))
                } else if matches!(
                    st.doc.node(wrapper).data,
                    NodeData::Document { .. }
                        | NodeData::Doctype { .. }
                        | NodeData::DocumentFragment
                ) {
                    Some((24u16, "InvalidNodeTypeError", "Invalid wrapper node type"))
                } else {
                    let parent_is_doc = matches!(st.doc.node(r.start_container).data, NodeData::Document { .. });
                    let wrapper_is_el = matches!(st.doc.node(wrapper).data, NodeData::Element(_));
                    let has_other_doc_el = parent_is_doc
                        && wrapper_is_el
                        && st.doc.element_children(r.start_container).into_iter().any(|c| {
                            let idx = child_index(&st.doc, c).map(|(_, i)| i).unwrap_or(0);
                            let in_range = r.start_container == r.end_container
                                && idx >= r.start_offset
                                && idx < r.end_offset;
                            !in_range
                        });
                    if has_other_doc_el || wrapper == r.start_container || st.doc.is_ancestor(wrapper, r.start_container) {
                        Some((3u16, "HierarchyRequestError", "Invalid node hierarchy"))
                    } else {
                        None
                    }
                }
            };
            if let Some((code, name, msg)) = err_kind {
                return Err(throw_dom_exception(rt, &d, code, name, msg));
            }
            let (frag, new_pos) = {
                let mut st = d.borrow_mut();
                let res = range_extract_or_delete(&mut st, r, true, false);
                for c in st.doc.children(wrapper) {
                    st.notify_pre_remove(c);
                    st.doc.detach(c);
                }
                let (ins_container, ins_offset) = res.1.unwrap_or((r.start_container, r.start_offset));
                let kids = st.doc.children(ins_container);
                let ref_node = kids.get(ins_offset).copied();
                if st.doc.node(wrapper).parent.is_some() {
                    st.notify_pre_remove(wrapper);
                }
                st.doc.insert_before(ins_container, wrapper, ref_node);
                st.doc.append_child(wrapper, res.0);
                st.mark_dirty();
                res
            };
            let _ = (frag, new_pos);
            if let Some((parent, idx)) = child_index(&d.borrow().doc, wrapper) {
                let mut rm = rh.borrow_mut();
                rm.start_container = parent;
                rm.start_offset = idx;
                rm.end_container = parent;
                rm.end_offset = idx + 1;
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(range_proto, "surroundContents", JsValue::Object(surround));

    let d = dom.clone();
    let clone_r = rt.create_native_function(
        "cloneRange",
        0,
        Rc::new(move |rt, this, _| {
            let Some(rh) = get_range_handle(rt, &this) else {
                return Ok(JsValue::Null);
            };
            let r = *rh.borrow();
            let proto = d.borrow().protos.unwrap().range_proto;
            let obj = rt.create_object_with_proto(Some(proto));
            let rc = Rc::new(RefCell::new(r));
            d.borrow_mut().live_ranges.push(rc.clone());
            rt.set_host_data(obj, RangeHandle(rc));
            Ok(JsValue::Object(obj))
        }),
    );
    let _ = rt.set_property(range_proto, "cloneRange", JsValue::Object(clone_r));

    let detach_r = rt.create_native_function("detach", 0, Rc::new(|_, _, _| Ok(JsValue::Undefined)));
    let _ = rt.set_property(range_proto, "detach", JsValue::Object(detach_r));

    let d = dom.clone();
    let to_str = rt.create_native_function(
        "toString",
        0,
        Rc::new(move |rt, this, _| {
            let Some(rh) = get_range_handle(rt, &this) else {
                return Ok(JsValue::String(String::new()));
            };
            let r = *rh.borrow();
            let txt = range_to_string(&d.borrow().doc, &r);
            Ok(JsValue::String(txt))
        }),
    );
    let _ = rt.set_property(range_proto, "toString", JsValue::Object(to_str));

    let d = dom.clone();
    let create_ctx_frag = rt.create_native_function(
        "createContextualFragment",
        1,
        Rc::new(move |rt, _, args| {
            let html = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let frag = {
                let mut st = d.borrow_mut();
                let f = st.doc.create_fragment();
                let nodes = peedle_html::parse_fragment_into(&mut st.doc, "div", Namespace::Html, &html);
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

fn range_to_string(doc: &Document, r: &RangeData) -> String {
    if r.start_container == r.end_container && r.start_offset == r.end_offset {
        return String::new();
    }
    if r.start_container == r.end_container {
        if let NodeData::Text(ref s) = doc.node(r.start_container).data {
            let a = utf16_offset_to_byte(s, r.start_offset);
            let b = utf16_offset_to_byte(s, r.end_offset);
            return s[a..b].to_string();
        }
        if matches!(
            doc.node(r.start_container).data,
            NodeData::Comment(_) | NodeData::ProcessingInstruction { .. }
        ) {
            return String::new();
        }
        let kids = doc.children(r.start_container);
        let end = r.end_offset.min(kids.len());
        let start = r.start_offset.min(end);
        let mut out = String::new();
        for &cid in &kids[start..end] {
            if !matches!(
                doc.node(cid).data,
                NodeData::Comment(_) | NodeData::ProcessingInstruction { .. }
            ) {
                out.push_str(&doc.text_content(cid));
            }
        }
        return out;
    }
    let anc = common_ancestor(doc, r.start_container, r.end_container);
    let mut order = Vec::new();
    collect_subtree_preorder(doc, anc, &mut order);
    let mut out = String::new();
    for nid in order {
        if let NodeData::Text(ref s) = doc.node(nid).data {
            if nid == r.start_container {
                let a = utf16_offset_to_byte(s, r.start_offset);
                out.push_str(&s[a..]);
            } else if nid == r.end_container {
                let b = utf16_offset_to_byte(s, r.end_offset);
                out.push_str(&s[..b]);
            } else if compare_bp(doc, nid, 0, r.start_container, r.start_offset) >= 0
                && compare_bp(doc, nid, node_length(doc, nid), r.end_container, r.end_offset) <= 0
            {
                out.push_str(s);
            }
        }
    }
    out
}

fn utf16_offset_to_byte(s: &str, u16_off: usize) -> usize {
    let mut cur = 0usize;
    for (byte_idx, ch) in s.char_indices() {
        if cur >= u16_off {
            return byte_idx;
        }
        cur += ch.len_utf16();
    }
    s.len()
}

fn clone_char_data_slice(st: &mut DomState, nid: NodeId, start_u16: usize, end_u16: usize, remove_from_orig: bool) -> Option<NodeId> {
    let kind_and_sub = match &mut st.doc.node_mut(nid).data {
        NodeData::Text(ref mut s) => {
            let a = utf16_offset_to_byte(s, start_u16);
            let b = utf16_offset_to_byte(s, end_u16);
            let sub = s[a..b].to_string();
            if remove_from_orig {
                s.replace_range(a..b, "");
            }
            Some((0u8, String::new(), sub))
        }
        NodeData::Comment(ref mut s) => {
            let a = utf16_offset_to_byte(s, start_u16);
            let b = utf16_offset_to_byte(s, end_u16);
            let sub = s[a..b].to_string();
            if remove_from_orig {
                s.replace_range(a..b, "");
            }
            Some((1u8, String::new(), sub))
        }
        NodeData::ProcessingInstruction { target, data } => {
            let a = utf16_offset_to_byte(data, start_u16);
            let b = utf16_offset_to_byte(data, end_u16);
            let sub = data[a..b].to_string();
            let t = target.clone();
            if remove_from_orig {
                data.replace_range(a..b, "");
            }
            Some((2u8, t, sub))
        }
        _ => None,
    }?;
    Some(match kind_and_sub.0 {
        0 => st.doc.create_text(kind_and_sub.2),
        1 => st.doc.create_comment(kind_and_sub.2),
        _ => st.doc.create_processing_instruction(kind_and_sub.1, kind_and_sub.2),
    })
}

fn range_extract_or_delete(
    st: &mut DomState,
    r: RangeData,
    keep_frag: bool,
    clone_only: bool,
) -> (NodeId, Option<(NodeId, usize)>) {
    let frag = st.doc.create_fragment();
    if r.start_container == r.end_container && r.start_offset == r.end_offset {
        return (frag, Some((r.start_container, r.start_offset)));
    }

    if r.start_container == r.end_container {
        if is_character_data(&st.doc, r.start_container) {
            if let Some(cloned) = clone_char_data_slice(
                st,
                r.start_container,
                r.start_offset,
                r.end_offset,
                !clone_only,
            ) {
                if keep_frag {
                    st.doc.append_child(frag, cloned);
                }
            }
            return (frag, Some((r.start_container, r.start_offset)));
        }
        let kids = st.doc.children(r.start_container);
        let end = r.end_offset.min(kids.len());
        let start = r.start_offset.min(end);
        for &cid in &kids[start..end] {
            if clone_only {
                let c = st.doc.clone_node(cid, true);
                st.doc.append_child(frag, c);
            } else if keep_frag {
                st.notify_pre_remove(cid);
                st.doc.append_child(frag, cid);
            } else {
                st.notify_pre_remove(cid);
                st.doc.detach(cid);
            }
        }
        return (frag, Some((r.start_container, r.start_offset)));
    }

    let anc = common_ancestor(&st.doc, r.start_container, r.end_container);
    let new_pos = if is_inclusive_ancestor(&st.doc, r.start_container, r.end_container) {
        (r.start_container, r.start_offset)
    } else {
        let mut ref_node = r.start_container;
        while let Some(p) = st.doc.node(ref_node).parent {
            if is_inclusive_ancestor(&st.doc, p, r.end_container) {
                break;
            }
            ref_node = p;
        }
        if let Some((parent, idx)) = child_index(&st.doc, ref_node) {
            (parent, idx + 1)
        } else {
            (r.start_container, r.start_offset)
        }
    };

    let kids = st.doc.children(anc);
    let first_partial = if is_inclusive_ancestor(&st.doc, r.start_container, r.end_container) {
        None
    } else {
        kids.iter()
            .copied()
            .find(|&c| is_inclusive_ancestor(&st.doc, c, r.start_container))
    };
    let last_partial = if is_inclusive_ancestor(&st.doc, r.end_container, r.start_container) {
        None
    } else {
        kids.iter()
            .copied()
            .find(|&c| is_inclusive_ancestor(&st.doc, c, r.end_container))
    };

    let mut contained = Vec::new();
    for (idx, &cid) in kids.iter().enumerate() {
        if Some(cid) == first_partial || Some(cid) == last_partial {
            continue;
        }
        let after_start = compare_bp(&st.doc, anc, idx, r.start_container, r.start_offset) >= 0;
        let before_end = compare_bp(&st.doc, anc, idx + 1, r.end_container, r.end_offset) <= 0;
        if after_start && before_end {
            contained.push(cid);
        }
    }

    if let Some(fp) = first_partial {
        if is_character_data(&st.doc, fp) {
            let len = node_length(&st.doc, fp);
            if let Some(cloned) = clone_char_data_slice(st, fp, r.start_offset, len, !clone_only) {
                if keep_frag {
                    st.doc.append_child(frag, cloned);
                }
            }
        } else {
            let sub_range = RangeData {
                start_container: r.start_container,
                start_offset: r.start_offset,
                end_container: fp,
                end_offset: node_length(&st.doc, fp),
            };
            let (sub_frag, _) = range_extract_or_delete(st, sub_range, keep_frag, clone_only);
            if keep_frag {
                let shallow = st.doc.clone_node(fp, false);
                st.doc.append_child(shallow, sub_frag);
                st.doc.append_child(frag, shallow);
            }
        }
    }

    for cid in contained {
        if clone_only {
            let c = st.doc.clone_node(cid, true);
            st.doc.append_child(frag, c);
        } else if keep_frag {
            st.notify_pre_remove(cid);
            st.doc.append_child(frag, cid);
        } else {
            st.notify_pre_remove(cid);
            st.doc.detach(cid);
        }
    }

    if let Some(lp) = last_partial {
        if is_character_data(&st.doc, lp) {
            if let Some(cloned) = clone_char_data_slice(st, lp, 0, r.end_offset, !clone_only) {
                if keep_frag {
                    st.doc.append_child(frag, cloned);
                }
            }
        } else {
            let sub_range = RangeData {
                start_container: lp,
                start_offset: 0,
                end_container: r.end_container,
                end_offset: r.end_offset,
            };
            let (sub_frag, _) = range_extract_or_delete(st, sub_range, keep_frag, clone_only);
            if keep_frag {
                let shallow = st.doc.clone_node(lp, false);
                st.doc.append_child(shallow, sub_frag);
                st.doc.append_child(frag, shallow);
            }
        }
    }

    (frag, Some(new_pos))
}
