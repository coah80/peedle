use crate::dom::{Document, Namespace, NodeId};
use crate::tokenizer::{Token, Tokenizer, TokenizerState};
use crate::tree_body::{insert_foreign_element, step_head_and_outer, step_in_body, step_in_head};
use crate::tree_core::{is_ascii_ws, Mode, TreeState};
use crate::tree_foreign::{
    breaks_out_of_foreign, is_html_integration_point, is_mathml_text_integration_point,
};
use crate::tree_table::{step_table_modes, StepAction};

pub fn parse_html(input: &str) -> Document {
    parse_html_with_options(input, true)
}

pub fn parse_html_with_options(input: &str, scripting_enabled: bool) -> Document {
    let mut tokenizer = Tokenizer::new(input);
    let mut state = TreeState::new(scripting_enabled);

    loop {
        let adjusted = state.adjusted_current_node();
        tokenizer.allow_cdata = adjusted.map_or(false, |id| state.node_ns(id) != Namespace::Html);

        let token = tokenizer.next_token();
        let is_eof = matches!(token, Token::Eof);
        dispatch_token(&mut state, &token, &mut tokenizer.state);
        if is_eof {
            break;
        }
    }

    sync_selectedcontent_elements(&mut state.doc);
    state.doc
}

pub fn parse_fragment(
    input: &str,
    context_tag: &str,
    context_ns: Namespace,
    scripting_enabled: bool,
) -> Document {
    let mut tokenizer = Tokenizer::new(input);
    let mut state = TreeState::new(scripting_enabled);

    if context_ns == Namespace::Html {
        match context_tag {
            "title" | "textarea" => tokenizer.state = TokenizerState::Rcdata,
            "style" | "xmp" | "iframe" | "noembed" | "noframes" => {
                tokenizer.state = TokenizerState::Rawtext;
            }
            "script" => tokenizer.state = TokenizerState::ScriptData,
            "noscript" if scripting_enabled => tokenizer.state = TokenizerState::Rawtext,
            "plaintext" => tokenizer.state = TokenizerState::Plaintext,
            _ => {}
        }
    }

    let html_id = state.doc.create_element_ns("html", Namespace::Html, Vec::new());
    let root = state.doc.root;
    state.doc.append_child(root, html_id);
    state.open_elements.push(html_id);

    let ctx_id = state
        .doc
        .create_element_ns(context_tag, context_ns, Vec::new());
    state.fragment_context = Some(ctx_id);

    if context_ns == Namespace::Html && context_tag == "template" {
        state.template_modes.push(Mode::InTemplate);
    }
    state.reset_insertion_mode();

    loop {
        let adjusted = state.adjusted_current_node();
        tokenizer.allow_cdata = adjusted.map_or(false, |id| state.node_ns(id) != Namespace::Html);

        let token = tokenizer.next_token();
        let is_eof = matches!(token, Token::Eof);
        dispatch_token(&mut state, &token, &mut tokenizer.state);
        if is_eof {
            break;
        }
    }

    sync_selectedcontent_elements(&mut state.doc);
    state.doc
}

pub fn parse_fragment_into(
    target_doc: &mut Document,
    context_tag: &str,
    context_ns: Namespace,
    html: &str,
) -> Vec<NodeId> {
    let parsed = parse_fragment(html, context_tag, context_ns, true);
    let Some(html_el) = parsed.document_element() else {
        return Vec::new();
    };
    let mut imported = Vec::new();
    for child in parsed.children(html_el) {
        imported.push(import_subtree(&parsed, child, target_doc));
    }
    imported
}

