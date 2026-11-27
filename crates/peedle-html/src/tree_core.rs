use crate::dom::{Attribute, Document, Namespace, NodeData, NodeId, QuirksMode};
use crate::tokenizer::{DoctypeToken, TagToken, TokenizerState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    InHeadNoscript,
    AfterHead,
    InBody,
    Text,
    InTable,
    InTableText,
    InCaption,
    InColumnGroup,
    InTableBody,
    InRow,
    InCell,
    InSelect,
    InSelectInTable,
    InTemplate,
    AfterBody,
    InFrameset,
    AfterFrameset,
    AfterAfterBody,
    AfterAfterFrameset,
}

#[derive(Clone, Debug)]
pub enum FormattingItem {
    Marker,
    Element {
        node: NodeId,
        tag: String,
        attrs: Vec<Attribute>,
    },
}

pub struct TreeState {
    pub doc: Document,
    pub mode: Mode,
    pub original_mode: Mode,
    pub template_modes: Vec<Mode>,
    pub open_elements: Vec<NodeId>,
    pub active_formatting: Vec<FormattingItem>,
    pub head_element: Option<NodeId>,
    pub form_element: Option<NodeId>,
    pub frameset_ok: bool,
    pub foster_parenting: bool,
    pub pending_table_chars: String,
    pub ignore_leading_lf: bool,
    pub fragment_context: Option<NodeId>,
    pub scripting_enabled: bool,
}

impl TreeState {
    pub fn new(scripting_enabled: bool) -> Self {
        Self {
            doc: Document::new(),
            mode: Mode::Initial,
            original_mode: Mode::Initial,
            template_modes: Vec::new(),
            open_elements: Vec::new(),
            active_formatting: Vec::new(),
            head_element: None,
            form_element: None,
            frameset_ok: true,
            foster_parenting: false,
            pending_table_chars: String::new(),
            ignore_leading_lf: false,
            fragment_context: None,
            scripting_enabled,
        }
    }

    pub fn current_node(&self) -> Option<NodeId> {
        self.open_elements.last().copied()
    }

    pub fn adjusted_current_node(&self) -> Option<NodeId> {
        if self.open_elements.len() == 1 {
            if let Some(ctx) = self.fragment_context {
                return Some(ctx);
            }
        }
        self.current_node()
    }

    pub fn node_is_html(&self, id: NodeId, tag: &str) -> bool {
        self.doc
            .element(id)
            .map_or(false, |e| e.namespace == Namespace::Html && e.tag_name == tag)
    }

    pub fn node_tag(&self, id: NodeId) -> &str {
        self.doc
            .element(id)
            .map_or("", |e| e.tag_name.as_str())
    }

    pub fn node_ns(&self, id: NodeId) -> Namespace {
        self.doc
            .element(id)
            .map_or(Namespace::Html, |e| e.namespace)
    }

    pub fn current_is_html(&self, tag: &str) -> bool {
        self.current_node().map_or(false, |id| self.node_is_html(id, tag))
    }

    pub fn fragment_context_is_html(&self, tag: &str) -> bool {
        self.fragment_context
            .map_or(false, |id| self.node_is_html(id, tag))
    }

    pub fn insertion_location(&self, override_target: Option<NodeId>) -> (NodeId, Option<NodeId>) {
        let target = override_target.or_else(|| self.current_node()).unwrap_or(self.doc.root);
        if self.foster_parenting {
            if let Some(el) = self.doc.element(target) {
                if el.namespace == Namespace::Html
                    && matches!(
                        el.tag_name.as_str(),
                        "table" | "tbody" | "tfoot" | "thead" | "tr"
                    )
                {
                    return self.foster_location();
                }
            }
        }
        if let Some(el) = self.doc.element(target) {
            if el.namespace == Namespace::Html && el.tag_name == "template" {
                if let Some(tc) = el.template_contents {
                    return (tc, None);
                }
            }
        }
        (target, None)
    }

