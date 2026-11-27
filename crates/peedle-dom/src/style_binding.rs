use std::rc::Rc;

use peedle_css::parse_inline_declarations;
use peedle_html::NodeId;
use peedle_js::{JsRuntime, JsValue, ObjectId};

use crate::events::define_getter_setter;
use crate::state::{ClassListHandle, SharedDom, StyleDeclHandle};

pub fn camel_to_kebab(s: &str) -> String {
    if s.starts_with("--") {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

pub fn parse_inline_map(style_attr: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for decl in parse_inline_declarations(style_attr) {
        let val = peedle_css::serialize_tokens(&decl.value).trim().to_string();
        if let Some(pos) = out.iter().position(|(k, _): &(String, String)| k == &decl.name) {
            out[pos].1 = val;
        } else {
            out.push((decl.name, val));
        }
    }
    out
}

fn serialize_inline_map(entries: &[(String, String)]) -> String {
    entries
        .iter()
        .map(|(k, v)| format!("{}: {};", k, v))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn create_style_proxy(rt: &mut JsRuntime, dom: &SharedDom, nid: NodeId, proto: ObjectId) -> ObjectId {
    let target = rt.create_object_with_proto(Some(proto));
    rt.set_host_data(target, StyleDeclHandle(nid));

    let handler = rt.create_object();
    let d_get = dom.clone();
    let get_trap = rt.create_native_function(
        "get",
        3,
        Rc::new(move |rt, _, args| {
            let Some(t_oid) = args.first().and_then(|v| v.as_object()) else {
                return Ok(JsValue::Undefined);
            };
            let prop_val = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            if matches!(prop_val, JsValue::Symbol(_)) {
                let key = rt.to_property_key(&prop_val)?;
                return rt.get_property_key(t_oid, &key);
            }
            let prop = rt.to_string(&prop_val)?;
            if prop == "cssText" {
                let txt = d_get
                    .borrow()
                    .doc
                    .element(nid)
                    .and_then(|e| e.get_attr("style"))
                    .unwrap_or("")
                    .to_string();
                return Ok(JsValue::String(txt));
            }
            if matches!(
                prop.as_str(),
                "setProperty" | "getPropertyValue" | "removeProperty" | "item" | "length"
            ) {
                return rt.get_property(t_oid, &prop);
            }
            let kebab = camel_to_kebab(&prop);
            let val = d_get
                .borrow()
                .doc
                .element(nid)
                .and_then(|e| e.get_attr("style"))
                .map(|s| {
                    parse_inline_map(s)
                        .into_iter()
                        .find(|(k, _)| k == &kebab)
                        .map(|(_, v)| v)
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            Ok(JsValue::String(val))
        }),
    );
    let _ = rt.set_property(handler, "get", JsValue::Object(get_trap));

    let d_set = dom.clone();
    let set_trap = rt.create_native_function(
        "set",
        4,
        Rc::new(move |rt, _, args| {
            let prop = args
                .get(1)
                .map(|v| rt.to_string(v))
                .transpose()?
                .unwrap_or_default();
            let val_s = args
                .get(2)
                .map(|v| {
                    if v.is_nullish() {
                        Ok(String::new())
                    } else {
                        rt.to_string(v)
                    }
                })
                .transpose()?
                .unwrap_or_default();
            let mut st = d_set.borrow_mut();
            if prop == "cssText" {
                st.doc.set_attribute(nid, "style", val_s);
                st.mark_dirty();
                return Ok(JsValue::Boolean(true));
            }
            let kebab = camel_to_kebab(&prop);
            let existing = st
                .doc
                .element(nid)
                .and_then(|e| e.get_attr("style"))
                .unwrap_or("")
                .to_string();
            let mut map = parse_inline_map(&existing);
            if val_s.trim().is_empty() {
                map.retain(|(k, _)| k != &kebab);
            } else if let Some(pos) = map.iter().position(|(k, _)| k == &kebab) {
                map[pos].1 = val_s;
            } else {
                map.push((kebab, val_s));
            }
            st.doc.set_attribute(nid, "style", serialize_inline_map(&map));
            st.mark_dirty();
            Ok(JsValue::Boolean(true))
        }),
    );
    let _ = rt.set_property(handler, "set", JsValue::Object(set_trap));

    let proxy_ctor = rt
        .get_property(rt.global_object(), "Proxy")
        .unwrap_or(JsValue::Undefined);
    if let Ok(JsValue::Object(p)) =
        rt.construct(proxy_ctor, &[JsValue::Object(target), JsValue::Object(handler)])
    {
        p
    } else {
        target
    }
}

pub fn define_style_and_classlist_protos(
    rt: &mut JsRuntime,
    dom: SharedDom,
    css_decl_proto: ObjectId,
    class_list_proto: ObjectId,
) {
    let d1 = dom.clone();
    let set_prop = rt.create_native_function(
        "setProperty",
        2,
        Rc::new(move |rt, this, args| {
            let Some(oid) = this.as_object() else {
                return Ok(JsValue::Undefined);
            };
            let Some(nid) = extract_style_node(rt, oid) else {
                return Ok(JsValue::Undefined);
            };
            let name = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let val = args.get(1).map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let mut st = d1.borrow_mut();
            let existing = st
                .doc
                .element(nid)
                .and_then(|e| e.get_attr("style"))
                .unwrap_or("")
                .to_string();
            let mut map = parse_inline_map(&existing);
            let kebab = camel_to_kebab(&name);
            if val.trim().is_empty() {
                map.retain(|(k, _)| k != &kebab);
            } else if let Some(pos) = map.iter().position(|(k, _)| k == &kebab) {
                map[pos].1 = val;
            } else {
                map.push((kebab, val));
            }
            st.doc.set_attribute(nid, "style", serialize_inline_map(&map));
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(css_decl_proto, "setProperty", JsValue::Object(set_prop));

    let d2 = dom.clone();
    let get_prop = rt.create_native_function(
        "getPropertyValue",
        1,
        Rc::new(move |rt, this, args| {
            let Some(oid) = this.as_object() else {
                return Ok(JsValue::String(String::new()));
            };
            let Some(nid) = extract_style_node(rt, oid) else {
                return Ok(JsValue::String(String::new()));
            };
            let name = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let kebab = camel_to_kebab(&name);
            let val = d2
                .borrow()
                .doc
                .element(nid)
                .and_then(|e| e.get_attr("style"))
                .map(|s| {
                    parse_inline_map(s)
                        .into_iter()
                        .find(|(k, _)| k == &kebab)
                        .map(|(_, v)| v)
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            Ok(JsValue::String(val))
        }),
    );
    let _ = rt.set_property(css_decl_proto, "getPropertyValue", JsValue::Object(get_prop));

    let d3 = dom.clone();
    let rem_prop = rt.create_native_function(
        "removeProperty",
        1,
        Rc::new(move |rt, this, args| {
            let Some(oid) = this.as_object() else {
                return Ok(JsValue::String(String::new()));
            };
            let Some(nid) = extract_style_node(rt, oid) else {
                return Ok(JsValue::String(String::new()));
            };
            let name = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let kebab = camel_to_kebab(&name);
            let mut st = d3.borrow_mut();
            let existing = st
                .doc
                .element(nid)
                .and_then(|e| e.get_attr("style"))
                .unwrap_or("")
                .to_string();
            let mut map = parse_inline_map(&existing);
            let mut old = String::new();
            if let Some(pos) = map.iter().position(|(k, _)| k == &kebab) {
                old = map.remove(pos).1;
            }
            st.doc.set_attribute(nid, "style", serialize_inline_map(&map));
            st.mark_dirty();
            Ok(JsValue::String(old))
        }),
    );
    let _ = rt.set_property(css_decl_proto, "removeProperty", JsValue::Object(rem_prop));

    let d_add = dom.clone();
    let cl_add = rt.create_native_function(
        "add",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = this.as_object().and_then(|o| rt.get_host_data::<ClassListHandle>(o)).map(|h| h.0) else {
                return Ok(JsValue::Undefined);
            };
            let mut to_add = Vec::new();
            for a in args {
                to_add.push(rt.to_string(a)?);
            }
            let mut st = d_add.borrow_mut();
            let mut list: Vec<String> = st
                .doc
                .element(nid)
                .and_then(|e| e.get_attr("class"))
                .map(|c| c.split_ascii_whitespace().map(|s| s.to_string()).collect())
                .unwrap_or_default();
            for tok in to_add {
                if !tok.is_empty() && !list.iter().any(|x| x == &tok) {
                    list.push(tok);
                }
            }
            st.doc.set_attribute(nid, "class", list.join(" "));
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(class_list_proto, "add", JsValue::Object(cl_add));

    let d_rem = dom.clone();
    let cl_rem = rt.create_native_function(
        "remove",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = this.as_object().and_then(|o| rt.get_host_data::<ClassListHandle>(o)).map(|h| h.0) else {
                return Ok(JsValue::Undefined);
            };
            let mut to_rem = Vec::new();
            for a in args {
                to_rem.push(rt.to_string(a)?);
            }
            let mut st = d_rem.borrow_mut();
            let mut list: Vec<String> = st
                .doc
                .element(nid)
                .and_then(|e| e.get_attr("class"))
                .map(|c| c.split_ascii_whitespace().map(|s| s.to_string()).collect())
                .unwrap_or_default();
            list.retain(|x| !to_rem.iter().any(|r| r == x));
            st.doc.set_attribute(nid, "class", list.join(" "));
            st.mark_dirty();
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(class_list_proto, "remove", JsValue::Object(cl_rem));

    let d_con = dom.clone();
    let cl_con = rt.create_native_function(
        "contains",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = this.as_object().and_then(|o| rt.get_host_data::<ClassListHandle>(o)).map(|h| h.0) else {
                return Ok(JsValue::Boolean(false));
            };
            let tok = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let has = d_con
                .borrow()
                .doc
                .element(nid)
                .map_or(false, |e| e.has_class(&tok));
            Ok(JsValue::Boolean(has))
        }),
    );
    let _ = rt.set_property(class_list_proto, "contains", JsValue::Object(cl_con));

    let d_tog = dom.clone();
    let cl_tog = rt.create_native_function(
        "toggle",
        1,
        Rc::new(move |rt, this, args| {
            let Some(nid) = this.as_object().and_then(|o| rt.get_host_data::<ClassListHandle>(o)).map(|h| h.0) else {
                return Ok(JsValue::Boolean(false));
            };
            let tok = args.first().map(|v| rt.to_string(v)).transpose()?.unwrap_or_default();
            let force = args.get(1).and_then(|v| {
                if v.is_undefined() {
                    None
                } else {
                    Some(rt.to_boolean(v))
                }
            });
            let mut st = d_tog.borrow_mut();
            let mut list: Vec<String> = st
                .doc
                .element(nid)
                .and_then(|e| e.get_attr("class"))
                .map(|c| c.split_ascii_whitespace().map(|s| s.to_string()).collect())
                .unwrap_or_default();
            let has = list.iter().any(|x| x == &tok);
            let next_present = force.unwrap_or(!has);
            if next_present && !has {
                list.push(tok);
            } else if !next_present && has {
                list.retain(|x| x != &tok);
            }
            st.doc.set_attribute(nid, "class", list.join(" "));
            st.mark_dirty();
            Ok(JsValue::Boolean(next_present))
        }),
    );
    let _ = rt.set_property(class_list_proto, "toggle", JsValue::Object(cl_tog));

    let d_len = dom.clone();
    let cl_len = rt.create_native_function(
        "get length",
        0,
        Rc::new(move |rt, this, _| {
            let Some(nid) = this.as_object().and_then(|o| rt.get_host_data::<ClassListHandle>(o)).map(|h| h.0) else {
                return Ok(JsValue::Number(0.0));
            };
            let len = d_len
                .borrow()
                .doc
                .element(nid)
                .map_or(0, |e| e.class_list().len());
            Ok(JsValue::Number(len as f64))
        }),
    );
    define_getter_setter(rt, class_list_proto, "length", Some(cl_len), None);
}

fn extract_style_node(rt: &JsRuntime, oid: ObjectId) -> Option<NodeId> {
    if let Some(h) = rt.get_host_data::<StyleDeclHandle>(oid) {
        return Some(h.0);
    }
    if let peedle_js::ObjectKind::Proxy(ref p) = rt.heap.get_obj(oid).kind {
        if !p.revoked {
            return rt.get_host_data::<StyleDeclHandle>(p.target).map(|h| h.0);
        }
    }
    None
}
