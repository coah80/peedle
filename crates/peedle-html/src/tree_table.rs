use crate::dom::Namespace;
use crate::tokenizer::{TagToken, Token, TokenizerState};
use crate::tree_core::{is_ascii_ws, FormattingItem, Mode, TreeState};

pub enum StepAction {
    Done,
    Reprocess(Mode),
    ProcessAsInHead,
    ProcessAsInBody,
}

pub fn step_table_modes(
    st: &mut TreeState,
    token: &Token,
    tok_state: &mut TokenizerState,
) -> StepAction {
    match st.mode {
        Mode::InTable => step_in_table(st, token, tok_state),
        Mode::InTableText => step_in_table_text(st, token),
        Mode::InCaption => step_in_caption(st, token),
        Mode::InColumnGroup => step_in_colgroup(st, token),
        Mode::InTableBody => step_in_table_body(st, token),
        Mode::InRow => step_in_row(st, token),
        Mode::InCell => step_in_cell(st, token),
        _ => StepAction::Done,
    }
}

fn clear_stack_to_table_context(st: &mut TreeState) {
    while let Some(id) = st.current_node() {
        if st.node_is_html(id, "table")
            || st.node_is_html(id, "template")
            || st.node_is_html(id, "html")
        {
            break;
        }
        st.open_elements.pop();
    }
}

fn clear_stack_to_table_body_context(st: &mut TreeState) {
    while let Some(id) = st.current_node() {
        if st.node_is_html(id, "tbody")
            || st.node_is_html(id, "tfoot")
            || st.node_is_html(id, "thead")
            || st.node_is_html(id, "template")
            || st.node_is_html(id, "html")
        {
            break;
        }
        st.open_elements.pop();
    }
}

fn clear_stack_to_table_row_context(st: &mut TreeState) {
    while let Some(id) = st.current_node() {
        if st.node_is_html(id, "tr")
            || st.node_is_html(id, "template")
            || st.node_is_html(id, "html")
        {
            break;
        }
        st.open_elements.pop();
    }
}

fn step_in_table(
    st: &mut TreeState,
    token: &Token,
    _tok_state: &mut TokenizerState,
) -> StepAction {
    if let Token::Character(_) = token {
        if let Some(cur) = st.current_node() {
            if st.node_ns(cur) == Namespace::Html
                && matches!(
                    st.node_tag(cur),
                    "table" | "tbody" | "tfoot" | "thead" | "tr"
                )
            {
                st.pending_table_chars.clear();
                st.original_mode = st.mode;
                st.mode = Mode::InTableText;
                return StepAction::Reprocess(Mode::InTableText);
            }
        }
    }

    match token {
        Token::Comment(data) => {
            st.insert_comment(data.clone(), None);
            StepAction::Done
        }
        Token::Doctype(_) => StepAction::Done,
        Token::StartTag(t) => match t.name.as_str() {
            "caption" => {
                clear_stack_to_table_context(st);
                st.active_formatting.push(FormattingItem::Marker);
                st.insert_html_element(t);
                st.mode = Mode::InCaption;
                StepAction::Done
            }
            "colgroup" => {
                clear_stack_to_table_context(st);
                st.insert_html_element(t);
                st.mode = Mode::InColumnGroup;
                StepAction::Done
            }
            "col" => {
                clear_stack_to_table_context(st);
                let cg = TagToken {
                    is_end: false,
                    name: "colgroup".to_string(),
                    self_closing: false,
                    attributes: Vec::new(),
                };
                st.insert_html_element(&cg);
                st.mode = Mode::InColumnGroup;
                StepAction::Reprocess(Mode::InColumnGroup)
            }
            "tbody" | "tfoot" | "thead" => {
                clear_stack_to_table_context(st);
                st.insert_html_element(t);
                st.mode = Mode::InTableBody;
                StepAction::Done
            }
            "td" | "th" | "tr" => {
                clear_stack_to_table_context(st);
                let tb = TagToken {
                    is_end: false,
                    name: "tbody".to_string(),
                    self_closing: false,
                    attributes: Vec::new(),
                };
                st.insert_html_element(&tb);
                st.mode = Mode::InTableBody;
                StepAction::Reprocess(Mode::InTableBody)
            }
            "table" => {
                if !st.in_table_scope("table") {
                    return StepAction::Done;
                }
                st.pop_until_html("table");
                st.reset_insertion_mode();
                StepAction::Reprocess(st.mode)
            }
            "style" | "script" | "template" => StepAction::ProcessAsInHead,
            "input" => {
                let is_hidden = t
                    .attributes
                    .iter()
                    .any(|a| a.name == "type" && a.value.eq_ignore_ascii_case("hidden"));
                if is_hidden {
                    st.insert_void_html(t);
                    StepAction::Done
                } else {
                    foster_in_body(st)
                }
            }
            "form" => {
                let has_template = st.open_elements.iter().any(|&id| st.node_is_html(id, "template"));
                if st.form_element.is_none() && !has_template {
                    let id = st.insert_html_element(t);
                    st.form_element = Some(id);
                    st.open_elements.pop();
                }
                StepAction::Done
            }
            _ => foster_in_body(st),
        },
        Token::EndTag(t) => match t.name.as_str() {
            "table" => {
                if st.in_table_scope("table") {
                    st.pop_until_html("table");
                    st.reset_insertion_mode();
                }
                StepAction::Done
            }
            "body" | "caption" | "col" | "colgroup" | "html" | "tbody" | "td" | "tfoot" | "th"
            | "thead" | "tr" => StepAction::Done,
            "template" => StepAction::ProcessAsInHead,
            _ => foster_in_body(st),
        },
        Token::Eof => StepAction::ProcessAsInBody,
        Token::Character(_) => foster_in_body(st),
    }
}

