use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use peedle_css::{compute_styles, parse_stylesheet, StyleMap, Stylesheet, Viewport};
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
pub struct MutationObserverEntry {
    pub callback: JsValue,
    pub targets: Vec<MutationTarget>,
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

pub struct DomState {
    pub doc: Document,
    pub url: String,
    pub external_stylesheets: HashMap<NodeId, Stylesheet>,
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
    pub xml_documents: std::collections::HashSet<NodeId>,
    pub xml_elements: std::collections::HashSet<NodeId>,
}

pub type SharedDom = Rc<RefCell<DomState>>;

impl DomState {
    pub fn new(doc: Document, url: String, viewport_w: f32, viewport_h: f32, client: HttpClient) -> Self {
        let mut st = Self {
            doc,
            url: url.clone(),
            external_stylesheets: HashMap::new(),
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
            xml_documents: std::collections::HashSet::new(),
            xml_elements: std::collections::HashSet::new(),
        };
        st.doc.track_mutations = true;
        st
    }

    pub fn mark_dirty(&mut self) {
        self.dirty_style = true;
        self.dirty_layout = true;
    }

    pub fn collect_inline_stylesheets(&mut self) {
        self.stylesheets.clear();
        let nodes = self.doc.descendants(self.doc.root);
        for nid in nodes {
            if let Some(ext) = self.external_stylesheets.get(&nid) {
                self.stylesheets.push(ext.clone());
                continue;
            }
            if let Some(el) = self.doc.element(nid) {
                if el.is_html() && el.tag_is("style") {
                    let css = self.doc.text_content(nid);
                    self.stylesheets.push(parse_stylesheet(&css));
                }
            }
        }
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
            self.layout = layout_document(
                &self.doc,
                &self.styles,
                &self.image_sizes,
                self.viewport_w,
                self.viewport_h,
            );
            self.dirty_layout = false;
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
