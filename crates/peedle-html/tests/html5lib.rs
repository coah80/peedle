use std::path::Path;

use peedle_html::{
    parse_fragment, parse_html, parse_html_with_options, Document, Namespace, NodeData, NodeId,
    Token, Tokenizer, TokenizerState,
};
use serde_json::Value;

#[test]
fn basic_html5_parsing() {
    let doc = parse_html("<!DOCTYPE html><html><body><p>Hello <b>world</p></b></body></html>");
    let body = doc.body().unwrap();
    let html = doc.inner_html(body);
    assert!(html.contains("Hello "));
    assert!(html.contains("<b>world</b>"));
}

#[test]
fn html5lib_tokenizer_suite() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/html5lib-tests/tokenizer");
    if !dir.exists() {
        return;
    }

    let files = [
        "test1.test",
        "test2.test",
        "test3.test",
        "test4.test",
        "entities.test",
        "namedEntities.test",
        "numericEntities.test",
        "contentModelFlags.test",
        "escapeFlag.test",
        "domjs.test",
    ];

    let mut total = 0usize;
    let mut passed = 0usize;
    let mut sample_failures = Vec::new();

    for fname in files {
        let path = dir.join(fname);
        if !path.exists() {
            continue;
        }
        let raw = std::fs::read_to_string(&path).unwrap();
        let json: Value = serde_json::from_str(&raw).unwrap();
        let Some(tests) = json.get("tests").and_then(|v| v.as_array()) else {
            continue;
        };
        for tc in tests {
            let double_escaped = tc
                .get("doubleEscaped")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if double_escaped {
                continue;
            }
            let input = tc.get("input").and_then(|v| v.as_str()).unwrap_or("");
            let expected = tc.get("output").and_then(|v| v.as_array()).unwrap();
            let last_start = tc.get("lastStartTag").and_then(|v| v.as_str());
            let initial_states: Vec<&str> = tc
                .get("initialStates")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|s| s.as_str()).collect())
                .unwrap_or_else(|| vec!["Data state"]);

            for state_name in initial_states {
                let Some(init_state) = map_state_name(state_name) else {
                    continue;
                };
                total += 1;
                let mut tok = Tokenizer::new(input);
                tok.state = init_state;
                if let Some(ls) = last_start {
                    tok.set_last_start_tag_name(ls);
                }
                let got = collect_html5lib_tokens(&mut tok);
                let exp_norm = normalize_expected_tokens(expected);
                if got == exp_norm {
                    passed += 1;
                } else if sample_failures.len() < 12 {
                    sample_failures.push(format!(
                        "{}: {:?} [{}] got={:?} exp={:?}",
                        fname, input, state_name, got, exp_norm
                    ));
                }
            }
        }
    }

    let pct = (passed as f64 / total.max(1) as f64) * 100.0;
    eprintln!("html5lib tokenizer: {}/{} ({:.1}%)", passed, total, pct);
    for f in &sample_failures {
        eprintln!("  {}", f);
    }
    assert!(pct >= 85.0, "html5lib tokenizer pass rate too low: {:.1}%", pct);
}

#[test]
fn html5lib_tree_construction_suite() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/html5lib-tests/tree-construction");
    if !dir.exists() {
        return;
    }

    let mut total = 0usize;
    let mut passed = 0usize;
    let mut sample_failures = Vec::new();

    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("dat"))
        .collect();
    entries.sort();

    for path in entries {
        let fname = path.file_name().unwrap().to_string_lossy().to_string();
        let raw = std::fs::read_to_string(&path).unwrap();
        for tc in parse_dat_tests(&raw) {
            if tc.script_off {
                continue;
            }
            total += 1;
            let doc = if let Some((ctx_ns, ctx_tag)) = &tc.fragment {
                parse_fragment(&tc.data, ctx_tag, *ctx_ns, !tc.script_off)
            } else {
                parse_html_with_options(&tc.data, !tc.script_off)
            };
            let got = if tc.fragment.is_some() {
                if let Some(html_el) = doc.document_element() {
                    serialize_tree_children(&doc, html_el, 0)
                } else {
                    String::new()
                }
            } else {
                serialize_tree_children(&doc, doc.root, 0)
            };
            let got_trim = got.trim_end_matches('\n');
            let exp_trim = tc.document.trim_end_matches('\n');
            if got_trim == exp_trim {
                passed += 1;
            } else if sample_failures.len() < 15 {
                sample_failures.push(format!(
                    "{}: data={:?}\n  GOT:\n{}\n  EXP:\n{}",
                    fname, tc.data, got_trim, exp_trim
                ));
            }
        }
    }

    let pct = (passed as f64 / total.max(1) as f64) * 100.0;
    eprintln!("html5lib tree-construction: {}/{} ({:.1}%)", passed, total, pct);
    for f in &sample_failures {
        eprintln!("  {}", f);
    }
    assert_eq!(
        passed, total,
        "html5lib tree-construction pass rate regressed: {}/{}",
        passed, total
    );
}

