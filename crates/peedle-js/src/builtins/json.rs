use crate::builtins::array::is_array_val_checked;
use crate::builtins::{set_global, set_method};
use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub fn parse_json_str(rt: &mut JsRuntime, text: &str) -> Result<JsValue, JsError> {
    let mut parser = JsonParser::new(text);
    let (val, _) = parser.parse_value(rt)?;
    parser.skip_ws();
    if parser.pos < parser.bytes.len() {
        return Err(JsError::syntax("Unexpected trailing data in JSON"));
    }
    Ok(val)
}

pub fn init(rt: &mut JsRuntime) {
    let json = rt.create_object();
    let tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(json).set_own_prop(
        PropertyKey::Symbol(tag),
        PropEntry::data(JsValue::String("JSON".into()), false, false, true),
    );

    set_method(
        rt,
        json,
        "parse",
        2,
        Rc::new(|rt, _, args| {
            let text = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            let reviver = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let mut parser = JsonParser::new(&text);
            let (val, record) = parser.parse_value(rt)?;
            parser.skip_ws();
            if parser.pos < parser.bytes.len() {
                return Err(JsError::syntax("Unexpected trailing data in JSON"));
            }
            if rt.is_callable(&reviver) {
                let root = rt.create_object();
                let _ = rt.define_property(
                    root,
                    PropertyKey::from(""),
                    PropertyDescriptor::data(val, true, true, true),
                );
                return revive_walk(rt, root, "", &reviver, Some(&record));
            }
            Ok(val)
        }),
    );

    set_method(
        rt,
        json,
        "rawJSON",
        1,
        Rc::new(|rt, _, args| {
            let text = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            if text.is_empty() {
                return Err(JsError::syntax("JSON.rawJSON cannot be empty"));
            }
            let bytes = text.as_bytes();
            let is_json_ws = |b: u8| matches!(b, b' ' | b'\t' | b'\n' | b'\r');
            if is_json_ws(bytes[0]) || is_json_ws(bytes[bytes.len() - 1]) {
                return Err(JsError::syntax("JSON.rawJSON cannot have leading or trailing whitespace"));
            }
            if matches!(bytes[0], b'{' | b'[') {
                return Err(JsError::syntax("JSON.rawJSON must be a JSON primitive"));
            }
            let mut parser = JsonParser::new(&text);
            let _ = parser.parse_value(rt)?;
            if parser.pos != parser.bytes.len() {
                return Err(JsError::syntax("Unexpected trailing data in JSON.rawJSON"));
            }
            let obj = rt.create_object_with_proto(None);
            rt.heap.get_obj_mut(obj).set_own_prop(
                PropertyKey::from("rawJSON"),
                PropEntry::data(JsValue::String(text), false, true, false),
            );
            rt.heap.get_obj_mut(obj).extensible = false;
            rt.heap.raw_json_objects.insert(obj);
            Ok(JsValue::Object(obj))
        }),
    );

    set_method(
        rt,
        json,
        "isRawJSON",
        1,
        Rc::new(|rt, _, args| {
            let b = matches!(
                args.first(),
                Some(JsValue::Object(oid)) if rt.heap.raw_json_objects.contains(oid)
            );
            Ok(JsValue::Boolean(b))
        }),
    );

    set_method(
        rt,
        json,
        "stringify",
        3,
        Rc::new(|rt, _, args| {
            let value = args.first().cloned().unwrap_or(JsValue::Undefined);
            let replacer = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let space_arg = args.get(2).cloned().unwrap_or(JsValue::Undefined);

            let replacer_fn = if rt.is_callable(&replacer) {
                Some(replacer.clone())
            } else {
                None
            };
            let prop_list = if replacer_fn.is_none() && is_array_val_checked(rt, &replacer)? {
                let mut list = Vec::new();
                if let JsValue::Object(rid) = replacer {
                    let len_v = rt.get_property(rid, "length")?;
                    let len = to_length(rt.to_number(&len_v)?);
                    for i in 0..len {
                        let item = rt.get_property(rid, &i.to_string())?;
                        let key = match &item {
                            JsValue::String(s) => Some(s.clone()),
                            JsValue::Number(n) => Some(number_to_string(*n)),
                            JsValue::Object(oid) => match &rt.heap.get_obj(*oid).kind {
                                ObjectKind::StringObj(_) | ObjectKind::NumberObj(_) => {
                                    Some(rt.to_string(&item)?)
                                }
                                _ => None,
                            },
                            _ => None,
                        };
                        if let Some(k) = key {
                            if !list.contains(&k) {
                                list.push(k);
                            }
                        }
                    }
                }
                Some(list)
            } else {
                None
            };

            let space_val = match &space_arg {
                JsValue::Object(oid) => match &rt.heap.get_obj(*oid).kind {
                    ObjectKind::NumberObj(_) => JsValue::Number(rt.to_number(&space_arg)?),
                    ObjectKind::StringObj(_) => JsValue::String(rt.to_string(&space_arg)?),
                    _ => space_arg,
                },
                _ => space_arg,
            };
            let gap = match space_val {
                JsValue::Number(n) => {
                    let count = to_integer_or_infinity(n).clamp(0.0, 10.0) as usize;
                    " ".repeat(count)
                }
                JsValue::String(s) => s.chars().take(10).collect(),
                _ => String::new(),
            };

            let wrapper = rt.create_object();
            let _ = rt.define_property(
                wrapper,
                PropertyKey::from(""),
                PropertyDescriptor::data(value, true, true, true),
            );
            let mut state = StringifyState {
                replacer_fn,
                prop_list,
                gap,
                indent: String::new(),
                stack: HashSet::new(),
            };
            match stringify_prop(rt, &mut state, "", wrapper)? {
                Some(s) => Ok(JsValue::String(s)),
                None => Ok(JsValue::Undefined),
            }
        }),
    );

    set_global(rt, "JSON", JsValue::Object(json));
}