fn import_subtree(src: &Document, id: NodeId, dst: &mut Document) -> NodeId {
    use crate::dom::NodeData;
    let new_id = match &src.node(id).data {
        NodeData::Element(e) => {
            let nid = dst.create_element_ns(e.tag_name.clone(), e.namespace, e.attributes.clone());
            if let Some(src_tc) = e.template_contents {
                let dst_tc = match dst.element(nid).and_then(|el| el.template_contents) {
                    Some(tc) => tc,
                    None => {
                        let tc = dst.create_fragment();
                        if let Some(el) = dst.element_mut(nid) {
                            el.template_contents = Some(tc);
                        }
                        tc
                    }
                };
                for child in src.children(src_tc) {
                    let c = import_subtree(src, child, dst);
                    dst.append_child(dst_tc, c);
                }
            }
            nid
        }
        NodeData::Text(s) => dst.create_text(s.clone()),
        NodeData::Comment(s) => dst.create_comment(s.clone()),
        NodeData::Doctype {
            name,
            public_id,
            system_id,
        } => dst.create_doctype(name.clone(), public_id.clone(), system_id.clone()),
        NodeData::DocumentFragment => dst.create_fragment(),
        NodeData::ProcessingInstruction { target, data } => {
            dst.create_processing_instruction(target.clone(), data.clone())
        }
        _ => dst.create_fragment(),
    };
    for child in src.children(id) {
        let c = import_subtree(src, child, dst);
        dst.append_child(new_id, c);
    }
    new_id
}

fn sync_selectedcontent_elements(doc: &mut Document) {
    let selects: Vec<NodeId> = doc
        .descendants(doc.root)
        .into_iter()
        .filter(|&id| {
            doc.element(id).map_or(false, |el| {
                el.namespace == Namespace::Html
                    && el.tag_name == "select"
                    && !el.has_attr("multiple")
            })
        })
        .collect();

    for select_id in selects {
        let mut selectedcontents = Vec::new();
        let mut options = Vec::new();
        collect_select_descendants(doc, select_id, &mut selectedcontents, &mut options);
        if selectedcontents.is_empty() {
            continue;
        }
        let chosen = options
            .iter()
            .rev()
            .copied()
            .find(|&opt_id| doc.element(opt_id).map_or(false, |e| e.has_attr("selected")))
            .or_else(|| {
                options
                    .iter()
                    .copied()
                    .find(|&opt_id| doc.element(opt_id).map_or(false, |e| !e.is_disabled()))
            });
        for sc_id in selectedcontents {
            let existing = doc.children(sc_id);
            for child in existing {
                doc.detach(child);
            }
            if let Some(opt_id) = chosen {
                let kids = doc.children(opt_id);
                for k in kids {
                    let cloned = doc.clone_node(k, true);
                    doc.append_child(sc_id, cloned);
                }
            }
        }
    }
}

fn collect_select_descendants(
    doc: &Document,
    parent: NodeId,
    selectedcontents: &mut Vec<NodeId>,
    options: &mut Vec<NodeId>,
) {
    for child in doc.children(parent) {
        if let Some(el) = doc.element(child) {
            if el.namespace == Namespace::Html {
                match el.tag_name.as_str() {
                    "select" | "template" => continue,
                    "selectedcontent" => {
                        selectedcontents.push(child);
                        continue;
                    }
                    "option" => {
                        options.push(child);
                        continue;
                    }
                    _ => {}
                }
            }
        }
        collect_select_descendants(doc, child, selectedcontents, options);
    }
}

fn should_process_in_foreign(st: &TreeState, token: &Token) -> bool {
    if st.open_elements.is_empty() {
        return false;
    }
    let Some(adj) = st.adjusted_current_node() else {
        return false;
    };
    if st.node_ns(adj) == Namespace::Html {
        return false;
    }
    if matches!(token, Token::Eof) {
        return false;
    }
    if is_mathml_text_integration_point(&st.doc, adj) {
        match token {
            Token::Character(_) => return false,
            Token::StartTag(t) if !matches!(t.name.as_str(), "mglyph" | "malignmark") => {
                return false;
            }
            _ => {}
        }
    }
    if st.node_ns(adj) == Namespace::MathMl && st.node_tag(adj) == "annotation-xml" {
        if let Token::StartTag(t) = token {
            if t.name == "svg" {
                return false;
            }
        }
    }
    if is_html_integration_point(&st.doc, adj) {
        if matches!(token, Token::StartTag(_) | Token::Character(_)) {
            return false;
        }
    }
    true
}

