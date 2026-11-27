use std::cell::RefCell;
use std::rc::Rc;

use peedle_html::{Document, DomMutation, NodeId};
use peedle_js::{JsError, JsRuntime, JsValue, ObjectId};
use peedle_net::HttpClient;
use peedle_url::Url;

use crate::canvas_binding::define_canvas2d_proto;
use crate::document::define_document_proto;
use crate::events::{build_event_object, define_event_system, dispatch_event_object};
use crate::nodes::{define_element_proto, define_node_proto, wrap_node};
use crate::state::{DomPrototypes, DomState, SharedDom};
use crate::style_binding::define_style_and_classlist_protos;
use crate::traversal::{define_dom_exception, define_traversal_protos};
use crate::web_apis::define_web_apis;

pub struct BrowserJsEnv {
    pub rt: JsRuntime,
    pub dom: SharedDom,
}

impl BrowserJsEnv {
    pub fn new(
        doc: Document,
        url: String,
        viewport_w: f32,
        viewport_h: f32,
        client: HttpClient,
    ) -> Self {
        let mut rt = JsRuntime::new();
        let dom = Rc::new(RefCell::new(DomState::new(
            doc, url, viewport_w, viewport_h, client,
        )));

        let obj_proto = rt.intrinsics.object_proto;
        let event_target = rt.create_object_with_proto(Some(obj_proto));
        let node = rt.create_object_with_proto(Some(event_target));
        let element = rt.create_object_with_proto(Some(node));
        let html_element = rt.create_object_with_proto(Some(element));
        let svg_element = rt.create_object_with_proto(Some(element));
        let document = rt.create_object_with_proto(Some(node));
        let character_data = rt.create_object_with_proto(Some(node));
        let text = rt.create_object_with_proto(Some(character_data));
        let comment = rt.create_object_with_proto(Some(character_data));
        let fragment = rt.create_object_with_proto(Some(node));
        let shadow_root = rt.create_object_with_proto(Some(fragment));
        let doctype = rt.create_object_with_proto(Some(node));
        let event = rt.create_object_with_proto(Some(obj_proto));
        let css_style_decl = rt.create_object_with_proto(Some(obj_proto));
        let dom_token_list = rt.create_object_with_proto(Some(obj_proto));
        let canvas2d = rt.create_object_with_proto(Some(obj_proto));
        let url_proto = rt.create_object_with_proto(Some(obj_proto));
        let url_search_params_proto = rt.create_object_with_proto(Some(obj_proto));
        let headers_proto = rt.create_object_with_proto(Some(obj_proto));
        let response_proto = rt.create_object_with_proto(Some(obj_proto));
        let range_proto = rt.create_object_with_proto(Some(obj_proto));
        let dom_exception_proto = rt.create_object_with_proto(Some(obj_proto));
        let node_iterator_proto = rt.create_object_with_proto(Some(obj_proto));
        let tree_walker_proto = rt.create_object_with_proto(Some(obj_proto));

        dom.borrow_mut().protos = Some(DomPrototypes {
            event_target,
            node,
            element,
            html_element,
            svg_element,
            document,
            character_data,
            text,
            comment,
            fragment,
            shadow_root,
            doctype,
            event,
            css_style_decl,
            dom_token_list,
            canvas2d,
            url_proto,
            url_search_params_proto,
            headers_proto,
            response_proto,
            range_proto,
            dom_exception_proto,
            node_iterator_proto,
            tree_walker_proto,
        });

        let global = rt.global_object();
        let _ = rt.set_prototype_of(global, Some(event_target));

        define_dom_exception(&mut rt, dom_exception_proto);
        define_event_system(&mut rt, dom.clone(), event_target, event);
        define_style_and_classlist_protos(&mut rt, dom.clone(), css_style_decl, dom_token_list);
        define_canvas2d_proto(&mut rt, dom.clone(), canvas2d);
        define_node_proto(&mut rt, dom.clone(), node);
        define_element_proto(&mut rt, dom.clone(), element, html_element);
        define_traversal_protos(&mut rt, dom.clone(), node_iterator_proto, tree_walker_proto);
        define_document_proto(
            &mut rt,
            dom.clone(),
            document,
            text,
            comment,
            fragment,
            doctype,
            range_proto,
        );
        define_web_apis(&mut rt, dom.clone());

        for (ctor_name, proto) in [
            ("Node", node),
            ("Element", element),
            ("HTMLElement", html_element),
            ("SVGElement", svg_element),
            ("Document", document),
            ("HTMLDocument", document),
            ("CharacterData", character_data),
            ("Text", text),
            ("Comment", comment),
            ("DocumentFragment", fragment),
            ("ShadowRoot", shadow_root),
            ("DocumentType", doctype),
            ("CSSStyleDeclaration", css_style_decl),
            ("DOMTokenList", dom_token_list),
            ("CanvasRenderingContext2D", canvas2d),
            ("Range", range_proto),
            ("NodeIterator", node_iterator_proto),
            ("TreeWalker", tree_walker_proto),
        ] {
            let ctor = rt.create_native_constructor(
                ctor_name,
                0,
                proto,
                Rc::new(move |rt, _, _| Ok(JsValue::Object(rt.create_object_with_proto(Some(proto))))),
            );
            if ctor_name == "Node" {
                for (c_name, code) in [
                    ("ELEMENT_NODE", 1.0),
                    ("TEXT_NODE", 3.0),
                    ("PROCESSING_INSTRUCTION_NODE", 7.0),
                    ("COMMENT_NODE", 8.0),
                    ("DOCUMENT_NODE", 9.0),
                    ("DOCUMENT_TYPE_NODE", 10.0),
                    ("DOCUMENT_FRAGMENT_NODE", 11.0),
                ] {
                    let _ = rt.set_property(ctor, c_name, JsValue::Number(code));
                }
            }
            if ctor_name == "Range" {
                for (c_name, code) in [
                    ("START_TO_START", 0.0),
                    ("START_TO_END", 1.0),
                    ("END_TO_END", 2.0),
                    ("END_TO_START", 3.0),
                ] {
                    let _ = rt.set_property(ctor, c_name, JsValue::Number(code));
                }
            }
            let _ = rt.set_property(global, ctor_name, JsValue::Object(ctor));
        }

        for html_sub in [
            "HTMLDivElement",
            "HTMLSpanElement",
            "HTMLInputElement",
            "HTMLTextAreaElement",
            "HTMLButtonElement",
            "HTMLSelectElement",
            "HTMLOptionElement",
            "HTMLFormElement",
            "HTMLAnchorElement",
            "HTMLImageElement",
            "HTMLCanvasElement",
            "HTMLTemplateElement",
            "HTMLScriptElement",
            "HTMLStyleElement",
            "HTMLIFrameElement",
        ] {
            let sub_proto = rt.create_object_with_proto(Some(html_element));
            let ctor = rt.create_native_constructor(
                html_sub,
                0,
                sub_proto,
                Rc::new(move |rt, _, _| {
                    Ok(JsValue::Object(rt.create_object_with_proto(Some(sub_proto))))
                }),
            );
            let _ = rt.set_property(global, html_sub, JsValue::Object(ctor));
        }

        let root_id = dom.borrow().doc.root;
        let doc_obj = wrap_node(&mut rt, &dom, root_id);
        dom.borrow_mut().document_obj = Some(doc_obj);
        let _ = rt.set_property(global, "document", JsValue::Object(doc_obj));

        let dom_for_gc = dom.clone();
        rt.add_gc_root_tracer(move |roots| {
            if let Ok(st) = dom_for_gc.try_borrow() {
                st.trace_js_roots(roots);
            }
        });

        Self { rt, dom }
    }