fn foster_in_body(st: &mut TreeState) -> StepAction {
    st.foster_parenting = true;
    StepAction::ProcessAsInBody
}

fn step_in_table_text(st: &mut TreeState, token: &Token) -> StepAction {
    match token {
        Token::Character('\0') => StepAction::Done,
        Token::Character(c) => {
            st.pending_table_chars.push(*c);
            StepAction::Done
        }
        _ => {
            let pending = std::mem::take(&mut st.pending_table_chars);
            let has_non_ws = pending.chars().any(|c| !is_ascii_ws(c));
            if has_non_ws {
                st.foster_parenting = true;
                for c in pending.chars() {
                    if c != '\0' {
                        st.reconstruct_formatting();
                        st.insert_char(c);
                        if !is_ascii_ws(c) {
                            st.frameset_ok = false;
                        }
                    }
                }
                st.foster_parenting = false;
            } else if !pending.is_empty() {
                st.insert_str(&pending);
            }
            st.mode = st.original_mode;
            StepAction::Reprocess(st.mode)
        }
    }
}

fn step_in_caption(st: &mut TreeState, token: &Token) -> StepAction {
    match token {
        Token::EndTag(t) if t.name == "caption" => {
            if st.in_table_scope("caption") {
                st.generate_implied_end_tags(None);
                st.pop_until_html("caption");
                st.clear_formatting_to_marker();
                st.mode = Mode::InTable;
            }
            StepAction::Done
        }
        Token::StartTag(t)
            if matches!(
                t.name.as_str(),
                "caption" | "col" | "colgroup" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr"
            ) =>
        {
            if st.in_table_scope("caption") {
                st.generate_implied_end_tags(None);
                st.pop_until_html("caption");
                st.clear_formatting_to_marker();
                st.mode = Mode::InTable;
                StepAction::Reprocess(Mode::InTable)
            } else {
                StepAction::Done
            }
        }
        Token::EndTag(t) if t.name == "table" => {
            if st.in_table_scope("caption") {
                st.generate_implied_end_tags(None);
                st.pop_until_html("caption");
                st.clear_formatting_to_marker();
                st.mode = Mode::InTable;
                StepAction::Reprocess(Mode::InTable)
            } else {
                StepAction::Done
            }
        }
        Token::EndTag(t)
            if matches!(
                t.name.as_str(),
                "body" | "col" | "colgroup" | "html" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr"
            ) =>
        {
            StepAction::Done
        }
        _ => StepAction::ProcessAsInBody,
    }
}

fn step_in_colgroup(st: &mut TreeState, token: &Token) -> StepAction {
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
        Token::StartTag(t) if t.name == "col" => {
            st.insert_void_html(t);
            StepAction::Done
        }
        Token::EndTag(t) if t.name == "colgroup" => {
            if st.current_is_html("colgroup") {
                st.open_elements.pop();
                st.mode = Mode::InTable;
            }
            StepAction::Done
        }
        Token::EndTag(t) if t.name == "col" => StepAction::Done,
        Token::StartTag(t) if t.name == "template" => StepAction::ProcessAsInHead,
        Token::EndTag(t) if t.name == "template" => StepAction::ProcessAsInHead,
        Token::Eof => StepAction::ProcessAsInBody,
        _ => {
            if !st.current_is_html("colgroup") {
                return StepAction::Done;
            }
            st.open_elements.pop();
            st.mode = Mode::InTable;
            StepAction::Reprocess(Mode::InTable)
        }
    }
}

