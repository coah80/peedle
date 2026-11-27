use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use peedle_css::{
    compute_styles, parse_stylesheet, CalcContext, StyleMap, Stylesheet, Viewport,
};
use peedle_html::{Document, NodeId};
use peedle_js::{JsValue, ObjectId};
use peedle_layout::{layout_document, LayoutTree};
use peedle_net::{BlobStore, HttpClient};
use peedle_render::Canvas2D;

#[derive(Clone, Debug)]
pub struct EventListenerEntry {
    pub event_type: String,
    pub callback: JsValue,
    pub capture: bool,
    pub once: bool,
    pub passive: bool,
}

#[derive(Clone, Debug)]
pub struct TimerEntry {
    pub id: u32,
    pub callback: JsValue,
    pub args: Vec<JsValue>,
    pub delay_ms: u64,
    pub due_ms: u64,
    pub repeat: bool,
    pub cancelled: bool,
    pub is_raf: bool,
}

#[derive(Clone, Debug)]
pub struct MutationRecordData {
    pub kind: &'static str,
    pub target: NodeId,
    pub attribute_name: Option<String>,
    pub old_value: Option<String>,
}

#[derive(Clone, Debug)]
pub struct MutationObserverEntry {
    pub callback: JsValue,
    pub targets: Vec<MutationTarget>,
    pub records: Vec<MutationRecordData>,
}