    pub fn eval(&mut self, source: &str) -> Result<JsValue, JsError> {
        let res = self.rt.eval(source);
        self.tick_event_loop(64);
        res
    }

    pub fn run_document_scripts(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        let script_nodes: Vec<NodeId> = {
            let st = self.dom.borrow();
            st.doc
                .descendants(st.doc.root)
                .into_iter()
                .filter(|&id| {
                    st.doc
                        .element(id)
                        .map_or(false, |e| e.is_html() && e.tag_is("script"))
                })
                .collect()
        };

        for sid in script_nodes {
            let (script_type, src_attr, inline_code) = {
                let st = self.dom.borrow();
                let Some(el) = st.doc.element(sid) else {
                    continue;
                };
                let t = el
                    .get_attr("type")
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                let src = el.get_attr("src").map(|s| s.to_string());
                let code = st.doc.text_content(sid);
                (t, src, code)
            };

            let is_module = script_type == "module";
            let is_js = script_type.is_empty()
                || is_module
                || script_type.contains("javascript")
                || script_type.contains("ecmascript");
            if !is_js {
                continue;
            }

            let code = if let Some(ref src) = src_attr {
                let resolved = {
                    let st = self.dom.borrow();
                    Url::parse(&st.url)
                        .ok()
                        .and_then(|b| b.join(src).ok())
                        .or_else(|| Url::parse(src).ok())
                        .map(|u| u.href())
                };
                if let Some(u) = resolved {
                    let client = self.dom.borrow().client.clone();
                    match client.get(&u) {
                        Ok(resp) if resp.status < 400 => resp.text(),
                        _ => continue,
                    }
                } else {
                    continue;
                }
            } else {
                inline_code
            };

            if code.trim().is_empty() {
                continue;
            }

            self.dom.borrow_mut().current_script = Some(sid);
            let res = if is_module {
                let spec = src_attr.unwrap_or_else(|| format!("inline:{}", sid.0));
                self.rt.eval_module(&spec, &code)
            } else {
                self.rt.eval(&code)
            };
            self.dom.borrow_mut().current_script = None;
            if let Err(e) = res {
                let msg = e.to_string();
                self.rt.console_lines.push(format!("[Error] {}", msg));
                errors.push(msg);
            }
            self.tick_event_loop(64);
        }

        self.dom.borrow_mut().ready_state = "interactive".to_string();
        let root_id = self.dom.borrow().doc.root;
        let _ = self.dispatch_dom_event(Some(root_id), "DOMContentLoaded", true, false);

        let embedded_nodes: Vec<NodeId> = {
            let st = self.dom.borrow();
            st.doc
                .descendants(st.doc.root)
                .into_iter()
                .filter(|&id| {
                    st.doc.element(id).map_or(false, |e| {
                        e.is_html() && (e.tag_is("iframe") || e.tag_is("object"))
                    })
                })
                .collect()
        };
        self.dom.borrow_mut().pending_subdoc_loads.clear();
        for eid in embedded_nodes {
            self.load_subdocument_and_fire_load(eid);
        }

        self.dom.borrow_mut().ready_state = "complete".to_string();
        let body_onload = {
            let st = self.dom.borrow();
            st.doc
                .body()
                .and_then(|bid| st.doc.element(bid))
                .and_then(|e| e.get_attr("onload"))
                .map(|s| s.to_string())
        };
        let _ = self.dispatch_dom_event(None, "load", false, false);
        if let Some(code) = body_onload {
            if let Err(e) = self.rt.eval(&code) {
                let msg = e.to_string();
                self.rt.console_lines.push(format!("[Error] {}", msg));
                errors.push(msg);
            }
        }
        self.tick_event_loop(64);
        errors
    }