enum ParseRecord {
    Primitive {
        value: JsValue,
        source: String,
    },
    Array {
        object: ObjectId,
        elements: Vec<ParseRecord>,
    },
    Object {
        object: ObjectId,
        entries: HashMap<String, ParseRecord>,
    },
}

struct StringifyState {
    replacer_fn: Option<JsValue>,
    prop_list: Option<Vec<String>>,
    gap: String,
    indent: String,
    stack: HashSet<ObjectId>,
}

fn stringify_prop(
    rt: &mut JsRuntime,
    state: &mut StringifyState,
    key: &str,
    holder: ObjectId,
) -> Result<Option<String>, JsError> {
    let mut val = rt.get_property(holder, key)?;
    match &val {
        JsValue::Object(oid) => {
            let to_json = rt.get_property(*oid, "toJSON")?;
            if rt.is_callable(&to_json) {
                val = rt.call_function(to_json, val, &[JsValue::String(key.to_string())])?;
            }
        }
        JsValue::BigInt(_) => {
            let bi_proto = rt.intrinsics.bigint_proto;
            let to_json = rt.get_property_key_receiver(
                bi_proto,
                &PropertyKey::from("toJSON"),
                val.clone(),
            )?;
            if rt.is_callable(&to_json) {
                val = rt.call_function(to_json, val, &[JsValue::String(key.to_string())])?;
            }
        }
        _ => {}
    }
    if let Some(ref rf) = state.replacer_fn {
        val = rt.call_function(
            rf.clone(),
            JsValue::Object(holder),
            &[JsValue::String(key.to_string()), val],
        )?;
    }
    if let JsValue::Object(oid) = val {
        if rt.heap.raw_json_objects.contains(&oid) {
            let raw = rt.get_property(oid, "rawJSON")?;
            if let JsValue::String(s) = raw {
                return Ok(Some(s));
            }
        }
        match &rt.heap.get_obj(oid).kind {
            ObjectKind::NumberObj(_) => val = JsValue::Number(rt.to_number(&val)?),
            ObjectKind::StringObj(_) => val = JsValue::String(rt.to_string(&val)?),
            ObjectKind::BooleanObj(b) => val = JsValue::Boolean(*b),
            ObjectKind::BigIntObj(b) => val = JsValue::BigInt(b.clone()),
            _ => {}
        }
    }
    match val {
        JsValue::Null => Ok(Some("null".into())),
        JsValue::Boolean(true) => Ok(Some("true".into())),
        JsValue::Boolean(false) => Ok(Some("false".into())),
        JsValue::String(s) => Ok(Some(quote_json_string(&s))),
        JsValue::Number(n) => {
            if n.is_finite() {
                Ok(Some(number_to_string(n)))
            } else {
                Ok(Some("null".into()))
            }
        }
        JsValue::BigInt(_) => Err(JsError::type_err("Do not know how to serialize a BigInt")),
        JsValue::Undefined | JsValue::Symbol(_) => Ok(None),
        JsValue::Object(oid) => {
            if rt.is_callable(&JsValue::Object(oid)) {
                return Ok(None);
            }
            if !state.stack.insert(oid) {
                return Err(JsError::type_err("Converting circular structure to JSON"));
            }
            let is_arr = match is_array_val_checked(rt, &JsValue::Object(oid)) {
                Ok(b) => b,
                Err(e) => {
                    state.stack.remove(&oid);
                    return Err(e);
                }
            };
            let res = if is_arr {
                stringify_array(rt, state, oid)
            } else {
                stringify_object(rt, state, oid)
            };
            state.stack.remove(&oid);
            res.map(Some)
        }
    }
}