fn map_state_name(name: &str) -> Option<TokenizerState> {
    match name {
        "Data state" => Some(TokenizerState::Data),
        "RCDATA state" => Some(TokenizerState::Rcdata),
        "RAWTEXT state" => Some(TokenizerState::Rawtext),
        "Script data state" => Some(TokenizerState::ScriptData),
        "PLAINTEXT state" => Some(TokenizerState::Plaintext),
        "CDATA section state" => Some(TokenizerState::CdataSection),
        _ => None,
    }
}

fn collect_html5lib_tokens(tok: &mut Tokenizer) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut char_buf = String::new();

    let flush_chars = |buf: &mut String, out: &mut Vec<Value>| {
        if !buf.is_empty() {
            let s = std::mem::take(buf);
            out.push(serde_json::json!(["Character", s]));
        }
    };

    loop {
        match tok.next_token() {
            Token::Eof => {
                flush_chars(&mut char_buf, &mut out);
                break;
            }
            Token::Character(c) => char_buf.push(c),
            Token::Comment(s) => {
                flush_chars(&mut char_buf, &mut out);
                out.push(serde_json::json!(["Comment", s]));
            }
            Token::Doctype(dt) => {
                flush_chars(&mut char_buf, &mut out);
                out.push(serde_json::json!([
                    "DOCTYPE",
                    dt.name,
                    dt.public_id,
                    dt.system_id,
                    !dt.force_quirks
                ]));
            }
            Token::StartTag(t) => {
                flush_chars(&mut char_buf, &mut out);
                let mut map = serde_json::Map::new();
                for a in t.attributes {
                    map.insert(a.name, Value::String(a.value));
                }
                if t.self_closing {
                    out.push(serde_json::json!(["StartTag", t.name, map, true]));
                } else {
                    out.push(serde_json::json!(["StartTag", t.name, map]));
                }
            }
            Token::EndTag(t) => {
                flush_chars(&mut char_buf, &mut out);
                out.push(serde_json::json!(["EndTag", t.name]));
            }
        }
    }
    out
}

fn normalize_expected_tokens(expected: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for item in expected {
        if let Some(arr) = item.as_array() {
            if arr.first().and_then(|v| v.as_str()) == Some("Character") {
                if let Some(last) = out.last_mut().and_then(|v| v.as_array_mut()) {
                    if last.first().and_then(|v| v.as_str()) == Some("Character") {
                        let s1 = last[1].as_str().unwrap_or("").to_string();
                        let s2 = arr[1].as_str().unwrap_or("");
                        last[1] = Value::String(format!("{}{}", s1, s2));
                        continue;
                    }
                }
            }
        }
        out.push(item.clone());
    }
    out
}

struct DatTest {
    data: String,
    document: String,
    fragment: Option<(Namespace, String)>,
    script_off: bool,
}