    pub fn dispatch_dom_event(
        &mut self,
        target: Option<NodeId>,
        event_type: &str,
        bubbles: bool,
        cancelable: bool,
    ) -> Result<bool, JsError> {
        let ev_proto = self.dom.borrow().protos.unwrap().event;
        let ev = build_event_object(
            &mut self.rt,
            ev_proto,
            event_type.to_string(),
            bubbles,
            cancelable,
            true,
        );
        let ok = dispatch_event_object(&mut self.rt, &self.dom, target, ev)?;
        self.tick_event_loop(64);
        Ok(ok)
    }

    fn load_subdocument_and_fire_load(&mut self, eid: NodeId) {
        let scripts = crate::nodes::load_subdocument_with_scripts(&self.dom, eid);
        for code in scripts {
            let _ = self.rt.eval(&code);
            self.rt.drain_microtasks();
        }
        let ev_proto = self.dom.borrow().protos.unwrap().event;
        let ev = build_event_object(
            &mut self.rt,
            ev_proto,
            "load".to_string(),
            false,
            false,
            true,
        );
        let _ = dispatch_event_object(&mut self.rt, &self.dom, Some(eid), ev);
        self.rt.drain_microtasks();
    }

    pub fn tick_event_loop(&mut self, max_turns: usize) {
        for _ in 0..max_turns {
            self.rt.drain_microtasks();
            self.flush_mutation_observers();

            let pending_loads: Vec<NodeId> = {
                let mut st = self.dom.borrow_mut();
                std::mem::take(&mut st.pending_subdoc_loads)
            };
            let mut did_work = !pending_loads.is_empty();
            for eid in pending_loads {
                let connected = {
                    let st = self.dom.borrow();
                    st.doc.is_ancestor(st.doc.root, eid)
                };
                if connected {
                    self.load_subdocument_and_fire_load(eid);
                }
            }

            let messages: Vec<(JsValue, JsValue)> = {
                let mut st = self.dom.borrow_mut();
                std::mem::take(&mut st.message_queue)
            };
            if !messages.is_empty() {
                did_work = true;
            }
            for (port, ev) in messages {
                if let Some(p_oid) = port.as_object() {
                    if let Ok(handler) = self.rt.get_property(p_oid, "onmessage") {
                        if self.rt.is_callable(&handler) {
                            let _ = self.rt.call_function(handler, port, &[ev]);
                        }
                    }
                }
                self.rt.drain_microtasks();
            }

            let due_timers = self.pop_due_timers();
            if !due_timers.is_empty() {
                did_work = true;
                let global = JsValue::Object(self.rt.global_object());
                for t in due_timers {
                    if self.rt.is_callable(&t.callback) {
                        let _ = self.rt.call_function(t.callback, global.clone(), &t.args);
                    } else if let Some(code) = t.callback.as_str() {
                        let _ = self.rt.eval(code);
                    }
                    self.rt.drain_microtasks();
                }
            }

            self.flush_mutation_observers();
            if !did_work && self.rt.microtasks.is_empty() {
                break;
            }
        }
    }

