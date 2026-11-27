use crate::dom::{Attribute, Namespace, QuirksMode};
use crate::tokenizer::{TagToken, Token, TokenizerState};
use crate::tree_core::{
    classify_doctype_quirks, is_ascii_ws, FormattingItem, Mode, TreeState,
};
use crate::tree_foreign::{
    adjust_foreign_attributes, adjust_mathml_attributes, adjust_svg_attributes,
    adjust_svg_tag_name,
};
use crate::tree_table::StepAction;

pub fn step_head_and_outer(
    st: &mut TreeState,
    token: &Token,
    tok_state: &mut TokenizerState,
) -> StepAction {
    match st.mode {
        Mode::Initial => step_initial(st, token),
        Mode::BeforeHtml => step_before_html(st, token),
        Mode::BeforeHead => step_before_head(st, token),
        Mode::InHead => step_in_head(st, token, tok_state),
        Mode::InHeadNoscript => step_in_head_noscript(st, token, tok_state),
        Mode::AfterHead => step_after_head(st, token, tok_state),
        Mode::Text => step_text(st, token),
        Mode::InSelect => step_in_select(st, token, tok_state),
        Mode::InSelectInTable => step_in_select_in_table(st, token, tok_state),
        Mode::InTemplate => step_in_template(st, token, tok_state),
        Mode::AfterBody => step_after_body(st, token),
        Mode::InFrameset => step_in_frameset(st, token),
        Mode::AfterFrameset => step_after_frameset(st, token),
        Mode::AfterAfterBody => step_after_after_body(st, token),
        Mode::AfterAfterFrameset => step_after_after_frameset(st, token),
        _ => StepAction::Done,
    }
}

fn step_initial(st: &mut TreeState, token: &Token) -> StepAction {
    match token {
        Token::Character(c) if is_ascii_ws(*c) => StepAction::Done,
        Token::Comment(data) => {
            let root = st.doc.root;
            st.insert_comment(data.clone(), Some(root));
            StepAction::Done
        }
        Token::Doctype(dt) => {
            let id = st.doc.create_doctype(
                dt.name.clone().unwrap_or_default(),
                dt.public_id.clone().unwrap_or_default(),
                dt.system_id.clone().unwrap_or_default(),
            );
            let root = st.doc.root;
            st.doc.append_child(root, id);
            st.doc.set_quirks_mode(classify_doctype_quirks(dt));
            st.mode = Mode::BeforeHtml;
            StepAction::Done
        }
        _ => {
            st.doc.set_quirks_mode(QuirksMode::Quirks);
            st.mode = Mode::BeforeHtml;
            StepAction::Reprocess(Mode::BeforeHtml)
        }
    }
}

fn step_before_html(st: &mut TreeState, token: &Token) -> StepAction {
    match token {
        Token::Doctype(_) => StepAction::Done,
        Token::Comment(data) => {
            let root = st.doc.root;
            st.insert_comment(data.clone(), Some(root));
            StepAction::Done
        }
        Token::Character(c) if is_ascii_ws(*c) => StepAction::Done,
        Token::StartTag(t) if t.name == "html" => {
            let id = st
                .doc
                .create_element_ns("html", Namespace::Html, t.attributes.clone());
            let root = st.doc.root;
            st.doc.append_child(root, id);
            st.open_elements.push(id);
            st.mode = Mode::BeforeHead;
            StepAction::Done
        }
        Token::EndTag(t)
            if !matches!(t.name.as_str(), "head" | "body" | "html" | "br") =>
        {
            StepAction::Done
        }
        _ => {
            let id = st
                .doc
                .create_element_ns("html", Namespace::Html, Vec::new());
            let root = st.doc.root;
            st.doc.append_child(root, id);
            st.open_elements.push(id);
            st.mode = Mode::BeforeHead;
            StepAction::Reprocess(Mode::BeforeHead)
        }
    }
}

fn step_before_head(st: &mut TreeState, token: &Token) -> StepAction {
    match token {
        Token::Character(c) if is_ascii_ws(*c) => StepAction::Done,
        Token::Comment(data) => {
            st.insert_comment(data.clone(), None);
            StepAction::Done
        }
        Token::Doctype(_) => StepAction::Done,
        Token::StartTag(t) if t.name == "html" => StepAction::ProcessAsInBody,
        Token::StartTag(t) if t.name == "head" => {
            let id = st.insert_html_element(t);
            st.head_element = Some(id);
            st.mode = Mode::InHead;
            StepAction::Done
        }
        Token::EndTag(t)
            if !matches!(t.name.as_str(), "head" | "body" | "html" | "br") =>
        {
            StepAction::Done
        }
        _ => {
            let head = TagToken {
                is_end: false,
                name: "head".to_string(),
                self_closing: false,
                attributes: Vec::new(),
            };
            let id = st.insert_html_element(&head);
            st.head_element = Some(id);
            st.mode = Mode::InHead;
            StepAction::Reprocess(Mode::InHead)
        }
    }
}