fn parse_dat_tests(raw: &str) -> Vec<DatTest> {
    let mut tests = Vec::new();
    let lines: Vec<&str> = raw.split('\n').collect();
    let mut i = 0;
    while i < lines.len() {
        if lines[i] != "#data" {
            i += 1;
            continue;
        }
        i += 1;
        let mut data_lines = Vec::new();
        while i < lines.len() && !lines[i].starts_with('#') {
            data_lines.push(lines[i]);
            i += 1;
        }
        let data = data_lines.join("\n");
        let mut fragment = None;
        let mut script_off = false;
        let mut doc_lines = Vec::new();

        while i < lines.len() && lines[i] != "#data" {
            let hdr = lines[i];
            i += 1;
            match hdr {
                "#document-fragment" => {
                    if i < lines.len() {
                        let frag_str = lines[i].trim();
                        if let Some((ns_s, tag)) = frag_str.split_once(' ') {
                            let ns = match ns_s {
                                "svg" => Namespace::Svg,
                                "math" => Namespace::MathMl,
                                _ => Namespace::Html,
                            };
                            fragment = Some((ns, tag.to_string()));
                        } else if !frag_str.is_empty() {
                            fragment = Some((Namespace::Html, frag_str.to_string()));
                        }
                        i += 1;
                    }
                }
                "#script-off" => {
                    script_off = true;
                }
                "#document" => {
                    while i < lines.len() && lines[i] != "#data" && !lines[i].starts_with("#new-errors") {
                        if lines[i].starts_with("| ") || !doc_lines.is_empty() {
                            doc_lines.push(lines[i]);
                        }
                        i += 1;
                    }
                    while doc_lines.last() == Some(&"") {
                        doc_lines.pop();
                    }
                }
                _ => {
                    while i < lines.len() && !lines[i].starts_with('#') {
                        i += 1;
                    }
                }
            }
        }
        tests.push(DatTest {
            data,
            document: doc_lines.join("\n"),
            fragment,
            script_off,
        });
    }
    tests
}

fn serialize_tree_children(doc: &Document, parent: NodeId, depth: usize) -> String {
    let mut out = String::new();
    for child in doc.children(parent) {
        serialize_tree_node(doc, child, depth, &mut out);
    }
    out
}

fn serialize_tree_node(doc: &Document, id: NodeId, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    match &doc.node(id).data {
        NodeData::Doctype {
            name,
            public_id,
            system_id,
        } => {
            if public_id.is_empty() && system_id.is_empty() {
                out.push_str(&format!("| {}<!DOCTYPE {}>\n", indent, name));
            } else {
                out.push_str(&format!(
                    "| {}<!DOCTYPE {} \"{}\" \"{}\">\n",
                    indent, name, public_id, system_id
                ));
            }
        }
        NodeData::Comment(s) => {
            out.push_str(&format!("| {}<!-- {} -->\n", indent, s));
        }
        NodeData::Text(s) => {
            out.push_str(&format!("| {}\"{}\"\n", indent, s));
        }
        NodeData::Element(e) => {
            let prefix = match e.namespace {
                Namespace::Html => "",
                Namespace::Svg => "svg ",
                Namespace::MathMl => "math ",
            };
            out.push_str(&format!("| {}<{}{}>\n", indent, prefix, e.tag_name));
            let mut attrs: Vec<(String, String)> = e
                .attributes
                .iter()
                .map(|a| {
                    let aname = if let Some((p, local)) = a.name.split_once(':') {
                        if a.namespace.is_some() {
                            format!("{} {}", p, local)
                        } else {
                            a.name.clone()
                        }
                    } else {
                        a.name.clone()
                    };
                    (aname, a.value.clone())
                })
                .collect();
            attrs.sort_by(|a, b| a.0.cmp(&b.0));
            let attr_indent = "  ".repeat(depth + 1);
            for (aname, value) in attrs {
                out.push_str(&format!("| {}{}=\"{}\"\n", attr_indent, aname, value));
            }
            if let Some(tc) = e.template_contents {
                out.push_str(&format!("| {}content\n", attr_indent));
                for c in doc.children(tc) {
                    serialize_tree_node(doc, c, depth + 2, out);
                }
            } else {
                for c in doc.children(id) {
                    serialize_tree_node(doc, c, depth + 1, out);
                }
            }
        }
        _ => {}
    }
}