fn stringify_array(
    rt: &mut JsRuntime,
    state: &mut StringifyState,
    oid: ObjectId,
) -> Result<String, JsError> {
    let stepback = state.indent.clone();
    state.indent.push_str(&state.gap);
    let len_v = rt.get_property(oid, "length")?;
    let len = to_length(rt.to_number(&len_v)?);
    let mut parts = Vec::with_capacity(len);
    for i in 0..len {
        let p = stringify_prop(rt, state, &i.to_string(), oid)?
            .unwrap_or_else(|| "null".to_string());
        parts.push(p);
    }
    let out = if parts.is_empty() {
        "[]".to_string()
    } else if state.gap.is_empty() {
        format!("[{}]", parts.join(","))
    } else {
        let sep = format!(",\n{}", state.indent);
        format!("[\n{}{}\n{}]", state.indent, parts.join(&sep), stepback)
    };
    state.indent = stepback;
    Ok(out)
}

fn stringify_object(
    rt: &mut JsRuntime,
    state: &mut StringifyState,
    oid: ObjectId,
) -> Result<String, JsError> {
    let stepback = state.indent.clone();
    state.indent.push_str(&state.gap);
    let keys: Vec<String> = if let Some(ref pl) = state.prop_list {
        pl.clone()
    } else {
        let own = rt.own_keys(oid)?;
        let mut ks = Vec::new();
        for k in own {
            if let PropertyKey::String(s) = k {
                if let Some(d) =
                    rt.get_own_property_descriptor(oid, &PropertyKey::String(s.clone()))?
                {
                    if d.enumerable.unwrap_or(false) {
                        ks.push(s);
                    }
                }
            }
        }
        ks
    };
    let mut parts = Vec::new();
    for k in keys {
        if let Some(v_str) = stringify_prop(rt, state, &k, oid)? {
            let mut member = quote_json_string(&k);
            member.push(':');
            if !state.gap.is_empty() {
                member.push(' ');
            }
            member.push_str(&v_str);
            parts.push(member);
        }
    }
    let out = if parts.is_empty() {
        "{}".to_string()
    } else if state.gap.is_empty() {
        format!("{{{}}}", parts.join(","))
    } else {
        let sep = format!(",\n{}", state.indent);
        format!("{{\n{}{}\n{}}}", state.indent, parts.join(&sep), stepback)
    };
    state.indent = stepback;
    Ok(out)
}

fn quote_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn revive_walk(
    rt: &mut JsRuntime,
    holder: ObjectId,
    name: &str,
    reviver: &JsValue,
    snapshot: Option<&ParseRecord>,
) -> Result<JsValue, JsError> {
    let val = rt.get_property(holder, name)?;
    let ctx = rt.create_object();
    if let JsValue::Object(oid) = val {
        let is_arr = is_array_val_checked(rt, &val)?;
        if is_arr {
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let elem_snaps = match snapshot {
                Some(ParseRecord::Array { object, elements }) if *object == oid => Some(elements),
                _ => None,
            };
            for i in 0..len {
                let k = i.to_string();
                let child_snap = elem_snaps.and_then(|els| els.get(i));
                let new_el = revive_walk(rt, oid, &k, reviver, child_snap)?;
                if new_el.is_undefined() {
                    rt.delete_property(oid, &PropertyKey::String(k))?;
                } else {
                    rt.define_property(
                        oid,
                        PropertyKey::String(k),
                        PropertyDescriptor::data(new_el, true, true, true),
                    )?;
                }
            }
        } else {
            let own = rt.own_keys(oid)?;
            let mut keys = Vec::new();
            for k in own {
                if let PropertyKey::String(s) = k {
                    if let Some(d) =
                        rt.get_own_property_descriptor(oid, &PropertyKey::String(s.clone()))?
                    {
                        if d.enumerable.unwrap_or(false) {
                            keys.push(s);
                        }
                    }
                }
            }
            let prop_snaps = match snapshot {
                Some(ParseRecord::Object { object, entries }) if *object == oid => Some(entries),
                _ => None,
            };
            for s in keys {
                let child_snap = prop_snaps.and_then(|m| m.get(&s));
                let new_el = revive_walk(rt, oid, &s, reviver, child_snap)?;
                if new_el.is_undefined() {
                    rt.delete_property(oid, &PropertyKey::String(s))?;
                } else {
                    rt.define_property(
                        oid,
                        PropertyKey::String(s),
                        PropertyDescriptor::data(new_el, true, true, true),
                    )?;
                }
            }
        }
    } else if let Some(ParseRecord::Primitive { value, source }) = snapshot {
        if val.same_value(value) {
            let _ = rt.define_property(
                ctx,
                PropertyKey::from("source"),
                PropertyDescriptor::data(JsValue::String(source.clone()), true, true, true),
            );
        }
    }
    rt.call_function(
        reviver.clone(),
        JsValue::Object(holder),
        &[JsValue::String(name.to_string()), val, JsValue::Object(ctx)],
    )
}