fn step_in_table_body(st: &mut TreeState, token: &Token) -> StepAction {
    match token {
        Token::StartTag(t) if t.name == "tr" => {
            clear_stack_to_table_body_context(st);
            st.insert_html_element(t);
            st.mode = Mode::InRow;
            StepAction::Done
        }
        Token::StartTag(t) if matches!(t.name.as_str(), "th" | "td") => {
            clear_stack_to_table_body_context(st);
            let tr = TagToken {
                is_end: false,
                name: "tr".to_string(),
                self_closing: false,
                attributes: Vec::new(),
            };
            st.insert_html_element(&tr);
            st.mode = Mode::InRow;
            StepAction::Reprocess(Mode::InRow)
        }
        Token::EndTag(t) if matches!(t.name.as_str(), "tbody" | "tfoot" | "thead") => {
            if st.in_table_scope(&t.name) {
                clear_stack_to_table_body_context(st);
                st.open_elements.pop();
                st.mode = Mode::InTable;
            }
            StepAction::Done
        }
        Token::StartTag(t)
            if matches!(
                t.name.as_str(),
                "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead"
            ) =>
        {
            if st.in_table_scope("tbody")
                || st.in_table_scope("thead")
                || st.in_table_scope("tfoot")
            {
                clear_stack_to_table_body_context(st);
                st.open_elements.pop();
                st.mode = Mode::InTable;
                StepAction::Reprocess(Mode::InTable)
            } else {
                StepAction::Done
            }
        }
        Token::EndTag(t) if t.name == "table" => {
            if st.in_table_scope("tbody")
                || st.in_table_scope("thead")
                || st.in_table_scope("tfoot")
            {
                clear_stack_to_table_body_context(st);
                st.open_elements.pop();
                st.mode = Mode::InTable;
                StepAction::Reprocess(Mode::InTable)
            } else {
                StepAction::Done
            }
        }
        Token::EndTag(t)
            if matches!(
                t.name.as_str(),
                "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th" | "tr"
            ) =>
        {
            StepAction::Done
        }
        _ => {
            st.mode = Mode::InTable;
            let action = step_in_table(st, token, &mut TokenizerState::Data);
            if st.mode == Mode::InTable {
                st.mode = Mode::InTableBody;
            }
            action
        }
    }
}

fn step_in_row(st: &mut TreeState, token: &Token) -> StepAction {
    match token {
        Token::StartTag(t) if matches!(t.name.as_str(), "th" | "td") => {
            clear_stack_to_table_row_context(st);
            st.insert_html_element(t);
            st.mode = Mode::InCell;
            st.active_formatting.push(FormattingItem::Marker);
            StepAction::Done
        }
        Token::EndTag(t) if t.name == "tr" => {
            if st.in_table_scope("tr") {
                clear_stack_to_table_row_context(st);
                st.open_elements.pop();
                st.mode = Mode::InTableBody;
            }
            StepAction::Done
        }
        Token::StartTag(t)
            if matches!(
                t.name.as_str(),
                "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead" | "tr"
            ) =>
        {
            if st.in_table_scope("tr") {
                clear_stack_to_table_row_context(st);
                st.open_elements.pop();
                st.mode = Mode::InTableBody;
                StepAction::Reprocess(Mode::InTableBody)
            } else {
                StepAction::Done
            }
        }
        Token::EndTag(t) if t.name == "table" => {
            if st.in_table_scope("tr") {
                clear_stack_to_table_row_context(st);
                st.open_elements.pop();
                st.mode = Mode::InTableBody;
                StepAction::Reprocess(Mode::InTableBody)
            } else {
                StepAction::Done
            }
        }
        Token::EndTag(t) if matches!(t.name.as_str(), "tbody" | "tfoot" | "thead") => {
            if st.in_table_scope(&t.name) && st.in_table_scope("tr") {
                clear_stack_to_table_row_context(st);
                st.open_elements.pop();
                st.mode = Mode::InTableBody;
                StepAction::Reprocess(Mode::InTableBody)
            } else {
                StepAction::Done
            }
        }
        Token::EndTag(t)
            if matches!(
                t.name.as_str(),
                "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th"
            ) =>
        {
            StepAction::Done
        }
        _ => {
            let prev = st.mode;
            let action = step_in_table(st, token, &mut TokenizerState::Data);
            if st.mode == Mode::InTable {
                st.mode = prev;
            }
            action
        }
    }
}

fn close_cell(st: &mut TreeState) {
    st.generate_implied_end_tags(None);
    while let Some(id) = st.open_elements.pop() {
        if st.node_is_html(id, "td") || st.node_is_html(id, "th") {
            break;
        }
    }
    st.clear_formatting_to_marker();
    st.mode = Mode::InRow;
}

fn step_in_cell(st: &mut TreeState, token: &Token) -> StepAction {
    match token {
        Token::EndTag(t) if matches!(t.name.as_str(), "td" | "th") => {
            if st.in_table_scope(&t.name) {
                st.generate_implied_end_tags(None);
                st.pop_until_html(&t.name);
                st.clear_formatting_to_marker();
                st.mode = Mode::InRow;
            }
            StepAction::Done
        }
        Token::StartTag(t)
            if matches!(
                t.name.as_str(),
                "caption" | "col" | "colgroup" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr"
            ) =>
        {
            if st.in_table_scope("td") || st.in_table_scope("th") {
                close_cell(st);
                StepAction::Reprocess(Mode::InRow)
            } else {
                StepAction::Done
            }
        }
        Token::EndTag(t)
            if matches!(t.name.as_str(), "body" | "caption" | "col" | "colgroup" | "html") =>
        {
            StepAction::Done
        }
        Token::EndTag(t)
            if matches!(t.name.as_str(), "table" | "tbody" | "tfoot" | "thead" | "tr") =>
        {
            if st.in_table_scope(&t.name) {
                close_cell(st);
                StepAction::Reprocess(Mode::InRow)
            } else {
                StepAction::Done
            }
        }
        _ => StepAction::ProcessAsInBody,
    }
}