    pub fn advance_time_ms(&mut self, delta_ms: u64) {
        self.dom.borrow_mut().now_ms += delta_ms;
        self.tick_event_loop(64);
    }

    fn pop_due_timers(&self) -> Vec<crate::state::TimerEntry> {
        let mut st = self.dom.borrow_mut();
        let now = st.now_ms;
        let mut ready = Vec::new();
        let mut remaining = Vec::new();
        for mut t in st.timers.drain(..) {
            if t.cancelled {
                continue;
            }
            if t.due_ms <= now {
                ready.push(t.clone());
                if t.repeat {
                    t.due_ms = now + t.delay_ms.max(4);
                    remaining.push(t);
                }
            } else {
                remaining.push(t);
            }
        }
        st.timers = remaining;
        ready
    }

    fn flush_mutation_observers(&mut self) {
        let mutations: Vec<DomMutation> = {
            let mut st = self.dom.borrow_mut();
            if st.doc.mutations.is_empty() || st.mutation_observers.is_empty() {
                st.doc.mutations.clear();
                return;
            }
            std::mem::take(&mut st.doc.mutations)
        };

        let observers = self.dom.borrow().mutation_observers.clone();
        for obs in observers {
            if obs.targets.is_empty() || !self.rt.is_callable(&obs.callback) {
                continue;
            }
            let mut records: Vec<JsValue> = Vec::new();
            for m in &mutations {
                let (m_target, m_kind) = match m {
                    DomMutation::ChildList { target, .. } => (*target, 0u8),
                    DomMutation::Attribute { target, .. } => (*target, 1u8),
                    DomMutation::CharacterData { target, .. } => (*target, 2u8),
                };
                let matched = obs.targets.iter().any(|t| {
                    let kind_ok = match m_kind {
                        0 => t.child_list,
                        1 => t.attributes,
                        _ => t.character_data,
                    };
                    if !kind_ok {
                        return false;
                    }
                    if t.node == m_target {
                        true
                    } else if t.subtree {
                        self.dom.borrow().doc.is_ancestor(t.node, m_target)
                    } else {
                        false
                    }
                });
                if !matched {
                    continue;
                }
                let rec = self.build_mutation_record(m);
                records.push(JsValue::Object(rec));
            }
            if !records.is_empty() {
                let arr = self.rt.create_array(records);
                let _ = self
                    .rt
                    .call_function(obs.callback, JsValue::Undefined, &[JsValue::Object(arr)]);
            }
        }
    }

    fn build_mutation_record(&mut self, m: &DomMutation) -> ObjectId {
        let obj = self.rt.create_object();
        match m {
            DomMutation::ChildList {
                target,
                added,
                removed,
                ..
            } => {
                let _ = self.rt.set_property(obj, "type", JsValue::String("childList".into()));
                let t_obj = wrap_node(&mut self.rt, &self.dom, *target);
                let _ = self.rt.set_property(obj, "target", JsValue::Object(t_obj));
                let add_vals: Vec<JsValue> = added
                    .iter()
                    .map(|&id| JsValue::Object(wrap_node(&mut self.rt, &self.dom, id)))
                    .collect();
                let rem_vals: Vec<JsValue> = removed
                    .iter()
                    .map(|&id| JsValue::Object(wrap_node(&mut self.rt, &self.dom, id)))
                    .collect();
                let add_arr = self.rt.create_array(add_vals);
                let rem_arr = self.rt.create_array(rem_vals);
                let _ = self.rt.set_property(obj, "addedNodes", JsValue::Object(add_arr));
                let _ = self.rt.set_property(obj, "removedNodes", JsValue::Object(rem_arr));
            }
            DomMutation::Attribute {
                target,
                name,
                old_value,
            } => {
                let _ = self.rt.set_property(obj, "type", JsValue::String("attributes".into()));
                let t_obj = wrap_node(&mut self.rt, &self.dom, *target);
                let _ = self.rt.set_property(obj, "target", JsValue::Object(t_obj));
                let _ = self.rt.set_property(obj, "attributeName", JsValue::String(name.clone()));
                let ov = old_value
                    .as_ref()
                    .map(|s| JsValue::String(s.clone()))
                    .unwrap_or(JsValue::Null);
                let _ = self.rt.set_property(obj, "oldValue", ov);
            }
            DomMutation::CharacterData { target, old_value } => {
                let _ = self
                    .rt
                    .set_property(obj, "type", JsValue::String("characterData".into()));
                let t_obj = wrap_node(&mut self.rt, &self.dom, *target);
                let _ = self.rt.set_property(obj, "target", JsValue::Object(t_obj));
                let _ = self
                    .rt
                    .set_property(obj, "oldValue", JsValue::String(old_value.clone()));
            }
        }
        obj
    }
}