struct JsonParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(src: &'a str) -> Self {
        JsonParser {
            src,
            bytes: src.as_bytes(),
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while let Some(&b) = self.bytes.get(self.pos) {
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse_value(&mut self, rt: &mut JsRuntime) -> Result<(JsValue, ParseRecord), JsError> {
        self.skip_ws();
        let start = self.pos;
        let Some(&b) = self.bytes.get(self.pos) else {
            return Err(JsError::syntax("Unexpected end of JSON input"));
        };
        match b {
            b'"' => {
                let s = self.parse_string()?;
                let raw = self.src[start..self.pos].to_string();
                let v = JsValue::String(s);
                Ok((v.clone(), ParseRecord::Primitive { value: v, source: raw }))
            }
            b'{' => self.parse_object(rt),
            b'[' => self.parse_array(rt),
            b't' => {
                if self.bytes.get(self.pos..self.pos + 4) == Some(b"true") {
                    self.pos += 4;
                    let v = JsValue::Boolean(true);
                    Ok((v.clone(), ParseRecord::Primitive { value: v, source: "true".into() }))
                } else {
                    Err(JsError::syntax("Invalid JSON token"))
                }
            }
            b'f' => {
                if self.bytes.get(self.pos..self.pos + 5) == Some(b"false") {
                    self.pos += 5;
                    let v = JsValue::Boolean(false);
                    Ok((v.clone(), ParseRecord::Primitive { value: v, source: "false".into() }))
                } else {
                    Err(JsError::syntax("Invalid JSON token"))
                }
            }
            b'n' => {
                if self.bytes.get(self.pos..self.pos + 4) == Some(b"null") {
                    self.pos += 4;
                    let v = JsValue::Null;
                    Ok((v.clone(), ParseRecord::Primitive { value: v, source: "null".into() }))
                } else {
                    Err(JsError::syntax("Invalid JSON token"))
                }
            }
            b'-' | b'0'..=b'9' => {
                let n = self.parse_number()?;
                let raw = self.src[start..self.pos].to_string();
                let v = JsValue::Number(n);
                Ok((v.clone(), ParseRecord::Primitive { value: v, source: raw }))
            }
            _ => Err(JsError::syntax("Unexpected token in JSON")),
        }
    }

    fn parse_object(&mut self, rt: &mut JsRuntime) -> Result<(JsValue, ParseRecord), JsError> {
        self.pos += 1;
        let obj = rt.create_object();
        let mut entries = HashMap::new();
        self.skip_ws();
        if self.bytes.get(self.pos) == Some(&b'}') {
            self.pos += 1;
            return Ok((
                JsValue::Object(obj),
                ParseRecord::Object { object: obj, entries },
            ));
        }
        loop {
            self.skip_ws();
            if self.bytes.get(self.pos) != Some(&b'"') {
                return Err(JsError::syntax("Expected string key in JSON object"));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.bytes.get(self.pos) != Some(&b':') {
                return Err(JsError::syntax("Expected ':' in JSON object"));
            }
            self.pos += 1;
            let (val, rec) = self.parse_value(rt)?;
            rt.heap.get_obj_mut(obj).set_own_prop(
                PropertyKey::String(key.clone()),
                PropEntry::data(val, true, true, true),
            );
            entries.insert(key, rec);
            self.skip_ws();
            match self.bytes.get(self.pos) {
                Some(&b',') => self.pos += 1,
                Some(&b'}') => {
                    self.pos += 1;
                    return Ok((
                        JsValue::Object(obj),
                        ParseRecord::Object { object: obj, entries },
                    ));
                }
                _ => return Err(JsError::syntax("Expected ',' or '}' in JSON object")),
            }
        }
    }

    fn parse_array(&mut self, rt: &mut JsRuntime) -> Result<(JsValue, ParseRecord), JsError> {
        self.pos += 1;
        let mut items = Vec::new();
        let mut records = Vec::new();
        self.skip_ws();
        if self.bytes.get(self.pos) == Some(&b']') {
            self.pos += 1;
            let arr = rt.create_array(items);
            return Ok((
                JsValue::Object(arr),
                ParseRecord::Array { object: arr, elements: records },
            ));
        }
        loop {
            let (v, r) = self.parse_value(rt)?;
            items.push(v);
            records.push(r);
            self.skip_ws();
            match self.bytes.get(self.pos) {
                Some(&b',') => self.pos += 1,
                Some(&b']') => {
                    self.pos += 1;
                    let arr = rt.create_array(items);
                    return Ok((
                        JsValue::Object(arr),
                        ParseRecord::Array { object: arr, elements: records },
                    ));
                }
                _ => return Err(JsError::syntax("Expected ',' or ']' in JSON array")),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, JsError> {
        self.pos += 1;
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let c = self.src[self.pos..].chars().next().unwrap();
            if c == '"' {
                self.pos += 1;
                return Ok(out);
            }
            if (c as u32) < 0x20 {
                return Err(JsError::syntax("Unescaped control character in JSON string"));
            }
            if c == '\\' {
                self.pos += 1;
                let Some(&esc) = self.bytes.get(self.pos) else {
                    return Err(JsError::syntax("Unterminated escape in JSON string"));
                };
                self.pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000C}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let code = self.read_hex4()?;
                        if (0xD800..=0xDBFF).contains(&code)
                            && self.bytes.get(self.pos..self.pos + 2) == Some(b"\\u")
                        {
                            let saved = self.pos;
                            self.pos += 2;
                            if let Ok(low) = self.read_hex4() {
                                if (0xDC00..=0xDFFF).contains(&low) {
                                    let cp =
                                        0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                    if let Some(ch) = char::from_u32(cp) {
                                        out.push(ch);
                                        continue;
                                    }
                                }
                            }
                            self.pos = saved;
                        }
                        out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                    }
                    _ => return Err(JsError::syntax("Invalid escape in JSON string")),
                }
            } else {
                self.pos += c.len_utf8();
                out.push(c);
            }
        }
        Err(JsError::syntax("Unterminated JSON string"))
    }

    fn read_hex4(&mut self) -> Result<u32, JsError> {
        if self.pos + 4 > self.bytes.len() {
            return Err(JsError::syntax("Invalid \\u escape in JSON"));
        }
        let s = &self.src[self.pos..self.pos + 4];
        let val = u32::from_str_radix(s, 16)
            .map_err(|_| JsError::syntax("Invalid hex digits in JSON \\u escape"))?;
        self.pos += 4;
        Ok(val)
    }

    fn parse_number(&mut self) -> Result<f64, JsError> {
        let start = self.pos;
        if self.bytes.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        match self.bytes.get(self.pos) {
            Some(&b'0') => {
                self.pos += 1;
                if self
                    .bytes
                    .get(self.pos)
                    .is_some_and(|b| b.is_ascii_digit())
                {
                    return Err(JsError::syntax("Leading zero in JSON number"));
                }
            }
            Some(b'1'..=b'9') => {
                while self
                    .bytes
                    .get(self.pos)
                    .is_some_and(|b| b.is_ascii_digit())
                {
                    self.pos += 1;
                }
            }
            _ => return Err(JsError::syntax("Invalid JSON number")),
        }
        if self.bytes.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            if !self
                .bytes
                .get(self.pos)
                .is_some_and(|b| b.is_ascii_digit())
            {
                return Err(JsError::syntax("Invalid decimal in JSON number"));
            }
            while self
                .bytes
                .get(self.pos)
                .is_some_and(|b| b.is_ascii_digit())
            {
                self.pos += 1;
            }
        }
        if matches!(self.bytes.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.bytes.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !self
                .bytes
                .get(self.pos)
                .is_some_and(|b| b.is_ascii_digit())
            {
                return Err(JsError::syntax("Invalid exponent in JSON number"));
            }
            while self
                .bytes
                .get(self.pos)
                .is_some_and(|b| b.is_ascii_digit())
            {
                self.pos += 1;
            }
        }
        self.src[start..self.pos]
            .parse::<f64>()
            .map_err(|_| JsError::syntax("Invalid JSON number"))
    }
}