pub fn step_in_head(
    st: &mut TreeState,
    token: &Token,
    tok_state: &mut TokenizerState,
) -> StepAction {
    match token {
        Token::Character(c) if is_ascii_ws(*c) => {
            st.insert_char(*c);
            StepAction::Done
        }
        Token::Comment(data) => {
            st.insert_comment(data.clone(), None);
            StepAction::Done
        }
        Token::Doctype(_) => StepAction::Done,
        Token::StartTag(t) => match t.name.as_str() {
            "html" => StepAction::ProcessAsInBody,
            "base" | "basefont" | "bgsound" | "link" | "meta" => {
                st.insert_void_html(t);
                StepAction::Done
            }
            "title" => {
                st.parse_raw_element(t, TokenizerState::Rcdata, tok_state);
                StepAction::Done
            }
            "noscript" => {
                if st.scripting_enabled {
                    st.parse_raw_element(t, TokenizerState::Rawtext, tok_state);
                } else {
                    st.insert_html_element(t);
                    st.mode = Mode::InHeadNoscript;
                }
                StepAction::Done
            }
            "noframes" | "style" => {
                st.parse_raw_element(t, TokenizerState::Rawtext, tok_state);
                StepAction::Done
            }
            "script" => {
                st.parse_raw_element(t, TokenizerState::ScriptData, tok_state);
                StepAction::Done
            }
            "template" => {
                st.insert_html_element(t);
                st.active_formatting.push(FormattingItem::Marker);
                st.frameset_ok = false;
                st.mode = Mode::InTemplate;
                st.template_modes.push(Mode::InTemplate);
                StepAction::Done
            }
            "head" => StepAction::Done,
            _ => {
                st.open_elements.pop();
                st.mode = Mode::AfterHead;
                StepAction::Reprocess(Mode::AfterHead)
            }
        },
        Token::EndTag(t) => match t.name.as_str() {
            "head" => {
                st.open_elements.pop();
                st.mode = Mode::AfterHead;
                StepAction::Done
            }
            "body" | "html" | "br" => {
                st.open_elements.pop();
                st.mode = Mode::AfterHead;
                StepAction::Reprocess(Mode::AfterHead)
            }
            "template" => {
                if st.open_elements.iter().any(|&id| st.node_is_html(id, "template")) {
                    st.generate_all_implied_end_tags(None);
                    st.pop_until_html("template");
                    st.clear_formatting_to_marker();
                    st.template_modes.pop();
                    st.reset_insertion_mode();
                }
                StepAction::Done
            }
            _ => StepAction::Done,
        },
        _ => {
            st.open_elements.pop();
            st.mode = Mode::AfterHead;
            StepAction::Reprocess(Mode::AfterHead)
        }
    }
}

fn step_in_head_noscript(
    st: &mut TreeState,
    token: &Token,
    tok_state: &mut TokenizerState,
) -> StepAction {
    match token {
        Token::Doctype(_) => StepAction::Done,
        Token::StartTag(t) if t.name == "html" => StepAction::ProcessAsInBody,
        Token::EndTag(t) if t.name == "noscript" => {
            st.open_elements.pop();
            st.mode = Mode::InHead;
            StepAction::Done
        }
        Token::Character(c) if is_ascii_ws(*c) => step_in_head(st, token, tok_state),
        Token::Comment(_) => step_in_head(st, token, tok_state),
        Token::StartTag(t)
            if matches!(
                t.name.as_str(),
                "basefont" | "bgsound" | "link" | "meta" | "noframes" | "style"
            ) =>
        {
            step_in_head(st, token, tok_state)
        }
        Token::EndTag(t) if t.name == "br" => {
            st.open_elements.pop();
            st.mode = Mode::InHead;
            StepAction::Reprocess(Mode::InHead)
        }
        Token::StartTag(t) if matches!(t.name.as_str(), "head" | "noscript") => StepAction::Done,
        Token::EndTag(_) => StepAction::Done,
        _ => {
            st.open_elements.pop();
            st.mode = Mode::InHead;
            StepAction::Reprocess(Mode::InHead)
        }
    }
}