#[derive(Clone, Debug)]
pub struct MutationTarget {
    pub node: NodeId,
    pub child_list: bool,
    pub attributes: bool,
    pub character_data: bool,
    pub subtree: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct DomPrototypes {
    pub event_target: ObjectId,
    pub node: ObjectId,
    pub element: ObjectId,
    pub html_element: ObjectId,
    pub svg_element: ObjectId,
    pub document: ObjectId,
    pub character_data: ObjectId,
    pub text: ObjectId,
    pub comment: ObjectId,
    pub fragment: ObjectId,
    pub shadow_root: ObjectId,
    pub doctype: ObjectId,
    pub event: ObjectId,
    pub css_style_decl: ObjectId,
    pub dom_token_list: ObjectId,
    pub canvas2d: ObjectId,
    pub url_proto: ObjectId,
    pub url_search_params_proto: ObjectId,
    pub headers_proto: ObjectId,
    pub response_proto: ObjectId,
    pub range_proto: ObjectId,
    pub dom_exception_proto: ObjectId,
    pub node_iterator_proto: ObjectId,
    pub tree_walker_proto: ObjectId,
}

pub struct LiveNodeIterator {
    pub root: NodeId,
    pub what_to_show: u32,
    pub filter: JsValue,
    pub reference: NodeId,
    pub pointer_before: bool,
    pub detached: bool,
}

#[derive(Clone)]
pub struct NodeIteratorHandle(pub Rc<RefCell<LiveNodeIterator>>);

#[derive(Clone)]
pub struct RangeHandle(pub Rc<RefCell<RangeData>>);

pub struct DomState {
    pub doc: Document,
    pub url: String,
    pub external_stylesheets: HashMap<NodeId, Stylesheet>,
    pub cssom_stylesheets: HashMap<NodeId, Stylesheet>,
    pub stylesheets: Vec<Stylesheet>,
    pub styles: StyleMap,
    pub layout: LayoutTree,
    pub image_sizes: HashMap<String, (u32, u32)>,
    pub canvases: HashMap<NodeId, Canvas2D>,
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub dark_mode: bool,
    pub dirty_style: bool,
    pub dirty_layout: bool,
    pub ready_state: String,
    pub node_wrappers: HashMap<NodeId, ObjectId>,
    pub document_obj: Option<ObjectId>,
    pub listeners: HashMap<NodeId, Vec<EventListenerEntry>>,
    pub window_listeners: Vec<EventListenerEntry>,
    pub timers: Vec<TimerEntry>,
    pub next_timer_id: u32,
    pub now_ms: u64,
    pub message_queue: Vec<(JsValue, JsValue)>,
    pub mutation_observers: Vec<MutationObserverEntry>,
    pub custom_elements: HashMap<String, ObjectId>,
    pub local_storage: HashMap<String, String>,
    pub session_storage: HashMap<String, String>,
    pub history_stack: Vec<(String, JsValue)>,
    pub history_index: usize,
    pub client: HttpClient,
    pub blobs: BlobStore,
    pub protos: Option<DomPrototypes>,
    pub focused_node: Option<NodeId>,
    pub pending_navigation: Option<String>,
    pub sub_documents: HashMap<NodeId, NodeId>,
    pub subdoc_urls: HashMap<NodeId, String>,
    pub pending_subdoc_loads: Vec<NodeId>,
    pub xml_documents: HashSet<NodeId>,
    pub xml_elements: HashSet<NodeId>,
    pub custom_namespaces: HashMap<NodeId, Option<String>>,
    pub current_script: Option<NodeId>,
    pub open_doc_buffers: HashMap<NodeId, String>,
    pub title_overrides: HashMap<NodeId, String>,
    pub live_iterators: Vec<Rc<RefCell<LiveNodeIterator>>>,
    pub live_ranges: Vec<Rc<RefCell<RangeData>>>,
}

pub type SharedDom = Rc<RefCell<DomState>>;

impl DomState {
    pub fn new(doc: Document, url: String, viewport_w: f32, viewport_h: f32, client: HttpClient) -> Self {
        let mut st = Self {
            doc,
            url: url.clone(),
            external_stylesheets: HashMap::new(),
            cssom_stylesheets: HashMap::new(),
            stylesheets: Vec::new(),
            styles: StyleMap::default(),
            layout: LayoutTree::default(),
            image_sizes: HashMap::new(),
            canvases: HashMap::new(),
            viewport_w,
            viewport_h,
            scroll_x: 0.0,
            scroll_y: 0.0,
            dark_mode: false,
            dirty_style: true,
            dirty_layout: true,
            ready_state: "loading".to_string(),
            node_wrappers: HashMap::new(),
            document_obj: None,
            listeners: HashMap::new(),
            window_listeners: Vec::new(),
            timers: Vec::new(),
            next_timer_id: 1,
            now_ms: 0,
            message_queue: Vec::new(),
            mutation_observers: Vec::new(),
            custom_elements: HashMap::new(),
            local_storage: HashMap::new(),
            session_storage: HashMap::new(),
            history_stack: vec![(url, JsValue::Null)],
            history_index: 0,
            client,
            blobs: BlobStore::new(),
            protos: None,
            focused_node: None,
            pending_navigation: None,
            sub_documents: HashMap::new(),
            subdoc_urls: HashMap::new(),
            pending_subdoc_loads: Vec::new(),
            xml_documents: HashSet::new(),
            xml_elements: HashSet::new(),
            custom_namespaces: HashMap::new(),
            current_script: None,
            open_doc_buffers: HashMap::new(),
            title_overrides: HashMap::new(),
            live_iterators: Vec::new(),
            live_ranges: Vec::new(),
        };
        st.doc.track_mutations = true;
        st
    }

