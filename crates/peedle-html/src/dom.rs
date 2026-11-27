#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Namespace {
    Html,
    Svg,
    MathMl,
}

impl Namespace {
    pub fn uri(self) -> &'static str {
        match self {
            Namespace::Html => "http://www.w3.org/1999/xhtml",
            Namespace::Svg => "http://www.w3.org/2000/svg",
            Namespace::MathMl => "http://www.w3.org/1998/Math/MathML",
        }
    }

    pub fn from_uri(uri: &str) -> Self {
        match uri {
            "http://www.w3.org/2000/svg" => Namespace::Svg,
            "http://www.w3.org/1998/Math/MathML" => Namespace::MathMl,
            _ => Namespace::Html,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuirksMode {
    NoQuirks,
    LimitedQuirks,
    Quirks,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowRootMode {
    Open,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub value: String,
    pub namespace: Option<String>,
    pub prefix: Option<String>,
}

impl Attribute {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            namespace: None,
            prefix: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ElementData {
    pub tag_name: String,
    pub namespace: Namespace,
    pub attributes: Vec<Attribute>,
    pub shadow_root: Option<NodeId>,
    pub template_contents: Option<NodeId>,
    pub input_value: Option<String>,
    pub checked: Option<bool>,
    pub selected_index: Option<usize>,
    pub scroll_left: f32,
    pub scroll_top: f32,
    pub hovered: bool,
    pub active: bool,
    pub focused: bool,
    pub visited: bool,
}

impl ElementData {
    pub fn new(tag_name: impl Into<String>, namespace: Namespace) -> Self {
        Self {
            tag_name: tag_name.into(),
            namespace,
            attributes: Vec::new(),
            shadow_root: None,
            template_contents: None,
            input_value: None,
            checked: None,
            selected_index: None,
            scroll_left: 0.0,
            scroll_top: 0.0,
            hovered: false,
            active: false,
            focused: false,
            visited: false,
        }
    }

    pub fn is_html(&self) -> bool {
        self.namespace == Namespace::Html
    }

    pub fn tag_is(&self, name: &str) -> bool {
        if self.namespace == Namespace::Html {
            self.tag_name.eq_ignore_ascii_case(name)
        } else {
            self.tag_name == name
        }
    }

    pub fn get_attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|a| {
                if self.namespace == Namespace::Html {
                    a.name.eq_ignore_ascii_case(name)
                } else {
                    a.name == name
                }
            })
            .map(|a| a.value.as_str())
    }

    pub fn has_attr(&self, name: &str) -> bool {
        self.get_attr(name).is_some()
    }

    pub fn set_attr(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();
        let normalized = if self.namespace == Namespace::Html {
            name.to_ascii_lowercase()
        } else {
            name
        };
        if let Some(existing) = self.attributes.iter_mut().find(|a| a.name == normalized) {
            existing.value = value;
        } else {
            self.attributes.push(Attribute::new(normalized, value));
        }
    }

    pub fn remove_attr(&mut self, name: &str) -> Option<String> {
        let pos = self.attributes.iter().position(|a| {
            if self.namespace == Namespace::Html {
                a.name.eq_ignore_ascii_case(name)
            } else {
                a.name == name
            }
        });
        pos.map(|i| self.attributes.remove(i).value)
    }

    pub fn id(&self) -> Option<&str> {
        self.get_attr("id").filter(|s| !s.is_empty())
    }

    pub fn class_list(&self) -> Vec<&str> {
        self.get_attr("class")
            .map(|c| c.split_ascii_whitespace().collect())
            .unwrap_or_default()
    }

    pub fn has_class(&self, cls: &str) -> bool {
        if cls.is_empty() {
            return false;
        }
        self.get_attr("class")
            .map_or(false, |c| c.split_ascii_whitespace().any(|t| t == cls))
    }

    pub fn current_value(&self) -> String {
        if let Some(ref v) = self.input_value {
            return v.clone();
        }
        self.get_attr("value").unwrap_or("").to_string()
    }

    pub fn is_checked(&self) -> bool {
        self.checked.unwrap_or_else(|| self.has_attr("checked"))
    }

    pub fn is_disabled(&self) -> bool {
        self.has_attr("disabled")
    }
}

#[derive(Clone, Debug)]
pub enum NodeData {
    Document { quirks_mode: QuirksMode },
    Doctype { name: String, public_id: String, system_id: String },
    Element(ElementData),
    Text(String),
    Comment(String),
    DocumentFragment,
    ShadowRoot { mode: ShadowRootMode, host: NodeId },
    ProcessingInstruction { target: String, data: String },
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub first_child: Option<NodeId>,
    pub last_child: Option<NodeId>,
    pub prev_sibling: Option<NodeId>,
    pub next_sibling: Option<NodeId>,
    pub data: NodeData,
}

#[derive(Clone, Debug)]
pub enum DomMutation {
    ChildList {
        target: NodeId,
        added: Vec<NodeId>,
        removed: Vec<NodeId>,
        prev_sibling: Option<NodeId>,
        next_sibling: Option<NodeId>,
    },
    Attribute {
        target: NodeId,
        name: String,
        old_value: Option<String>,
    },
    CharacterData {
        target: NodeId,
        old_value: String,
    },
}

#[derive(Clone, Debug)]
pub struct Document {
    pub nodes: Vec<Node>,
    pub root: NodeId,
    pub url: String,
    pub title_override: Option<String>,
    pub mutations: Vec<DomMutation>,
    pub track_mutations: bool,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    pub fn new() -> Self {
        let root = Node {
            id: NodeId(0),
            parent: None,
            first_child: None,
            last_child: None,
            prev_sibling: None,
            next_sibling: None,
            data: NodeData::Document {
                quirks_mode: QuirksMode::NoQuirks,
            },
        };
        Self {
            nodes: vec![root],
            root: NodeId(0),
            url: "about:blank".to_string(),
            title_override: None,
            mutations: Vec::new(),
            track_mutations: false,
        }
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.0]
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0)
    }

    pub fn element(&self, id: NodeId) -> Option<&ElementData> {
        match &self.nodes.get(id.0)?.data {
            NodeData::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn element_mut(&mut self, id: NodeId) -> Option<&mut ElementData> {
        match &mut self.nodes.get_mut(id.0)?.data {
            NodeData::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn quirks_mode(&self) -> QuirksMode {
        match self.nodes[self.root.0].data {
            NodeData::Document { quirks_mode } => quirks_mode,
            _ => QuirksMode::NoQuirks,
        }
    }

    pub fn set_quirks_mode(&mut self, mode: QuirksMode) {
        if let NodeData::Document { ref mut quirks_mode } = self.nodes[self.root.0].data {
            *quirks_mode = mode;
        }
    }

    fn alloc(&mut self, data: NodeData) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            id,
            parent: None,
            first_child: None,
            last_child: None,
            prev_sibling: None,
            next_sibling: None,
            data,
        });
        id
    }

    pub fn create_element(&mut self, tag: impl Into<String>) -> NodeId {
        let lower = tag.into().to_ascii_lowercase();
        let is_template = lower == "template";
        let id = self.alloc(NodeData::Element(ElementData::new(lower, Namespace::Html)));
        if is_template {
            let frag = self.create_fragment();
            if let Some(el) = self.element_mut(id) {
                el.template_contents = Some(frag);
            }
        }
        id
    }

    pub fn create_element_ns(
        &mut self,
        tag: impl Into<String>,
        ns: Namespace,
        attrs: Vec<Attribute>,
    ) -> NodeId {
        let tag_s = tag.into();
        let is_template = ns == Namespace::Html && tag_s == "template";
        let mut ed = ElementData::new(tag_s, ns);
        ed.attributes = attrs;
        let id = self.alloc(NodeData::Element(ed));
        if is_template {
            let frag = self.create_fragment();
            if let Some(el) = self.element_mut(id) {
                el.template_contents = Some(frag);
            }
        }
        id
    }

    pub fn create_text(&mut self, text: impl Into<String>) -> NodeId {
        self.alloc(NodeData::Text(text.into()))
    }

    pub fn create_comment(&mut self, text: impl Into<String>) -> NodeId {
        self.alloc(NodeData::Comment(text.into()))
    }

    pub fn create_doctype(
        &mut self,
        name: impl Into<String>,
        public_id: impl Into<String>,
        system_id: impl Into<String>,
    ) -> NodeId {
        self.alloc(NodeData::Doctype {
            name: name.into(),
            public_id: public_id.into(),
            system_id: system_id.into(),
        })
    }

    pub fn create_fragment(&mut self) -> NodeId {
        self.alloc(NodeData::DocumentFragment)
    }

    pub fn create_sub_document(&mut self) -> NodeId {
        let doc = self.alloc(NodeData::Document {
            quirks_mode: QuirksMode::NoQuirks,
        });
        let html = self.create_element("html");
        let head = self.create_element("head");
        let body = self.create_element("body");
        self.append_child(doc, html);
        self.append_child(html, head);
        self.append_child(html, body);
        doc
    }

    pub fn create_shadow_root(&mut self, host: NodeId, mode: ShadowRootMode) -> NodeId {
        let sr = self.alloc(NodeData::ShadowRoot { mode, host });
        if let Some(el) = self.element_mut(host) {
            el.shadow_root = Some(sr);
        }
        sr
    }

    pub fn create_processing_instruction(
        &mut self,
        target: impl Into<String>,
        data: impl Into<String>,
    ) -> NodeId {
        self.alloc(NodeData::ProcessingInstruction {
            target: target.into(),
            data: data.into(),
        })
    }

    pub fn detach(&mut self, child: NodeId) {
        let parent = self.nodes[child.0].parent;
        let prev = self.nodes[child.0].prev_sibling;
        let next = self.nodes[child.0].next_sibling;
        let Some(parent_id) = parent else {
            return;
        };
        if let Some(p) = prev {
            self.nodes[p.0].next_sibling = next;
        } else {
            self.nodes[parent_id.0].first_child = next;
        }
        if let Some(n) = next {
            self.nodes[n.0].prev_sibling = prev;
        } else {
            self.nodes[parent_id.0].last_child = prev;
        }
        self.nodes[child.0].parent = None;
        self.nodes[child.0].prev_sibling = None;
        self.nodes[child.0].next_sibling = None;
        if self.track_mutations {
            self.mutations.push(DomMutation::ChildList {
                target: parent_id,
                added: Vec::new(),
                removed: vec![child],
                prev_sibling: prev,
                next_sibling: next,
            });
        }
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        if matches!(self.nodes[child.0].data, NodeData::DocumentFragment) {
            let kids = self.children(child);
            for k in kids {
                self.append_child(parent, k);
            }
            return;
        }
        if self.is_ancestor(child, parent) || child == parent {
            return;
        }
        self.detach(child);
        let prev_last = self.nodes[parent.0].last_child;
        if let Some(last) = prev_last {
            self.nodes[last.0].next_sibling = Some(child);
            self.nodes[child.0].prev_sibling = Some(last);
        } else {
            self.nodes[parent.0].first_child = Some(child);
        }
        self.nodes[parent.0].last_child = Some(child);
        self.nodes[child.0].parent = Some(parent);
        if self.track_mutations {
            self.mutations.push(DomMutation::ChildList {
                target: parent,
                added: vec![child],
                removed: Vec::new(),
                prev_sibling: prev_last,
                next_sibling: None,
            });
        }
    }

    pub fn insert_before(&mut self, parent: NodeId, new_child: NodeId, ref_child: Option<NodeId>) {
        let Some(reference) = ref_child else {
            self.append_child(parent, new_child);
            return;
        };
        if self.nodes[reference.0].parent != Some(parent) {
            self.append_child(parent, new_child);
            return;
        }
        if matches!(self.nodes[new_child.0].data, NodeData::DocumentFragment) {
            let kids = self.children(new_child);
            for k in kids {
                self.insert_before(parent, k, Some(reference));
            }
            return;
        }
        if self.is_ancestor(new_child, parent) || new_child == parent || new_child == reference {
            return;
        }
        self.detach(new_child);
        let prev = self.nodes[reference.0].prev_sibling;
        self.nodes[new_child.0].parent = Some(parent);
        self.nodes[new_child.0].prev_sibling = prev;
        self.nodes[new_child.0].next_sibling = Some(reference);
        self.nodes[reference.0].prev_sibling = Some(new_child);
        if let Some(p) = prev {
            self.nodes[p.0].next_sibling = Some(new_child);
        } else {
            self.nodes[parent.0].first_child = Some(new_child);
        }
        if self.track_mutations {
            self.mutations.push(DomMutation::ChildList {
                target: parent,
                added: vec![new_child],
                removed: Vec::new(),
                prev_sibling: prev,
                next_sibling: Some(reference),
            });
        }
    }

    pub fn replace_child(&mut self, parent: NodeId, new_child: NodeId, old_child: NodeId) -> NodeId {
        self.insert_before(parent, new_child, Some(old_child));
        self.detach(old_child);
        old_child
    }

    pub fn set_attribute(&mut self, target: NodeId, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();
        let old_value = self
            .element(target)
            .and_then(|e| e.get_attr(&name).map(|s| s.to_string()));
        if let Some(el) = self.element_mut(target) {
            el.set_attr(name.clone(), value);
        }
        if self.track_mutations {
            self.mutations.push(DomMutation::Attribute {
                target,
                name,
                old_value,
            });
        }
    }

    pub fn remove_attribute(&mut self, target: NodeId, name: &str) {
        let old_value = self.element_mut(target).and_then(|e| e.remove_attr(name));
        if old_value.is_some() && self.track_mutations {
            self.mutations.push(DomMutation::Attribute {
                target,
                name: name.to_string(),
                old_value,
            });
        }
    }

    pub fn set_character_data(&mut self, target: NodeId, value: String) {
        let old = match &mut self.nodes[target.0].data {
            NodeData::Text(ref mut s) | NodeData::Comment(ref mut s) => {
                std::mem::replace(s, value)
            }
            _ => return,
        };
        if self.track_mutations {
            self.mutations.push(DomMutation::CharacterData {
                target,
                old_value: old,
            });
        }
    }

    pub fn clone_node(&mut self, id: NodeId, deep: bool) -> NodeId {
        let cloned_data = match &self.nodes[id.0].data {
            NodeData::Document { quirks_mode } => NodeData::Document {
                quirks_mode: *quirks_mode,
            },
            NodeData::Doctype {
                name,
                public_id,
                system_id,
            } => NodeData::Doctype {
                name: name.clone(),
                public_id: public_id.clone(),
                system_id: system_id.clone(),
            },
            NodeData::Element(e) => {
                let mut copy = e.clone();
                copy.shadow_root = None;
                copy.template_contents = None;
                NodeData::Element(copy)
            }
            NodeData::Text(s) => NodeData::Text(s.clone()),
            NodeData::Comment(s) => NodeData::Comment(s.clone()),
            NodeData::DocumentFragment => NodeData::DocumentFragment,
            NodeData::ShadowRoot { mode, host } => NodeData::ShadowRoot {
                mode: *mode,
                host: *host,
            },
            NodeData::ProcessingInstruction { target, data } => NodeData::ProcessingInstruction {
                target: target.clone(),
                data: data.clone(),
            },
        };
        let new_id = self.alloc(cloned_data);
        if deep {
            let kids = self.children(id);
            for k in kids {
                let ck = self.clone_node(k, true);
                self.append_child(new_id, ck);
            }
        }
        new_id
    }

    pub fn is_ancestor(&self, maybe_ancestor: NodeId, node: NodeId) -> bool {
        let mut cur = self.nodes[node.0].parent;
        while let Some(p) = cur {
            if p == maybe_ancestor {
                return true;
            }
            cur = self.nodes[p.0].parent;
        }
        false
    }

    pub fn children(&self, parent: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut cur = self.nodes[parent.0].first_child;
        while let Some(id) = cur {
            out.push(id);
            cur = self.nodes[id.0].next_sibling;
        }
        out
    }

    pub fn element_children(&self, parent: NodeId) -> Vec<NodeId> {
        self.children(parent)
            .into_iter()
            .filter(|&id| matches!(self.nodes[id.0].data, NodeData::Element(_)))
            .collect()
    }

    pub fn descendants(&self, root: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = Vec::new();
        let mut cur = self.nodes[root.0].last_child;
        while let Some(c) = cur {
            stack.push(c);
            cur = self.nodes[c.0].prev_sibling;
        }
        while let Some(id) = stack.pop() {
            out.push(id);
            let mut ch = self.nodes[id.0].last_child;
            while let Some(c) = ch {
                stack.push(c);
                ch = self.nodes[c.0].prev_sibling;
            }
        }
        out
    }

    pub fn text_content(&self, id: NodeId) -> String {
        match &self.nodes[id.0].data {
            NodeData::Text(s) | NodeData::Comment(s) => s.clone(),
            NodeData::ProcessingInstruction { data, .. } => data.clone(),
            NodeData::Element(_) | NodeData::DocumentFragment | NodeData::ShadowRoot { .. } => {
                let mut out = String::new();
                for d in self.descendants(id) {
                    if let NodeData::Text(ref s) = self.nodes[d.0].data {
                        out.push_str(s);
                    }
                }
                out
            }
            _ => String::new(),
        }
    }

    pub fn set_text_content(&mut self, id: NodeId, text: String) {
        match &self.nodes[id.0].data {
            NodeData::Text(_) | NodeData::Comment(_) => {
                self.set_character_data(id, text);
            }
            NodeData::Element(_) | NodeData::DocumentFragment | NodeData::ShadowRoot { .. } => {
                let kids = self.children(id);
                for k in kids {
                    self.detach(k);
                }
                if !text.is_empty() {
                    let t = self.create_text(text);
                    self.append_child(id, t);
                }
            }
            _ => {}
        }
    }

    pub fn document_element(&self) -> Option<NodeId> {
        self.document_element_of(self.root)
    }

    pub fn document_element_of(&self, doc_root: NodeId) -> Option<NodeId> {
        self.element_children(doc_root).into_iter().next()
    }

    pub fn head(&self) -> Option<NodeId> {
        self.head_of(self.root)
    }

    pub fn head_of(&self, doc_root: NodeId) -> Option<NodeId> {
        let html = self.document_element_of(doc_root)?;
        self.element_children(html)
            .into_iter()
            .find(|&id| self.element(id).map_or(false, |e| e.tag_is("head")))
    }

    pub fn body(&self) -> Option<NodeId> {
        self.body_of(self.root)
    }

    pub fn body_of(&self, doc_root: NodeId) -> Option<NodeId> {
        let html = self.document_element_of(doc_root)?;
        self.element_children(html)
            .into_iter()
            .find(|&id| {
                self.element(id)
                    .map_or(false, |e| e.tag_is("body") || e.tag_is("frameset"))
            })
    }

    pub fn title(&self) -> String {
        if let Some(ref t) = self.title_override {
            return t.clone();
        }
        for id in self.descendants(self.root) {
            if let Some(el) = self.element(id) {
                if el.is_html() && el.tag_is("title") {
                    return self
                        .text_content(id)
                        .split_ascii_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                }
            }
        }
        String::new()
    }

    pub fn get_element_by_id(&self, target_id: &str) -> Option<NodeId> {
        self.get_element_by_id_in(self.root, target_id)
    }

    pub fn get_element_by_id_in(&self, root: NodeId, target_id: &str) -> Option<NodeId> {
        if target_id.is_empty() {
            return None;
        }
        for id in self.descendants(root) {
            if let Some(el) = self.element(id) {
                if el.id() == Some(target_id) {
                    return Some(id);
                }
            }
        }
        None
    }

    pub fn get_elements_by_tag_name(&self, root: NodeId, tag: &str) -> Vec<NodeId> {
        let wildcard = tag == "*";
        self.descendants(root)
            .into_iter()
            .filter(|&id| {
                self.element(id)
                    .map_or(false, |e| wildcard || e.tag_is(tag))
            })
            .collect()
    }

    pub fn get_elements_by_class_name(&self, root: NodeId, classes: &str) -> Vec<NodeId> {
        let wanted: Vec<&str> = classes.split_ascii_whitespace().collect();
        if wanted.is_empty() {
            return Vec::new();
        }
        self.descendants(root)
            .into_iter()
            .filter(|&id| {
                self.element(id)
                    .map_or(false, |e| wanted.iter().all(|c| e.has_class(c)))
            })
            .collect()
    }

    pub fn inner_html(&self, id: NodeId) -> String {
        let mut out = String::new();
        for child in self.children(id) {
            self.serialize_node(child, &mut out);
        }
        out
    }

    pub fn outer_html(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.serialize_node(id, &mut out);
        out
    }

    pub fn serialize_node(&self, id: NodeId, out: &mut String) {
        match &self.nodes[id.0].data {
            NodeData::Document { .. } | NodeData::DocumentFragment | NodeData::ShadowRoot { .. } => {
                for c in self.children(id) {
                    self.serialize_node(c, out);
                }
            }
            NodeData::Doctype { name, .. } => {
                out.push_str("<!DOCTYPE ");
                out.push_str(name);
                out.push('>');
            }
            NodeData::Comment(s) => {
                out.push_str("<!--");
                out.push_str(s);
                out.push_str("-->");
            }
            NodeData::ProcessingInstruction { target, data } => {
                out.push_str("<?");
                out.push_str(target);
                out.push(' ');
                out.push_str(data);
                out.push('>');
            }
            NodeData::Text(s) => {
                let raw_parent = self.nodes[id.0]
                    .parent
                    .and_then(|p| self.element(p))
                    .map_or(false, |e| {
                        e.is_html()
                            && matches!(
                                e.tag_name.as_str(),
                                "style" | "script" | "xmp" | "iframe" | "noembed" | "noframes" | "plaintext"
                            )
                    });
                if raw_parent {
                    out.push_str(s);
                } else {
                    escape_html_text(s, out);
                }
            }
            NodeData::Element(e) => {
                out.push('<');
                out.push_str(&e.tag_name);
                for attr in &e.attributes {
                    out.push(' ');
                    out.push_str(&attr.name);
                    out.push_str("=\"");
                    escape_html_attr(&attr.value, out);
                    out.push('"');
                }
                out.push('>');
                if e.is_html() && is_void_html_element(&e.tag_name) {
                    return;
                }
                if let Some(tc) = e.template_contents {
                    for c in self.children(tc) {
                        self.serialize_node(c, out);
                    }
                } else {
                    for c in self.children(id) {
                        self.serialize_node(c, out);
                    }
                }
                out.push_str("</");
                out.push_str(&e.tag_name);
                out.push('>');
            }
        }
    }
}

pub fn is_void_html_element(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "basefont"
            | "bgsound"
            | "br"
            | "col"
            | "embed"
            | "frame"
            | "hr"
            | "img"
            | "input"
            | "keygen"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn escape_html_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{00A0}' => out.push_str("&nbsp;"),
            _ => out.push(c),
        }
    }
}

fn escape_html_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\u{00A0}' => out.push_str("&nbsp;"),
            _ => out.push(c),
        }
    }
}