    fn foster_location(&self) -> (NodeId, Option<NodeId>) {
        let last_template = self
            .open_elements
            .iter()
            .rposition(|&id| self.node_is_html(id, "template"));
        let last_table = self
            .open_elements
            .iter()
            .rposition(|&id| self.node_is_html(id, "table"));

        if let Some(t_idx) = last_template {
            if last_table.map_or(true, |tbl_idx| t_idx > tbl_idx) {
                let tpl_id = self.open_elements[t_idx];
                if let Some(tc) = self.doc.element(tpl_id).and_then(|e| e.template_contents) {
                    return (tc, None);
                }
            }
        }
        if let Some(tbl_idx) = last_table {
            let tbl_id = self.open_elements[tbl_idx];
            if let Some(parent) = self.doc.node(tbl_id).parent {
                return (parent, Some(tbl_id));
            }
            if tbl_idx > 0 {
                return (self.open_elements[tbl_idx - 1], None);
            }
        }
        if let Some(&first) = self.open_elements.first() {
            return (first, None);
        }
        (self.doc.root, None)
    }

    pub fn insert_element(&mut self, tag: &str, ns: Namespace, attrs: Vec<Attribute>) -> NodeId {
        let (parent, before) = self.insertion_location(None);
        let id = self.doc.create_element_ns(tag, ns, attrs);
        self.doc.insert_before(parent, id, before);
        self.open_elements.push(id);
        id
    }

    pub fn insert_html_element(&mut self, tok: &TagToken) -> NodeId {
        self.insert_element(&tok.name, Namespace::Html, tok.attributes.clone())
    }

    pub fn insert_void_html(&mut self, tok: &TagToken) -> NodeId {
        let id = self.insert_html_element(tok);
        self.open_elements.pop();
        id
    }

    pub fn insert_comment(&mut self, data: String, target: Option<NodeId>) {
        let (parent, before) = match target {
            Some(t) => (t, None),
            None => self.insertion_location(None),
        };
        let id = self.doc.create_comment(data);
        self.doc.insert_before(parent, id, before);
    }

