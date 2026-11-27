use std::collections::HashMap;

use peedle_html::{Attribute, Namespace, NodeId};

use crate::state::DomState;

const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";
const SVG_NS: &str = "http://www.w3.org/2000/svg";
const MATHML_NS: &str = "http://www.w3.org/1998/Math/MathML";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";

pub fn parse_xml_into(st: &mut DomState, bytes: &[u8], sub_doc: NodeId) -> Result<Vec<String>, ()> {
    let text = std::str::from_utf8(bytes).map_err(|_| ())?;
    let input = text.strip_prefix('\u{FEFF}').unwrap_or(text);

    let mut parser = XmlParser {
        src: input,
        pos: 0,
        st,
        sub_doc,
        stack: Vec::new(),
        ns_stack: vec![HashMap::new()],
        root_count: 0,
        scripts: Vec::new(),
    };
    parser.parse()
}

struct XmlParser<'a> {
    src: &'a str,
    pos: usize,
    st: &'a mut DomState,
    sub_doc: NodeId,
    stack: Vec<(NodeId, String)>,
    ns_stack: Vec<HashMap<String, String>>,
    root_count: usize,
    scripts: Vec<String>,
}

impl<'a> XmlParser<'a> {
    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn starts_with(&self, s: &str) -> bool {
        self.rest().starts_with(s)
    }