fn step_after_head(
    st: &mut TreeState,
    token: &Token,
    tok_state: &mut TokenizerState,
) -> StepAction {
    match token {
        Token::Character(c) if is_ascii_ws(*c) => {
            st.insert_char(*c);
            StepAction::Done
        }
        Token::Comment(data) => {
            st.insert_comment(data.clone(), None);
            StepAction::Done
        }
        Token::Doctype(_) => StepAction::Done,
        Token::StartTag(t) => match t.name.as_str() {
            "html" => StepAction::ProcessAsInBody,
            "body" => {
                st.insert_html_element(t);
                st.frameset_ok = false;
                st.mode = Mode::InBody;
                StepAction::Done
            }
            "frameset" => {
                st.insert_html_element(t);
                st.mode = Mode::InFrameset;
                StepAction::Done
            }
            "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script" | "style"
            | "template" | "title" => {
                if let Some(head_id) = st.head_element {
                    st.open_elements.push(head_id);
                    let act = step_in_head(st, token, tok_state);
                    if let Some(pos) = st.open_elements.iter().rposition(|&id| id == head_id) {
                        st.open_elements.remove(pos);
                    }
                    act
                } else {
                    StepAction::Done
                }
            }
            "head" => StepAction::Done,
            _ => {
                let body = TagToken {
                    is_end: false,
                    name: "body".to_string(),
                    self_closing: false,
                    attributes: Vec::new(),
                };
                st.insert_html_element(&body);
                st.mode = Mode::InBody;
                StepAction::Reprocess(Mode::InBody)
            }
        },
        Token::EndTag(t) if t.name == "template" => step_in_head(st, token, tok_state),
        Token::EndTag(t)
            if !matches!(t.name.as_str(), "body" | "html" | "br") =>
        {
            StepAction::Done
        }
        _ => {
            let body = TagToken {
                is_end: false,
                name: "body".to_string(),
                self_closing: false,
                attributes: Vec::new(),
            };
            st.insert_html_element(&body);
            st.mode = Mode::InBody;
            StepAction::Reprocess(Mode::InBody)
        }
    }
}

fn step_text(st: &mut TreeState, token: &Token) -> StepAction {
    match token {
        Token::Character(c) => {
            if st.ignore_leading_lf {
                st.ignore_leading_lf = false;
                if *c == '\n' {
                    return StepAction::Done;
                }
            }
            st.insert_char(*c);
            StepAction::Done
        }
        Token::Eof => {
            st.ignore_leading_lf = false;
            st.open_elements.pop();
            st.mode = st.original_mode;
            StepAction::Reprocess(st.mode)
        }
        Token::EndTag(_) => {
            st.ignore_leading_lf = false;
            st.open_elements.pop();
            st.mode = st.original_mode;
            StepAction::Done
        }
        _ => StepAction::Done,
    }
}

pub fn step_in_body(
    st: &mut TreeState,
    token: &Token,
    tok_state: &mut TokenizerState,
) -> StepAction {
    match token {
        Token::Character('\0') => StepAction::Done,
        Token::Character(c) => {
            if st.ignore_leading_lf {
                st.ignore_leading_lf = false;
                if *c == '\n' {
                    return StepAction::Done;
                }
            }
            st.reconstruct_formatting();
            st.insert_char(*c);
            if !is_ascii_ws(*c) {
                st.frameset_ok = false;
            }
            StepAction::Done
        }
        Token::Comment(data) => {
            st.insert_comment(data.clone(), None);
            StepAction::Done
        }
        Token::Doctype(_) => StepAction::Done,
        Token::Eof => {
            if !st.template_modes.is_empty() {
                return step_in_template(st, token, tok_state);
            }
            StepAction::Done
        }
        Token::StartTag(t) => step_in_body_start(st, t, tok_state),
        Token::EndTag(t) => step_in_body_end(st, t),
    }
}