    pub fn insert_char(&mut self, ch: char) {
        let (parent, before) = self.insertion_location(None);
        if matches!(self.doc.node(parent).data, NodeData::Document { .. }) {
            return;
        }
        let prev = match before {
            Some(ref_id) => self.doc.node(ref_id).prev_sibling,
            None => self.doc.node(parent).last_child,
        };
        if let Some(prev_id) = prev {
            if let NodeData::Text(ref mut s) = self.doc.node_mut(prev_id).data {
                s.push(ch);
                return;
            }
        }
        let text_id = self.doc.create_text(ch.to_string());
        self.doc.insert_before(parent, text_id, before);
    }

    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.insert_char(c);
        }
    }

    pub fn in_scope_with(&self, target: &str, barriers: &[(&str, Namespace)]) -> bool {
        for &id in self.open_elements.iter().rev() {
            let Some(el) = self.doc.element(id) else {
                continue;
            };
            if el.namespace == Namespace::Html && el.tag_name == target {
                return true;
            }
            if barriers
                .iter()
                .any(|&(b_tag, b_ns)| el.namespace == b_ns && el.tag_name == b_tag)
            {
                return false;
            }
        }
        false
    }

    pub fn in_scope(&self, tag: &str) -> bool {
        self.in_scope_with(tag, DEFAULT_SCOPE_BARRIERS)
    }

    pub fn in_button_scope(&self, tag: &str) -> bool {
        self.in_scope_with(tag, BUTTON_SCOPE_BARRIERS)
    }

    pub fn in_list_item_scope(&self, tag: &str) -> bool {
        self.in_scope_with(tag, LIST_ITEM_SCOPE_BARRIERS)
    }

    pub fn in_table_scope(&self, tag: &str) -> bool {
        self.in_scope_with(tag, TABLE_SCOPE_BARRIERS)
    }

    pub fn in_select_scope(&self, tag: &str) -> bool {
        for &id in self.open_elements.iter().rev() {
            let Some(el) = self.doc.element(id) else {
                continue;
            };
            if el.namespace == Namespace::Html && el.tag_name == tag {
                return true;
            }
            if !(el.namespace == Namespace::Html
                && matches!(el.tag_name.as_str(), "optgroup" | "option"))
            {
                return false;
            }
        }
        false
    }

    pub fn has_heading_in_scope(&self) -> bool {
        for &id in self.open_elements.iter().rev() {
            let Some(el) = self.doc.element(id) else {
                continue;
            };
            if el.namespace == Namespace::Html
                && matches!(el.tag_name.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
            {
                return true;
            }
            if DEFAULT_SCOPE_BARRIERS
                .iter()
                .any(|&(b_tag, b_ns)| el.namespace == b_ns && el.tag_name == b_tag)
            {
                return false;
            }
        }
        false
    }

    pub fn pop_until_html(&mut self, tag: &str) {
        while let Some(id) = self.open_elements.pop() {
            if self.node_is_html(id, tag) {
                break;
            }
        }
    }

    pub fn generate_implied_end_tags(&mut self, except: Option<&str>) {
        while let Some(id) = self.current_node() {
            let is_implied = self.doc.element(id).map_or(false, |e| {
                e.namespace == Namespace::Html
                    && except.map_or(true, |ex| e.tag_name != ex)
                    && matches!(
                        e.tag_name.as_str(),
                        "dd" | "dt" | "li" | "optgroup" | "option" | "p" | "rb" | "rp" | "rt" | "rtc"
                    )
            });
            if is_implied {
                self.open_elements.pop();
            } else {
                break;
            }
        }
    }

    pub fn generate_all_implied_end_tags(&mut self, except: Option<&str>) {
        while let Some(id) = self.current_node() {
            let is_implied = self.doc.element(id).map_or(false, |e| {
                e.namespace == Namespace::Html
                    && except.map_or(true, |ex| e.tag_name != ex)
                    && matches!(
                        e.tag_name.as_str(),
                        "caption"
                            | "colgroup"
                            | "dd"
                            | "dt"
                            | "li"
                            | "optgroup"
                            | "option"
                            | "p"
                            | "rb"
                            | "rp"
                            | "rt"
                            | "rtc"
                            | "tbody"
                            | "td"
                            | "tfoot"
                            | "th"
                            | "thead"
                            | "tr"
                    )
            });
            if is_implied {
                self.open_elements.pop();
            } else {
                break;
            }
        }
    }

    pub fn close_p_element(&mut self) {
        self.generate_implied_end_tags(Some("p"));
        self.pop_until_html("p");
    }

    pub fn push_formatting(&mut self, node: NodeId, tag: String, attrs: Vec<Attribute>) {
        let mut matches_seen = Vec::new();
        for (i, item) in self.active_formatting.iter().enumerate().rev() {
            match item {
                FormattingItem::Marker => break,
                FormattingItem::Element {
                    tag: itag,
                    attrs: iattrs,
                    ..
                } => {
                    if *itag == tag && *iattrs == attrs {
                        matches_seen.push(i);
                    }
                }
            }
        }
        if matches_seen.len() >= 3 {
            let earliest = *matches_seen.last().unwrap();
            self.active_formatting.remove(earliest);
        }
        self.active_formatting
            .push(FormattingItem::Element { node, tag, attrs });
    }

    pub fn clear_formatting_to_marker(&mut self) {
        while let Some(item) = self.active_formatting.pop() {
            if matches!(item, FormattingItem::Marker) {
                break;
            }
        }
    }

    pub fn reconstruct_formatting(&mut self) {
        if self.active_formatting.is_empty() {
            return;
        }
        let last = self.active_formatting.len() - 1;
        match &self.active_formatting[last] {
            FormattingItem::Marker => return,
            FormattingItem::Element { node, .. } if self.open_elements.contains(node) => return,
            _ => {}
        }
        let mut idx = last;
        loop {
            if idx == 0 {
                break;
            }
            idx -= 1;
            match &self.active_formatting[idx] {
                FormattingItem::Marker => {
                    idx += 1;
                    break;
                }
                FormattingItem::Element { node, .. } if self.open_elements.contains(node) => {
                    idx += 1;
                    break;
                }
                _ => {}
            }
        }
        while idx < self.active_formatting.len() {
            let (tag, attrs) = match &self.active_formatting[idx] {
                FormattingItem::Element { tag, attrs, .. } => (tag.clone(), attrs.clone()),
                FormattingItem::Marker => {
                    idx += 1;
                    continue;
                }
            };
            let new_node = self.insert_element(&tag, Namespace::Html, attrs.clone());
            self.active_formatting[idx] = FormattingItem::Element {
                node: new_node,
                tag,
                attrs,
            };
            idx += 1;
        }
    }

    pub fn run_adoption_agency(&mut self, subject: &str) {
        if let Some(cur) = self.current_node() {
            if self.node_is_html(cur, subject) && !self.in_formatting_list(cur) {
                self.open_elements.pop();
                return;
            }
        }
        for _ in 0..8 {
            let fmt_pos = self.find_formatting_after_marker(subject);
            let Some(fmt_idx) = fmt_pos else {
                self.any_other_end_tag(subject);
                return;
            };
            let (fmt_node, fmt_tag, fmt_attrs) = match &self.active_formatting[fmt_idx] {
                FormattingItem::Element { node, tag, attrs } => (*node, tag.clone(), attrs.clone()),
                FormattingItem::Marker => return,
            };
            let Some(oe_fmt_idx) = self.open_elements.iter().position(|&id| id == fmt_node) else {
                self.active_formatting.remove(fmt_idx);
                return;
            };
            if !self.in_scope(subject) {
                return;
            }
            let furthest_idx = self.open_elements[oe_fmt_idx + 1..]
                .iter()
                .position(|&id| self.is_special_element(id))
                .map(|rel| oe_fmt_idx + 1 + rel);
            let Some(fb_idx) = furthest_idx else {
                while let Some(popped) = self.open_elements.pop() {
                    if popped == fmt_node {
                        break;
                    }
                }
                self.active_formatting.remove(fmt_idx);
                return;
            };
            let furthest_block = self.open_elements[fb_idx];
            let common_ancestor = self.open_elements[oe_fmt_idx - 1];
            let mut bookmark = fmt_idx;
            let mut node_idx = fb_idx;
            let mut last_node = furthest_block;

            let mut inner = 0usize;
            loop {
                inner += 1;
                node_idx -= 1;
                let cur_node = self.open_elements[node_idx];
                if cur_node == fmt_node {
                    break;
                }
                if inner > 3 {
                    if let Some(p) = self.formatting_index(cur_node) {
                        self.active_formatting.remove(p);
                        if p < bookmark {
                            bookmark -= 1;
                        }
                    }
                }
                let Some(af_idx) = self.formatting_index(cur_node) else {
                    self.open_elements.remove(node_idx);
                    continue;
                };
                let (ctag, cattrs) = match &self.active_formatting[af_idx] {
                    FormattingItem::Element { tag, attrs, .. } => (tag.clone(), attrs.clone()),
                    FormattingItem::Marker => continue,
                };
                let replacement = self
                    .doc
                    .create_element_ns(ctag.clone(), Namespace::Html, cattrs.clone());
                self.active_formatting[af_idx] = FormattingItem::Element {
                    node: replacement,
                    tag: ctag,
                    attrs: cattrs,
                };
                self.open_elements[node_idx] = replacement;
                if last_node == furthest_block {
                    bookmark = af_idx + 1;
                }
                self.doc.append_child(replacement, last_node);
                last_node = replacement;
            }

            let (ins_parent, ins_before) = self.insertion_location(Some(common_ancestor));
            self.doc.insert_before(ins_parent, last_node, ins_before);

            let new_fmt = self
                .doc
                .create_element_ns(fmt_tag.clone(), Namespace::Html, fmt_attrs.clone());
            let fb_children = self.doc.children(furthest_block);
            for ch in fb_children {
                self.doc.append_child(new_fmt, ch);
            }
            self.doc.append_child(furthest_block, new_fmt);

            self.active_formatting.remove(fmt_idx);
            let insert_at = if bookmark > fmt_idx {
                (bookmark - 1).min(self.active_formatting.len())
            } else {
                bookmark.min(self.active_formatting.len())
            };
            self.active_formatting.insert(
                insert_at,
                FormattingItem::Element {
                    node: new_fmt,
                    tag: fmt_tag,
                    attrs: fmt_attrs,
                },
            );
            if let Some(pos) = self.open_elements.iter().position(|&id| id == fmt_node) {
                self.open_elements.remove(pos);
            }
            if let Some(fb_pos) = self
                .open_elements
                .iter()
                .position(|&id| id == furthest_block)
            {
                self.open_elements.insert(fb_pos + 1, new_fmt);
            }
        }
    }

    pub fn any_other_end_tag(&mut self, name: &str) {
        for idx in (0..self.open_elements.len()).rev() {
            let id = self.open_elements[idx];
            if self.node_is_html(id, name) {
                self.generate_implied_end_tags(Some(name));
                self.open_elements.truncate(idx);
                return;
            }
            if self.is_special_element(id) {
                return;
            }
        }
    }

    fn in_formatting_list(&self, target: NodeId) -> bool {
        self.formatting_index(target).is_some()
    }

    fn formatting_index(&self, target: NodeId) -> Option<usize> {
        self.active_formatting.iter().position(|item| match item {
            FormattingItem::Element { node, .. } => *node == target,
            FormattingItem::Marker => false,
        })
    }

    fn find_formatting_after_marker(&self, tag_name: &str) -> Option<usize> {
        for (i, item) in self.active_formatting.iter().enumerate().rev() {
            match item {
                FormattingItem::Marker => return None,
                FormattingItem::Element { tag, .. } if tag == tag_name => return Some(i),
                _ => {}
            }
        }
        None
    }

    pub fn is_special_element(&self, id: NodeId) -> bool {
        let Some(el) = self.doc.element(id) else {
            return false;
        };
        match el.namespace {
            Namespace::Html => matches!(
                el.tag_name.as_str(),
                "address"
                    | "applet"
                    | "area"
                    | "article"
                    | "aside"
                    | "base"
                    | "basefont"
                    | "bgsound"
                    | "blockquote"
                    | "body"
                    | "br"
                    | "button"
                    | "caption"
                    | "center"
                    | "col"
                    | "colgroup"
                    | "dd"
                    | "details"
                    | "dir"
                    | "div"
                    | "dl"
                    | "dt"
                    | "embed"
                    | "fieldset"
                    | "figcaption"
                    | "figure"
                    | "footer"
                    | "form"
                    | "frame"
                    | "frameset"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "head"
                    | "header"
                    | "hgroup"
                    | "hr"
                    | "html"
                    | "iframe"
                    | "img"
                    | "input"
                    | "keygen"
                    | "li"
                    | "link"
                    | "listing"
                    | "main"
                    | "marquee"
                    | "menu"
                    | "meta"
                    | "nav"
                    | "noembed"
                    | "noframes"
                    | "noscript"
                    | "object"
                    | "ol"
                    | "p"
                    | "param"
                    | "plaintext"
                    | "pre"
                    | "script"
                    | "search"
                    | "section"
                    | "select"
                    | "source"
                    | "style"
                    | "summary"
                    | "table"
                    | "tbody"
                    | "td"
                    | "template"
                    | "textarea"
                    | "tfoot"
                    | "th"
                    | "thead"
                    | "title"
                    | "tr"
                    | "track"
                    | "ul"
                    | "wbr"
                    | "xmp"
            ),
            Namespace::MathMl => matches!(
                el.tag_name.as_str(),
                "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml"
            ),
            Namespace::Svg => matches!(el.tag_name.as_str(), "foreignObject" | "desc" | "title"),
        }
    }

    pub fn reset_insertion_mode(&mut self) {
        for idx in (0..self.open_elements.len()).rev() {
            let last = idx == 0;
            let node = if last {
                self.fragment_context.unwrap_or(self.open_elements[idx])
            } else {
                self.open_elements[idx]
            };
            let Some(el) = self.doc.element(node) else {
                continue;
            };
            if el.namespace != Namespace::Html {
                if last {
                    self.mode = Mode::InBody;
                    return;
                }
                continue;
            }
            match el.tag_name.as_str() {
                "td" | "th" if !last => {
                    self.mode = Mode::InCell;
                    return;
                }
                "tr" => {
                    self.mode = Mode::InRow;
                    return;
                }
                "tbody" | "thead" | "tfoot" => {
                    self.mode = Mode::InTableBody;
                    return;
                }
                "caption" => {
                    self.mode = Mode::InCaption;
                    return;
                }
                "colgroup" => {
                    self.mode = Mode::InColumnGroup;
                    return;
                }
                "table" => {
                    self.mode = Mode::InTable;
                    return;
                }
                "template" => {
                    self.mode = self.template_modes.last().copied().unwrap_or(Mode::InBody);
                    return;
                }
                "head" if !last => {
                    self.mode = Mode::InHead;
                    return;
                }
                "body" => {
                    self.mode = Mode::InBody;
                    return;
                }
                "frameset" => {
                    self.mode = Mode::InFrameset;
                    return;
                }
                "html" => {
                    self.mode = if self.head_element.is_none() {
                        Mode::BeforeHead
                    } else {
                        Mode::AfterHead
                    };
                    return;
                }
                _ if last => {
                    self.mode = Mode::InBody;
                    return;
                }
                _ => {}
            }
        }
        self.mode = Mode::InBody;
    }

    pub fn parse_raw_element(
        &mut self,
        tok: &TagToken,
        next_tok_state: TokenizerState,
        tok_state: &mut TokenizerState,
    ) {
        self.insert_html_element(tok);
        *tok_state = next_tok_state;
        self.original_mode = self.mode;
        self.mode = Mode::Text;
    }
}