fn dispatch_token(st: &mut TreeState, token: &Token, tok_state: &mut TokenizerState) {
    if should_process_in_foreign(st, token) {
        if process_foreign_token(st, token, tok_state) {
            return;
        }
    }

    let mut active_mode = st.mode;
    for _ in 0..16 {
        let action = match active_mode {
            Mode::InBody => step_in_body(st, token, tok_state),
            Mode::InTable
            | Mode::InTableText
            | Mode::InCaption
            | Mode::InColumnGroup
            | Mode::InTableBody
            | Mode::InRow
            | Mode::InCell => step_table_modes(st, token, tok_state),
            _ => step_head_and_outer(st, token, tok_state),
        };
        match action {
            StepAction::Done => {
                st.foster_parenting = false;
                return;
            }
            StepAction::Reprocess(next) => {
                active_mode = next;
            }
            StepAction::ProcessAsInHead => {
                let res = step_in_head(st, token, tok_state);
                match res {
                    StepAction::Done => {
                        st.foster_parenting = false;
                        return;
                    }
                    StepAction::Reprocess(next) => active_mode = next,
                    _ => return,
                }
            }
            StepAction::ProcessAsInBody => {
                let res = step_in_body(st, token, tok_state);
                match res {
                    StepAction::Done => {
                        st.foster_parenting = false;
                        return;
                    }
                    StepAction::Reprocess(next) => {
                        st.foster_parenting = false;
                        active_mode = next;
                    }
                    StepAction::ProcessAsInHead => {
                        let _ = step_in_head(st, token, tok_state);
                        st.foster_parenting = false;
                        return;
                    }
                    StepAction::ProcessAsInBody => {
                        st.foster_parenting = false;
                        return;
                    }
                }
            }
        }
    }
    st.foster_parenting = false;
}

fn process_foreign_token(
    st: &mut TreeState,
    token: &Token,
    _tok_state: &mut TokenizerState,
) -> bool {
    match token {
        Token::Character('\0') => {
            st.insert_char('\u{FFFD}');
            true
        }
        Token::Character(c) => {
            st.insert_char(*c);
            if !is_ascii_ws(*c) {
                st.frameset_ok = false;
            }
            true
        }
        Token::Comment(data) => {
            st.insert_comment(data.clone(), None);
            true
        }
        Token::Doctype(_) => true,
        Token::StartTag(t) => {
            if breaks_out_of_foreign(&t.name, &t.attributes) {
                while let Some(cur) = st.current_node() {
                    if st.node_ns(cur) == Namespace::Html
                        || is_mathml_text_integration_point(&st.doc, cur)
                        || is_html_integration_point(&st.doc, cur)
                    {
                        break;
                    }
                    st.open_elements.pop();
                }
                return false;
            }
            let ns = st
                .adjusted_current_node()
                .map_or(Namespace::Html, |id| st.node_ns(id));
            insert_foreign_element(st, t, ns);
            true
        }
        Token::EndTag(t) => {
            if matches!(t.name.as_str(), "br" | "p") {
                while let Some(cur) = st.current_node() {
                    if st.node_ns(cur) == Namespace::Html
                        || is_mathml_text_integration_point(&st.doc, cur)
                        || is_html_integration_point(&st.doc, cur)
                    {
                        break;
                    }
                    st.open_elements.pop();
                }
                return false;
            }
            let mut idx = st.open_elements.len();
            while idx > 0 {
                idx -= 1;
                let id = st.open_elements[idx];
                if idx == st.open_elements.len() - 1
                    && st.node_tag(id).eq_ignore_ascii_case(&t.name)
                {
                    st.open_elements.pop();
                    return true;
                }
                if st.node_ns(id) == Namespace::Html {
                    return false;
                }
                if st.node_tag(id).eq_ignore_ascii_case(&t.name) {
                    st.open_elements.truncate(idx);
                    return true;
                }
            }
            true
        }
        Token::Eof => false,
    }
}