fn step_in_body_start(
    st: &mut TreeState,
    t: &TagToken,
    tok_state: &mut TokenizerState,
) -> StepAction {
    st.ignore_leading_lf = false;
    match t.name.as_str() {
        "html" => {
            let has_tpl = st.open_elements.iter().any(|&id| st.node_is_html(id, "template"));
            if !has_tpl {
                if let Some(&first) = st.open_elements.first() {
                    if let Some(el) = st.doc.element_mut(first) {
                        for a in &t.attributes {
                            if !el.has_attr(&a.name) {
                                el.attributes.push(a.clone());
                            }
                        }
                    }
                }
            }
            StepAction::Done
        }
        "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script" | "style"
        | "template" | "title" => StepAction::ProcessAsInHead,
        "body" => {
            if st.open_elements.len() >= 2
                && st.node_is_html(st.open_elements[1], "body")
                && !st.open_elements.iter().any(|&id| st.node_is_html(id, "template"))
            {
                st.frameset_ok = false;
                let body_id = st.open_elements[1];
                if let Some(el) = st.doc.element_mut(body_id) {
                    for a in &t.attributes {
                        if !el.has_attr(&a.name) {
                            el.attributes.push(a.clone());
                        }
                    }
                }
            }
            StepAction::Done
        }
        "frameset" => {
            if st.open_elements.len() >= 2
                && st.node_is_html(st.open_elements[1], "body")
                && st.frameset_ok
            {
                let body_id = st.open_elements[1];
                st.doc.detach(body_id);
                st.open_elements.truncate(1);
                st.insert_html_element(t);
                st.mode = Mode::InFrameset;
            }
            StepAction::Done
        }
        "address" | "article" | "aside" | "blockquote" | "center" | "details" | "dialog"
        | "dir" | "div" | "dl" | "fieldset" | "figcaption" | "figure" | "footer" | "header"
        | "hgroup" | "main" | "menu" | "nav" | "ol" | "p" | "search" | "section" | "summary"
        | "ul" => {
            if st.in_button_scope("p") {
                st.close_p_element();
            }
            st.insert_html_element(t);
            StepAction::Done
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            if st.in_button_scope("p") {
                st.close_p_element();
            }
            if let Some(cur) = st.current_node() {
                if st.node_ns(cur) == Namespace::Html
                    && matches!(st.node_tag(cur), "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
                {
                    st.open_elements.pop();
                }
            }
            st.insert_html_element(t);
            StepAction::Done
        }
        "pre" | "listing" => {
            if st.in_button_scope("p") {
                st.close_p_element();
            }
            st.insert_html_element(t);
            st.ignore_leading_lf = true;
            st.frameset_ok = false;
            StepAction::Done
        }
        "form" => {
            let has_tpl = st.open_elements.iter().any(|&id| st.node_is_html(id, "template"));
            if st.form_element.is_some() && !has_tpl {
                return StepAction::Done;
            }
            if st.in_button_scope("p") {
                st.close_p_element();
            }
            let id = st.insert_html_element(t);
            if !has_tpl {
                st.form_element = Some(id);
            }
            StepAction::Done
        }
        "li" => {
            st.frameset_ok = false;
            for &id in st.open_elements.iter().rev() {
                if st.node_is_html(id, "li") {
                    st.generate_implied_end_tags(Some("li"));
                    st.pop_until_html("li");
                    break;
                }
                if st.is_special_element(id)
                    && !matches!(st.node_tag(id), "address" | "div" | "p")
                {
                    break;
                }
            }
            if st.in_button_scope("p") {
                st.close_p_element();
            }
            st.insert_html_element(t);
            StepAction::Done
        }
        "dd" | "dt" => {
            st.frameset_ok = false;
            for &id in st.open_elements.iter().rev() {
                if st.node_is_html(id, "dd") {
                    st.generate_implied_end_tags(Some("dd"));
                    st.pop_until_html("dd");
                    break;
                }
                if st.node_is_html(id, "dt") {
                    st.generate_implied_end_tags(Some("dt"));
                    st.pop_until_html("dt");
                    break;
                }
                if st.is_special_element(id)
                    && !matches!(st.node_tag(id), "address" | "div" | "p")
                {
                    break;
                }
            }
            if st.in_button_scope("p") {
                st.close_p_element();
            }
            st.insert_html_element(t);
            StepAction::Done
        }
        "plaintext" => {
            if st.in_button_scope("p") {
                st.close_p_element();
            }
            st.insert_html_element(t);
            *tok_state = TokenizerState::Plaintext;
            StepAction::Done
        }
        "button" => {
            if st.in_scope("button") {
                st.generate_implied_end_tags(None);
                st.pop_until_html("button");
            }
            st.reconstruct_formatting();
            st.insert_html_element(t);
            st.frameset_ok = false;
            StepAction::Done
        }
        "a" => {
            let mut existing_a = None;
            for item in st.active_formatting.iter().rev() {
                match item {
                    FormattingItem::Marker => break,
                    FormattingItem::Element { node, tag, .. } if tag == "a" => {
                        existing_a = Some(*node);
                        break;
                    }
                    _ => {}
                }
            }
            if let Some(a_node) = existing_a {
                st.run_adoption_agency("a");
                if let Some(pos) = st.active_formatting.iter().position(|item| match item {
                    FormattingItem::Element { node, .. } => *node == a_node,
                    FormattingItem::Marker => false,
                }) {
                    st.active_formatting.remove(pos);
                }
                if let Some(pos) = st.open_elements.iter().rposition(|&id| id == a_node) {
                    st.open_elements.remove(pos);
                }
            }
            st.reconstruct_formatting();
            let id = st.insert_html_element(t);
            st.push_formatting(id, "a".to_string(), t.attributes.clone());
            StepAction::Done
        }
        "b" | "big" | "code" | "em" | "font" | "i" | "s" | "small" | "strike" | "strong"
        | "tt" | "u" => {
            st.reconstruct_formatting();
            let id = st.insert_html_element(t);
            st.push_formatting(id, t.name.clone(), t.attributes.clone());
            StepAction::Done
        }
        "nobr" => {
            st.reconstruct_formatting();
            if st.in_scope("nobr") {
                st.run_adoption_agency("nobr");
                st.reconstruct_formatting();
            }
            let id = st.insert_html_element(t);
            st.push_formatting(id, "nobr".to_string(), t.attributes.clone());
            StepAction::Done
        }
        "applet" | "marquee" | "object" => {
            st.reconstruct_formatting();
            st.insert_html_element(t);
            st.active_formatting.push(FormattingItem::Marker);
            st.frameset_ok = false;
            StepAction::Done
        }
        "table" => {
            if st.doc.quirks_mode() != QuirksMode::Quirks && st.in_button_scope("p") {
                st.close_p_element();
            }
            st.insert_html_element(t);
            st.frameset_ok = false;
            st.mode = Mode::InTable;
            StepAction::Done
        }
        "area" | "br" | "embed" | "img" | "keygen" | "wbr" => {
            st.reconstruct_formatting();
            st.insert_void_html(t);
            st.frameset_ok = false;
            StepAction::Done
        }
        "input" => {
            st.reconstruct_formatting();
            st.insert_void_html(t);
            let is_hidden = t
                .attributes
                .iter()
                .any(|a| a.name == "type" && a.value.eq_ignore_ascii_case("hidden"));
            if !is_hidden {
                st.frameset_ok = false;
            }
            StepAction::Done
        }
        "param" | "source" | "track" => {
            st.insert_void_html(t);
            StepAction::Done
        }
        "hr" => {
            if st.in_button_scope("p") {
                st.close_p_element();
            }
            st.insert_void_html(t);
            st.frameset_ok = false;
            StepAction::Done
        }
        "image" => {
            let mut img_tok = t.clone();
            img_tok.name = "img".to_string();
            st.reconstruct_formatting();
            st.insert_void_html(&img_tok);
            st.frameset_ok = false;
            StepAction::Done
        }
        "textarea" => {
            st.insert_html_element(t);
            st.ignore_leading_lf = true;
            *tok_state = TokenizerState::Rcdata;
            st.original_mode = st.mode;
            st.frameset_ok = false;
            st.mode = Mode::Text;
            StepAction::Done
        }
        "xmp" => {
            if st.in_button_scope("p") {
                st.close_p_element();
            }
            st.reconstruct_formatting();
            st.frameset_ok = false;
            st.parse_raw_element(t, TokenizerState::Rawtext, tok_state);
            StepAction::Done
        }
        "iframe" => {
            st.frameset_ok = false;
            st.parse_raw_element(t, TokenizerState::Rawtext, tok_state);
            StepAction::Done
        }
        "noembed" => {
            st.parse_raw_element(t, TokenizerState::Rawtext, tok_state);
            StepAction::Done
        }
        "noscript" if st.scripting_enabled => {
            st.parse_raw_element(t, TokenizerState::Rawtext, tok_state);
            StepAction::Done
        }
        "select" => {
            st.reconstruct_formatting();
            st.insert_html_element(t);
            st.frameset_ok = false;
            if matches!(
                st.mode,
                Mode::InTable | Mode::InCaption | Mode::InTableBody | Mode::InRow | Mode::InCell
            ) {
                st.mode = Mode::InSelectInTable;
            } else {
                st.mode = Mode::InSelect;
            }
            StepAction::Done
        }
        "optgroup" | "option" => {
            if st.current_is_html("option") {
                st.open_elements.pop();
            }
            st.reconstruct_formatting();
            st.insert_html_element(t);
            StepAction::Done
        }
        "rb" | "rtc" => {
            if st.in_scope("ruby") {
                st.generate_implied_end_tags(None);
            }
            st.insert_html_element(t);
            StepAction::Done
        }
        "rp" | "rt" => {
            if st.in_scope("ruby") {
                st.generate_implied_end_tags(Some("rtc"));
            }
            st.insert_html_element(t);
            StepAction::Done
        }
        "math" => {
            st.reconstruct_formatting();
            let mut attrs = t.attributes.clone();
            adjust_mathml_attributes(&mut attrs);
            adjust_foreign_attributes(&mut attrs);
            st.insert_element("math", Namespace::MathMl, attrs);
            if t.self_closing {
                st.open_elements.pop();
            }
            StepAction::Done
        }
        "svg" => {
            st.reconstruct_formatting();
            let mut attrs = t.attributes.clone();
            adjust_svg_attributes(&mut attrs);
            adjust_foreign_attributes(&mut attrs);
            st.insert_element("svg", Namespace::Svg, attrs);
            if t.self_closing {
                st.open_elements.pop();
            }
            StepAction::Done
        }
        "caption" | "col" | "colgroup" | "frame" | "head" | "tbody" | "td" | "tfoot" | "th"
        | "thead" | "tr" => StepAction::Done,
        _ => {
            st.reconstruct_formatting();
            st.insert_html_element(t);
            StepAction::Done
        }
    }
}

fn step_in_body_end(st: &mut TreeState, t: &TagToken) -> StepAction {
    match t.name.as_str() {
        "template" => StepAction::ProcessAsInHead,
        "body" => {
            if st.in_scope("body") {
                st.mode = Mode::AfterBody;
            }
            StepAction::Done
        }
        "html" => {
            if st.in_scope("body") {
                st.mode = Mode::AfterBody;
                return StepAction::Reprocess(Mode::AfterBody);
            }
            StepAction::Done
        }
        "address" | "article" | "aside" | "blockquote" | "button" | "center" | "details"
        | "dialog" | "dir" | "div" | "dl" | "fieldset" | "figcaption" | "figure" | "footer"
        | "header" | "hgroup" | "listing" | "main" | "menu" | "nav" | "ol" | "pre" | "search"
        | "section" | "summary" | "ul" => {
            if st.in_scope(&t.name) {
                st.generate_implied_end_tags(None);
                st.pop_until_html(&t.name);
            }
            StepAction::Done
        }
        "form" => {
            let has_tpl = st.open_elements.iter().any(|&id| st.node_is_html(id, "template"));
            if !has_tpl {
                let node = st.form_element.take();
                if let Some(form_id) = node {
                    if st.in_scope("form") {
                        st.generate_implied_end_tags(None);
                        if let Some(pos) = st.open_elements.iter().rposition(|&id| id == form_id) {
                            st.open_elements.remove(pos);
                        }
                    }
                }
            } else if st.in_scope("form") {
                st.generate_implied_end_tags(None);
                st.pop_until_html("form");
            }
            StepAction::Done
        }
        "p" => {
            if !st.in_button_scope("p") {
                let p_tok = TagToken {
                    is_end: false,
                    name: "p".to_string(),
                    self_closing: false,
                    attributes: Vec::new(),
                };
                st.insert_html_element(&p_tok);
            }
            st.close_p_element();
            StepAction::Done
        }
        "li" => {
            if st.in_list_item_scope("li") {
                st.generate_implied_end_tags(Some("li"));
                st.pop_until_html("li");
            }
            StepAction::Done
        }
        "dd" | "dt" => {
            if st.in_scope(&t.name) {
                st.generate_implied_end_tags(Some(&t.name));
                st.pop_until_html(&t.name);
            }
            StepAction::Done
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            if st.has_heading_in_scope() {
                st.generate_implied_end_tags(None);
                while let Some(id) = st.open_elements.pop() {
                    if st.node_ns(id) == Namespace::Html
                        && matches!(st.node_tag(id), "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
                    {
                        break;
                    }
                }
            }
            StepAction::Done
        }
        "a" | "b" | "big" | "code" | "em" | "font" | "i" | "nobr" | "s" | "small" | "strike"
        | "strong" | "tt" | "u" => {
            st.run_adoption_agency(&t.name);
            StepAction::Done
        }
        "applet" | "marquee" | "object" => {
            if st.in_scope(&t.name) {
                st.generate_implied_end_tags(None);
                st.pop_until_html(&t.name);
                st.clear_formatting_to_marker();
            }
            StepAction::Done
        }
        "br" => {
            st.reconstruct_formatting();
            let br_tok = TagToken {
                is_end: false,
                name: "br".to_string(),
                self_closing: false,
                attributes: Vec::new(),
            };
            st.insert_void_html(&br_tok);
            st.frameset_ok = false;
            StepAction::Done
        }
        _ => {
            st.any_other_end_tag(&t.name);
            StepAction::Done
        }
    }
}

fn step_in_select(
    st: &mut TreeState,
    token: &Token,
    _tok_state: &mut TokenizerState,
) -> StepAction {
    match token {
        Token::Character('\0') => StepAction::Done,
        Token::Character(c) => {
            st.insert_char(*c);
            StepAction::Done
        }
        Token::Comment(data) => {
            st.insert_comment(data.clone(), None);
            StepAction::Done
        }
        Token::Doctype(_) => StepAction::Done,
        Token::StartTag(t) => match t.name.as_str() {
            "html" => StepAction::ProcessAsInBody,
            "option" => {
                if st.current_is_html("option") {
                    st.open_elements.pop();
                }
                st.insert_html_element(t);
                StepAction::Done
            }
            "optgroup" => {
                if st.current_is_html("option") {
                    st.open_elements.pop();
                }
                if st.current_is_html("optgroup") {
                    st.open_elements.pop();
                }
                st.insert_html_element(t);
                StepAction::Done
            }
            "hr" => {
                if st.current_is_html("option") {
                    st.open_elements.pop();
                }
                if st.current_is_html("optgroup") {
                    st.open_elements.pop();
                }
                st.insert_void_html(t);
                StepAction::Done
            }
            "select" => {
                if st.in_select_scope("select") {
                    st.pop_until_html("select");
                    st.reset_insertion_mode();
                }
                StepAction::Done
            }
            "input" | "keygen" | "textarea" => {
                if st.in_select_scope("select") {
                    st.pop_until_html("select");
                    st.reset_insertion_mode();
                    return StepAction::Reprocess(st.mode);
                }
                StepAction::Done
            }
            "script" | "template" => StepAction::ProcessAsInHead,
            _ => StepAction::ProcessAsInBody,
        },
        Token::EndTag(t) => match t.name.as_str() {
            "optgroup" => {
                if st.current_is_html("option") && st.open_elements.len() >= 2 {
                    let prev = st.open_elements[st.open_elements.len() - 2];
                    if st.node_is_html(prev, "optgroup") {
                        st.open_elements.pop();
                    }
                }
                if st.current_is_html("optgroup") {
                    st.open_elements.pop();
                }
                StepAction::Done
            }
            "option" => {
                if st.current_is_html("option") {
                    st.open_elements.pop();
                }
                StepAction::Done
            }
            "select" => {
                if st.in_select_scope("select") {
                    st.pop_until_html("select");
                    st.reset_insertion_mode();
                }
                StepAction::Done
            }
            "template" => StepAction::ProcessAsInHead,
            _ => StepAction::ProcessAsInBody,
        },
        Token::Eof => StepAction::ProcessAsInBody,
    }
}

fn step_in_select_in_table(
    st: &mut TreeState,
    token: &Token,
    tok_state: &mut TokenizerState,
) -> StepAction {
    match token {
        Token::StartTag(t)
            if matches!(
                t.name.as_str(),
                "caption" | "table" | "tbody" | "tfoot" | "thead" | "tr" | "td" | "th"
            ) =>
        {
            st.pop_until_html("select");
            st.reset_insertion_mode();
            StepAction::Reprocess(st.mode)
        }
        Token::EndTag(t)
            if matches!(
                t.name.as_str(),
                "caption" | "table" | "tbody" | "tfoot" | "thead" | "tr" | "td" | "th"
            ) =>
        {
            if st.in_table_scope(&t.name) {
                st.pop_until_html("select");
                st.reset_insertion_mode();
                StepAction::Reprocess(st.mode)
            } else {
                StepAction::Done
            }
        }
        _ => step_in_select(st, token, tok_state),
    }
}

fn step_in_template(
    st: &mut TreeState,
    token: &Token,
    _tok_state: &mut TokenizerState,
) -> StepAction {
    match token {
        Token::Character(_) | Token::Comment(_) | Token::Doctype(_) => StepAction::ProcessAsInBody,
        Token::StartTag(t) => match t.name.as_str() {
            "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script" | "style"
            | "template" | "title" => StepAction::ProcessAsInHead,
            "caption" | "colgroup" | "tbody" | "tfoot" | "thead" => {
                st.template_modes.pop();
                st.template_modes.push(Mode::InTable);
                st.mode = Mode::InTable;
                StepAction::Reprocess(Mode::InTable)
            }
            "col" => {
                st.template_modes.pop();
                st.template_modes.push(Mode::InColumnGroup);
                st.mode = Mode::InColumnGroup;
                StepAction::Reprocess(Mode::InColumnGroup)
            }
            "tr" => {
                st.template_modes.pop();
                st.template_modes.push(Mode::InTableBody);
                st.mode = Mode::InTableBody;
                StepAction::Reprocess(Mode::InTableBody)
            }
            "td" | "th" => {
                st.template_modes.pop();
                st.template_modes.push(Mode::InRow);
                st.mode = Mode::InRow;
                StepAction::Reprocess(Mode::InRow)
            }
            _ => {
                st.template_modes.pop();
                st.template_modes.push(Mode::InBody);
                st.mode = Mode::InBody;
                StepAction::Reprocess(Mode::InBody)
            }
        },
        Token::EndTag(t) if t.name == "template" => StepAction::ProcessAsInHead,
        Token::EndTag(_) => StepAction::Done,
        Token::Eof => {
            if !st.open_elements.iter().any(|&id| st.node_is_html(id, "template")) {
                return StepAction::Done;
            }
            st.pop_until_html("template");
            st.clear_formatting_to_marker();
            st.template_modes.pop();
            st.reset_insertion_mode();
            StepAction::Reprocess(st.mode)
        }
    }
}

fn step_after_body(st: &mut TreeState, token: &Token) -> StepAction {
    match token {
        Token::Character(c) if is_ascii_ws(*c) => StepAction::ProcessAsInBody,
        Token::Comment(data) => {
            let target = st.open_elements.first().copied();
            st.insert_comment(data.clone(), target);
            StepAction::Done
        }
        Token::Doctype(_) => StepAction::Done,
        Token::StartTag(t) if t.name == "html" => StepAction::ProcessAsInBody,
        Token::EndTag(t) if t.name == "html" => {
            if st.fragment_context.is_none() {
                st.mode = Mode::AfterAfterBody;
            }
            StepAction::Done
        }
        Token::Eof => StepAction::Done,
        _ => {
            st.mode = Mode::InBody;
            StepAction::Reprocess(Mode::InBody)
        }
    }
}

fn step_in_frameset(st: &mut TreeState, token: &Token) -> StepAction {
    match token {
        Token::Character(c) if is_ascii_ws(*c) => {
            st.insert_char(*c);
            StepAction::Done
        }
        Token::Comment(data) => {
            st.insert_comment(data.clone(), None);
            StepAction::Done
        }
        Token::Doctype(_) => StepAction::Done,
        Token::StartTag(t) => match t.name.as_str() {
            "html" => StepAction::ProcessAsInBody,
            "frameset" => {
                st.insert_html_element(t);
                StepAction::Done
            }
            "frame" => {
                st.insert_void_html(t);
                StepAction::Done
            }
            "noframes" => StepAction::ProcessAsInHead,
            _ => StepAction::Done,
        },
        Token::EndTag(t) if t.name == "frameset" => {
            if st.open_elements.len() > 1 {
                st.open_elements.pop();
                if st.fragment_context.is_none() && !st.current_is_html("frameset") {
                    st.mode = Mode::AfterFrameset;
                }
            }
            StepAction::Done
        }
        _ => StepAction::Done,
    }
}

fn step_after_frameset(st: &mut TreeState, token: &Token) -> StepAction {
    match token {
        Token::Character(c) if is_ascii_ws(*c) => {
            st.insert_char(*c);
            StepAction::Done
        }
        Token::Comment(data) => {
            st.insert_comment(data.clone(), None);
            StepAction::Done
        }
        Token::Doctype(_) => StepAction::Done,
        Token::StartTag(t) if t.name == "html" => StepAction::ProcessAsInBody,
        Token::EndTag(t) if t.name == "html" => {
            st.mode = Mode::AfterAfterFrameset;
            StepAction::Done
        }
        Token::StartTag(t) if t.name == "noframes" => StepAction::ProcessAsInHead,
        _ => StepAction::Done,
    }
}

fn step_after_after_body(st: &mut TreeState, token: &Token) -> StepAction {
    match token {
        Token::Comment(data) => {
            let root = st.doc.root;
            st.insert_comment(data.clone(), Some(root));
            StepAction::Done
        }
        Token::Doctype(_) => StepAction::ProcessAsInBody,
        Token::Character(c) if is_ascii_ws(*c) => StepAction::ProcessAsInBody,
        Token::StartTag(t) if t.name == "html" => StepAction::ProcessAsInBody,
        Token::Eof => StepAction::Done,
        _ => {
            st.mode = Mode::InBody;
            StepAction::Reprocess(Mode::InBody)
        }
    }
}

fn step_after_after_frameset(st: &mut TreeState, token: &Token) -> StepAction {
    match token {
        Token::Comment(data) => {
            let root = st.doc.root;
            st.insert_comment(data.clone(), Some(root));
            StepAction::Done
        }
        Token::Doctype(_) => StepAction::ProcessAsInBody,
        Token::Character(c) if is_ascii_ws(*c) => StepAction::ProcessAsInBody,
        Token::StartTag(t) if t.name == "html" => StepAction::ProcessAsInBody,
        Token::StartTag(t) if t.name == "noframes" => StepAction::ProcessAsInHead,
        _ => StepAction::Done,
    }
}

pub fn insert_foreign_element(st: &mut TreeState, tag: &TagToken, ns: Namespace) {
    let mut attrs: Vec<Attribute> = tag.attributes.clone();
    let name = match ns {
        Namespace::Svg => {
            adjust_svg_attributes(&mut attrs);
            adjust_svg_tag_name(&tag.name)
        }
        Namespace::MathMl => {
            adjust_mathml_attributes(&mut attrs);
            tag.name.clone()
        }
        Namespace::Html => tag.name.clone(),
    };
    adjust_foreign_attributes(&mut attrs);
    st.insert_element(&name, ns, attrs);
    if tag.self_closing {
        st.open_elements.pop();
    }
}