pub fn classify_doctype_quirks(dt: &DoctypeToken) -> QuirksMode {
    if dt.force_quirks {
        return QuirksMode::Quirks;
    }
    let name_ok = dt.name.as_deref().map_or(false, |n| n == "html");
    if !name_ok {
        return QuirksMode::Quirks;
    }
    let pub_id = dt.public_id.as_deref().unwrap_or("");
    let sys_id = dt.system_id.as_deref();
    if pub_id.is_empty() && sys_id.is_none() {
        return QuirksMode::NoQuirks;
    }
    if let Some(s) = sys_id {
        if s.eq_ignore_ascii_case("http://www.ibm.com/data/dtd/v11/ibmxhtml1-transitional.dtd") {
            return QuirksMode::Quirks;
        }
    }
    let lower_pub = pub_id.to_ascii_lowercase();
    if lower_pub == "html"
        || lower_pub == "-//w3o//dtd w3 html strict 3.0//en//"
        || lower_pub == "-/w3c/dtd html 4.0 transitional/en"
    {
        return QuirksMode::Quirks;
    }
    if lower_pub.starts_with("-//w3c//dtd html 4.01 transitional//")
        || lower_pub.starts_with("-//w3c//dtd html 4.01 frameset//")
    {
        return if sys_id.is_some() {
            QuirksMode::LimitedQuirks
        } else {
            QuirksMode::Quirks
        };
    }
    if lower_pub.starts_with("-//w3c//dtd xhtml 1.0 transitional//")
        || lower_pub.starts_with("-//w3c//dtd xhtml 1.0 frameset//")
    {
        return QuirksMode::LimitedQuirks;
    }
    if lower_pub.starts_with("-//w3c//dtd html 4.0//")
        || lower_pub.starts_with("-//w3c//dtd html 3.")
        || lower_pub.starts_with("-//ietf//dtd html")
        || lower_pub.starts_with("+//silmaril//dtd html")
        || lower_pub.starts_with("-//advasoft ltd//dtd html")
        || lower_pub.starts_with("-//netscape comm. corp.//dtd")
    {
        return QuirksMode::Quirks;
    }
    QuirksMode::NoQuirks
}