    fn advance(&mut self, n: usize) {
        self.pos += n;
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if matches!(c, ' ' | '\t' | '\r' | '\n') {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
    }

    fn parse(&mut self) -> Result<Vec<String>, ()> {
        let mut nodes_created: Vec<NodeId> = Vec::new();
        while self.pos < self.src.len() {
            if self.starts_with("<!--") {
                self.advance(4);
                let Some(end) = self.rest().find("-->") else {
                    self.rollback(&nodes_created);
                    return Err(());
                };
                let comment_text = self.rest()[..end].to_string();
                if comment_text.contains("--") {
                    self.rollback(&nodes_created);
                    return Err(());
                }
                self.advance(end + 3);
                let cid = self.st.doc.create_comment(comment_text);
                let parent = self.stack.last().map(|(id, _)| *id).unwrap_or(self.sub_doc);
                self.st.doc.append_child(parent, cid);
                nodes_created.push(cid);
            } else if self.starts_with("<![CDATA[") {
                if self.stack.is_empty() {
                    self.rollback(&nodes_created);
                    return Err(());
                }
                self.advance(9);
                let Some(end) = self.rest().find("]]>") else {
                    self.rollback(&nodes_created);
                    return Err(());
                };
                let cdata = self.rest()[..end].to_string();
                self.advance(end + 3);
                let tid = self.st.doc.create_text(cdata);
                let parent = self.stack.last().unwrap().0;
                self.st.doc.append_child(parent, tid);
                nodes_created.push(tid);
            } else if self.starts_with("<?") {
                self.advance(2);
                let Some(end) = self.rest().find("?>") else {
                    self.rollback(&nodes_created);
                    return Err(());
                };
                let inner = &self.rest()[..end];
                let mut parts = inner.splitn(2, |c: char| c.is_ascii_whitespace());
                let target = parts.next().unwrap_or("").to_string();
                let data = parts.next().unwrap_or("").trim_start().to_string();
                self.advance(end + 2);
                if !target.eq_ignore_ascii_case("xml") {
                    let pid = self.st.doc.create_processing_instruction(target, data);
                    let parent = self.stack.last().map(|(id, _)| *id).unwrap_or(self.sub_doc);
                    self.st.doc.append_child(parent, pid);
                    nodes_created.push(pid);
                }
            } else if self.starts_with("<!DOCTYPE") {
                if self.root_count > 0 || !self.stack.is_empty() {
                    self.rollback(&nodes_created);
                    return Err(());
                }
                self.advance(9);
                let Some(end) = self.rest().find('>') else {
                    self.rollback(&nodes_created);
                    return Err(());
                };
                let inner = self.rest()[..end].trim();
                let name = inner.split_ascii_whitespace().next().unwrap_or("html").to_string();
                self.advance(end + 1);
                let dt = self.st.doc.create_doctype(name, String::new(), String::new());
                self.st.doc.append_child(self.sub_doc, dt);
                nodes_created.push(dt);
            } else if self.starts_with("</") {
                self.advance(2);
                let name = match self.read_name() {
                    Some(n) => n,
                    None => {
                        self.rollback(&nodes_created);
                        return Err(());
                    }
                };
                self.skip_ws();
                if !self.starts_with(">") {
                    self.rollback(&nodes_created);
                    return Err(());
                }
                self.advance(1);
                let Some((closed_id, open_name)) = self.stack.pop() else {
                    self.rollback(&nodes_created);
                    return Err(());
                };
                if open_name != name {
                    self.rollback(&nodes_created);
                    return Err(());
                }
                self.ns_stack.pop();
                self.maybe_collect_script(closed_id);
            } else if self.starts_with("<") {
                self.advance(1);
                let name = match self.read_name() {
                    Some(n) => n,
                    None => {
                        self.rollback(&nodes_created);
                        return Err(());
                    }
                };
                let mut raw_attrs: Vec<(String, String)> = Vec::new();
                let mut self_closing = false;
                loop {
                    let had_ws = self.peek().map_or(false, |c| matches!(c, ' ' | '\t' | '\r' | '\n'));
                    self.skip_ws();
                    if self.starts_with("/>") {
                        self.advance(2);
                        self_closing = true;
                        break;
                    }
                    if self.starts_with(">") {
                        self.advance(1);
                        break;
                    }
                    if !had_ws && !raw_attrs.is_empty() {
                        self.rollback(&nodes_created);
                        return Err(());
                    }
                    let Some(aname) = self.read_name() else {
                        self.rollback(&nodes_created);
                        return Err(());
                    };
                    self.skip_ws();
                    if !self.starts_with("=") {
                        self.rollback(&nodes_created);
                        return Err(());
                    }
                    self.advance(1);
                    self.skip_ws();
                    let Some(aval) = self.read_attr_value() else {
                        self.rollback(&nodes_created);
                        return Err(());
                    };
                    if raw_attrs.iter().any(|(k, _)| k == &aname) {
                        self.rollback(&nodes_created);
                        return Err(());
                    }
                    raw_attrs.push((aname, aval));
                }

                if self.stack.is_empty() {
                    self.root_count += 1;
                    if self.root_count > 1 {
                        self.rollback(&nodes_created);
                        return Err(());
                    }
                }

                let mut scope = self.ns_stack.last().cloned().unwrap_or_default();
                for (k, v) in &raw_attrs {
                    if k == "xmlns" {
                        scope.insert(String::new(), v.clone());
                    } else if let Some(pfx) = k.strip_prefix("xmlns:") {
                        if pfx.is_empty() {
                            self.rollback(&nodes_created);
                            return Err(());
                        }
                        scope.insert(pfx.to_string(), v.clone());
                    }
                }

                let ns_uri = if let Some((pfx, _)) = name.split_once(':') {
                    if pfx == "xml" {
                        Some(XML_NS.to_string())
                    } else if pfx == "xmlns" {
                        Some(XMLNS_NS.to_string())
                    } else {
                        match scope.get(pfx) {
                            Some(u) if !u.is_empty() => Some(u.clone()),
                            _ => {
                                self.rollback(&nodes_created);
                                return Err(());
                            }
                        }
                    }
                } else {
                    scope.get("").filter(|u| !u.is_empty()).cloned()
                };

                let dom_ns = match ns_uri.as_deref() {
                    Some(SVG_NS) => Namespace::Svg,
                    Some(MATHML_NS) => Namespace::MathMl,
                    _ => Namespace::Html,
                };
                let attrs: Vec<Attribute> = raw_attrs
                    .into_iter()
                    .map(|(k, v)| Attribute::new(k, v))
                    .collect();
                let eid = self.st.doc.create_element_ns(name.clone(), dom_ns, attrs);
                self.st.xml_elements.insert(eid);
                self.st.custom_namespaces.insert(eid, ns_uri);
                let parent = self.stack.last().map(|(id, _)| *id).unwrap_or(self.sub_doc);
                self.st.doc.append_child(parent, eid);
                nodes_created.push(eid);

                if self_closing {
                    self.maybe_collect_script(eid);
                } else {
                    self.stack.push((eid, name));
                    self.ns_stack.push(scope);
                }
            } else {
                let start = self.pos;
                while let Some(c) = self.peek() {
                    if c == '<' {
                        break;
                    }
                    self.pos += c.len_utf8();
                }
                let raw_text = &self.src[start..self.pos];
                let decoded = match decode_xml_entities(raw_text) {
                    Some(s) => s,
                    None => {
                        self.rollback(&nodes_created);
                        return Err(());
                    }
                };
                if self.stack.is_empty() {
                    if decoded.chars().any(|c| !matches!(c, ' ' | '\t' | '\r' | '\n')) {
                        self.rollback(&nodes_created);
                        return Err(());
                    }
                } else {
                    let tid = self.st.doc.create_text(decoded);
                    let parent = self.stack.last().unwrap().0;
                    self.st.doc.append_child(parent, tid);
                    nodes_created.push(tid);
                }
            }
        }

        if !self.stack.is_empty() || self.root_count != 1 {
            self.rollback(&nodes_created);
            return Err(());
        }
        Ok(std::mem::take(&mut self.scripts))
    }

    fn rollback(&mut self, created: &[NodeId]) {
        for c in self.st.doc.children(self.sub_doc) {
            self.st.doc.detach(c);
        }
        for &id in created {
            self.st.xml_elements.remove(&id);
            self.st.custom_namespaces.remove(&id);
        }
    }

    fn maybe_collect_script(&mut self, eid: NodeId) {
        let is_xhtml_ns = self
            .st
            .custom_namespaces
            .get(&eid)
            .and_then(|o| o.as_deref())
            == Some(XHTML_NS);
        if !is_xhtml_ns {
            return;
        }
        let Some(el) = self.st.doc.element(eid) else {
            return;
        };
        let local = el
            .tag_name
            .split_once(':')
            .map(|(_, l)| l)
            .unwrap_or(&el.tag_name);
        if local != "script" {
            return;
        }
        let stype = el
            .get_attr("type")
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let is_js = stype.is_empty()
            || stype.contains("javascript")
            || stype.contains("ecmascript");
        if is_js {
            let code = self.st.doc.text_content(eid);
            if !code.trim().is_empty() {
                self.scripts.push(code);
            }
        }
    }

    fn read_name(&mut self) -> Option<String> {
        let start = self.pos;
        let first = self.peek()?;
        if !is_xml_name_start(first) {
            return None;
        }
        self.pos += first.len_utf8();
        while let Some(c) = self.peek() {
            if is_xml_name_char(c) {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
        Some(self.src[start..self.pos].to_string())
    }

    fn read_attr_value(&mut self) -> Option<String> {
        let quote = self.peek()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        self.pos += 1;
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == quote {
                let raw = &self.src[start..self.pos];
                self.pos += 1;
                return decode_xml_entities(raw);
            }
            if c == '<' {
                return None;
            }
            self.pos += c.len_utf8();
        }
        None
    }
}

fn is_xml_name_start(c: char) -> bool {
    c.is_ascii_alphabetic()
        || c == '_'
        || c == ':'
        || (c as u32 >= 0xC0 && c != '\u{D7}' && c != '\u{F7}')
}

fn is_xml_name_char(c: char) -> bool {
    is_xml_name_start(c) || c.is_ascii_digit() || matches!(c, '-' | '.') || c as u32 == 0xB7
}

fn decode_xml_entities(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let semi = after.find(';')?;
        let ent = &after[..semi];
        match ent {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                let code = u32::from_str_radix(&ent[2..], 16).ok()?;
                let ch = char::from_u32(code)?;
                out.push(ch);
            }
            _ if ent.starts_with('#') => {
                let code = ent[1..].parse::<u32>().ok()?;
                let ch = char::from_u32(code)?;
                out.push(ch);
            }
            _ => return None,
        }
        rest = &after[semi + 1..];
    }
    out.push_str(rest);
    Some(out)
}