    pub fn trace_js_roots(&self, roots: &mut Vec<ObjectId>) {
        fn push_val(v: &JsValue, roots: &mut Vec<ObjectId>) {
            if let JsValue::Object(oid) = v {
                roots.push(*oid);
            }
        }
        if let Some(p) = self.protos {
            roots.extend_from_slice(&[
                p.event_target,
                p.node,
                p.element,
                p.html_element,
                p.svg_element,
                p.document,
                p.character_data,
                p.text,
                p.comment,
                p.fragment,
                p.shadow_root,
                p.doctype,
                p.event,
                p.css_style_decl,
                p.dom_token_list,
                p.canvas2d,
                p.url_proto,
                p.url_search_params_proto,
                p.headers_proto,
                p.response_proto,
                p.range_proto,
                p.dom_exception_proto,
                p.node_iterator_proto,
                p.tree_walker_proto,
            ]);
        }
        if let Some(doc_obj) = self.document_obj {
            roots.push(doc_obj);
        }
        for &oid in self.node_wrappers.values() {
            roots.push(oid);
        }
        for list in self.listeners.values() {
            for entry in list {
                push_val(&entry.callback, roots);
            }
        }
        for entry in &self.window_listeners {
            push_val(&entry.callback, roots);
        }
        for timer in &self.timers {
            push_val(&timer.callback, roots);
            for arg in &timer.args {
                push_val(arg, roots);
            }
        }
        for (data, origin) in &self.message_queue {
            push_val(data, roots);
            push_val(origin, roots);
        }
        for obs in &self.mutation_observers {
            push_val(&obs.callback, roots);
        }
        for &oid in self.custom_elements.values() {
            roots.push(oid);
        }
        for (_, state) in &self.history_stack {
            push_val(state, roots);
        }
        for iter in &self.live_iterators {
            if let Ok(it) = iter.try_borrow() {
                push_val(&it.filter, roots);
            }
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty_style = true;
        self.dirty_layout = true;
    }

    pub fn mark_dirty_for(&mut self, nid: NodeId) {
        let mut cur = nid;
        while let Some(p) = self.doc.node(cur).parent {
            cur = p;
        }
        if cur == self.doc.root {
            self.dirty_style = true;
            self.dirty_layout = true;
        }
    }

    pub fn ensure_styles(&mut self) {
        if self.dirty_style {
            self.collect_inline_stylesheets();
            self.styles = compute_styles(
                &self.doc,
                &self.stylesheets,
                Viewport {
                    width: self.viewport_w,
                    height: self.viewport_h,
                    dark_mode: self.dark_mode,
                },
            );
            self.dirty_style = false;
            self.dirty_layout = true;
        }
    }

    pub fn invalidate_style_owner(&mut self, nid: NodeId) {
        let mut cur = Some(nid);
        while let Some(id) = cur {
            if self.cssom_stylesheets.contains_key(&id) {
                self.cssom_stylesheets.remove(&id);
            }
            cur = self.doc.node(id).parent;
        }
    }

    pub fn stylesheet_for_node(&self, nid: NodeId) -> Option<Stylesheet> {
        if let Some(sheet) = self.cssom_stylesheets.get(&nid) {
            return Some(sheet.clone());
        }
        if let Some(ext) = self.external_stylesheets.get(&nid) {
            return Some(ext.clone());
        }
        if let Some(el) = self.doc.element(nid) {
            if el.is_html() && el.tag_is("style") {
                let css = self.doc.text_content(nid);
                return Some(parse_stylesheet(&css));
            }
        }
        None
    }

    pub fn collect_stylesheets_for_root(&self, root: NodeId) -> Vec<Stylesheet> {
        let mut out = Vec::new();
        for nid in self.doc.descendants(root) {
            if let Some(sheet) = self.stylesheet_for_node(nid) {
                out.push(sheet);
            }
        }
        out
    }

    pub fn collect_inline_stylesheets(&mut self) {
        self.stylesheets = self.collect_stylesheets_for_root(self.doc.root);
    }

    pub fn ensure_layout(&mut self) {
        if self.dirty_style {
            self.collect_inline_stylesheets();
            self.styles = compute_styles(
                &self.doc,
                &self.stylesheets,
                Viewport {
                    width: self.viewport_w,
                    height: self.viewport_h,
                    dark_mode: self.dark_mode,
                },
            );
            self.dirty_style = false;
            self.dirty_layout = true;
        }
        if self.dirty_layout {
            let mut effective_sizes = self.image_sizes.clone();
            for &host_id in self.sub_documents.keys() {
                if let Some(el) = self.doc.element(host_id) {
                    if el.is_html() && el.tag_is("object") {
                        if let Some(d) = el.get_attr("data") {
                            effective_sizes.entry(d.to_string()).or_insert((0, 0));
                        }
                    }
                }
            }
            self.layout = layout_document(
                &self.doc,
                &self.styles,
                &effective_sizes,
                self.viewport_w,
                self.viewport_h,
            );
            self.dirty_layout = false;
        }
    }

    pub fn resolve_iframe_viewport(&mut self, sub_root: NodeId) -> Viewport {
        let host_id = self
            .sub_documents
            .iter()
            .find_map(|(&host, &sub)| if sub == sub_root { Some(host) } else { None });
        let Some(hid) = host_id else {
            return Viewport {
                width: self.viewport_w,
                height: self.viewport_h,
                dark_mode: self.dark_mode,
            };
        };
        self.ensure_styles();
        let ctx_w = CalcContext {
            font_size: 16.0,
            root_font_size: 16.0,
            viewport_width: self.viewport_w,
            viewport_height: self.viewport_h,
            percentage_base: self.viewport_w,
        };
        let ctx_h = CalcContext {
            font_size: 16.0,
            root_font_size: 16.0,
            viewport_width: self.viewport_w,
            viewport_height: self.viewport_h,
            percentage_base: self.viewport_h,
        };
        let (w, h) = if let Some(style) = self.styles.get(hid) {
            let sw = style.width.resolve(&ctx_w);
            let sh = style.height.resolve(&ctx_h);
            let attr_w = self
                .doc
                .element(hid)
                .and_then(|e| e.get_attr("width"))
                .and_then(|s| s.trim_end_matches("px").parse::<f32>().ok());
            let attr_h = self
                .doc
                .element(hid)
                .and_then(|e| e.get_attr("height"))
                .and_then(|s| s.trim_end_matches("px").parse::<f32>().ok());
            let w = sw.or(attr_w).unwrap_or(300.0);
            let h = sh.or(attr_h).unwrap_or(150.0);
            (w.max(0.0), h.max(0.0))
        } else {
            (300.0, 150.0)
        };
        Viewport {
            width: w,
            height: h,
            dark_mode: self.dark_mode,
        }
    }

    pub fn queue_subdoc_loads_in(&mut self, root: NodeId) {
        let connected = root == self.doc.root || self.doc.is_ancestor(self.doc.root, root);
        if !connected {
            return;
        }
        let mut candidates = vec![root];
        candidates.extend(self.doc.descendants(root));
        for cid in candidates {
            let is_embed = self.doc.element(cid).map_or(false, |e| {
                e.is_html() && (e.tag_is("iframe") || e.tag_is("object"))
            });
            if is_embed {
                self.sub_documents.remove(&cid);
                if !self.pending_subdoc_loads.contains(&cid) {
                    self.pending_subdoc_loads.push(cid);
                }
            }
        }
    }

    pub fn notify_pre_remove(&mut self, node: NodeId) {
        let Some(parent) = self.doc.node(node).parent else {
            return;
        };
        let index = self
            .doc
            .children(parent)
            .iter()
            .position(|&c| c == node)
            .unwrap_or(0);

        for it_rc in &self.live_iterators {
            let mut it = it_rc.borrow_mut();
            if it.detached || node == it.root {
                continue;
            }
            if node != it.reference && !self.doc.is_ancestor(node, it.reference) {
                continue;
            }
            if it.pointer_before {
                let mut cur = node;
                let mut next = None;
                loop {
                    if cur == it.root {
                        break;
                    }
                    if let Some(ns) = self.doc.node(cur).next_sibling {
                        next = Some(ns);
                        break;
                    }
                    match self.doc.node(cur).parent {
                        Some(p) => cur = p,
                        None => break,
                    }
                }
                if let Some(n) = next {
                    it.reference = n;
                    continue;
                }
                it.pointer_before = false;
            }
            if let Some(mut prev) = self.doc.node(node).prev_sibling {
                while let Some(lc) = self.doc.node(prev).last_child {
                    prev = lc;
                }
                it.reference = prev;
            } else {
                it.reference = parent;
            }
        }

        for r_rc in &self.live_ranges {
            let mut r = r_rc.borrow_mut();
            if r.start_container == node || self.doc.is_ancestor(node, r.start_container) {
                r.start_container = parent;
                r.start_offset = index;
            } else if r.start_container == parent && r.start_offset > index {
                r.start_offset -= 1;
            }
            if r.end_container == node || self.doc.is_ancestor(node, r.end_container) {
                r.end_container = parent;
                r.end_offset = index;
            } else if r.end_container == parent && r.end_offset > index {
                r.end_offset -= 1;
            }
        }
    }

    pub fn notify_post_insert(&mut self, parent: NodeId, index: usize, count: usize) {
        if count == 0 {
            return;
        }
        for r_rc in &self.live_ranges {
            let mut r = r_rc.borrow_mut();
            if r.start_container == parent && r.start_offset > index {
                r.start_offset += count;
            }
            if r.end_container == parent && r.end_offset > index {
                r.end_offset += count;
            }
        }
        self.record_mutation("childList", parent, None, None);
    }

    pub fn record_mutation(
        &mut self,
        kind: &'static str,
        target: NodeId,
        attribute_name: Option<String>,
        old_value: Option<String>,
    ) {
        if self.mutation_observers.is_empty() {
            return;
        }
        for obs in &mut self.mutation_observers {
            let mut matched = false;
            for t in &obs.targets {
                let kind_ok = match kind {
                    "childList" => t.child_list,
                    "attributes" => t.attributes,
                    "characterData" => t.character_data,
                    _ => false,
                };
                if !kind_ok {
                    continue;
                }
                if t.node == target || (t.subtree && self.doc.is_ancestor(t.node, target)) {
                    matched = true;
                    break;
                }
            }
            if matched {
                obs.records.push(MutationRecordData {
                    kind,
                    target,
                    attribute_name: attribute_name.clone(),
                    old_value: old_value.clone(),
                });
            }
        }
    }

    pub fn add_listener(&mut self, target: Option<NodeId>, entry: EventListenerEntry) {
        let list = match target {
            Some(nid) => self.listeners.entry(nid).or_default(),
            None => &mut self.window_listeners,
        };
        let dup = list.iter().any(|e| {
            e.event_type == entry.event_type
                && e.capture == entry.capture
                && e.callback == entry.callback
        });
        if !dup {
            list.push(entry);
        }
    }

    pub fn remove_listener(
        &mut self,
        target: Option<NodeId>,
        event_type: &str,
        callback: &JsValue,
        capture: bool,
    ) {
        let list = match target {
            Some(nid) => match self.listeners.get_mut(&nid) {
                Some(l) => l,
                None => return,
            },
            None => &mut self.window_listeners,
        };
        list.retain(|e| {
            !(e.event_type == event_type && e.capture == capture && &e.callback == callback)
        });
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NodeHandle(pub NodeId);

#[derive(Clone, Copy, Debug)]
pub struct StyleDeclHandle(pub NodeId);

#[derive(Clone, Copy, Debug)]
pub struct ClassListHandle(pub NodeId);

#[derive(Clone, Copy, Debug)]
pub struct Canvas2DHandle(pub NodeId);

#[derive(Clone, Debug)]
pub struct EventData {
    pub event_type: String,
    pub bubbles: bool,
    pub cancelable: bool,
    pub composed: bool,
    pub default_prevented: bool,
    pub propagation_stopped: bool,
    pub immediate_propagation_stopped: bool,
    pub event_phase: u8,
    pub target: JsValue,
    pub current_target: JsValue,
    pub path: Vec<JsValue>,
}

#[derive(Clone, Copy, Debug)]
pub struct RangeData {
    pub start_container: NodeId,
    pub start_offset: usize,
    pub end_container: NodeId,
    pub end_offset: usize,
}