pub fn is_ascii_ws(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0C' | '\r' | ' ')
}

const DEFAULT_SCOPE_BARRIERS: &[(&str, Namespace)] = &[
    ("applet", Namespace::Html),
    ("caption", Namespace::Html),
    ("html", Namespace::Html),
    ("table", Namespace::Html),
    ("td", Namespace::Html),
    ("th", Namespace::Html),
    ("marquee", Namespace::Html),
    ("object", Namespace::Html),
    ("select", Namespace::Html),
    ("template", Namespace::Html),
    ("mi", Namespace::MathMl),
    ("mo", Namespace::MathMl),
    ("mn", Namespace::MathMl),
    ("ms", Namespace::MathMl),
    ("mtext", Namespace::MathMl),
    ("annotation-xml", Namespace::MathMl),
    ("foreignObject", Namespace::Svg),
    ("desc", Namespace::Svg),
    ("title", Namespace::Svg),
];

const BUTTON_SCOPE_BARRIERS: &[(&str, Namespace)] = &[
    ("applet", Namespace::Html),
    ("caption", Namespace::Html),
    ("html", Namespace::Html),
    ("table", Namespace::Html),
    ("td", Namespace::Html),
    ("th", Namespace::Html),
    ("marquee", Namespace::Html),
    ("object", Namespace::Html),
    ("select", Namespace::Html),
    ("template", Namespace::Html),
    ("button", Namespace::Html),
    ("mi", Namespace::MathMl),
    ("mo", Namespace::MathMl),
    ("mn", Namespace::MathMl),
    ("ms", Namespace::MathMl),
    ("mtext", Namespace::MathMl),
    ("annotation-xml", Namespace::MathMl),
    ("foreignObject", Namespace::Svg),
    ("desc", Namespace::Svg),
    ("title", Namespace::Svg),
];

const LIST_ITEM_SCOPE_BARRIERS: &[(&str, Namespace)] = &[
    ("applet", Namespace::Html),
    ("caption", Namespace::Html),
    ("html", Namespace::Html),
    ("table", Namespace::Html),
    ("td", Namespace::Html),
    ("th", Namespace::Html),
    ("marquee", Namespace::Html),
    ("object", Namespace::Html),
    ("select", Namespace::Html),
    ("template", Namespace::Html),
    ("ol", Namespace::Html),
    ("ul", Namespace::Html),
    ("mi", Namespace::MathMl),
    ("mo", Namespace::MathMl),
    ("mn", Namespace::MathMl),
    ("ms", Namespace::MathMl),
    ("mtext", Namespace::MathMl),
    ("annotation-xml", Namespace::MathMl),
    ("foreignObject", Namespace::Svg),
    ("desc", Namespace::Svg),
    ("title", Namespace::Svg),
];

const TABLE_SCOPE_BARRIERS: &[(&str, Namespace)] = &[
    ("html", Namespace::Html),
    ("table", Namespace::Html),
    ("template", Namespace::Html),
];
