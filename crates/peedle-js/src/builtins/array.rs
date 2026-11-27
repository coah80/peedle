use crate::builtins::{set_global, set_method, set_symbol_method};
use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::rc::Rc;

pub fn init(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.array_proto;

    set_method(
        rt,
        proto,
        "push",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let mut len = to_length(rt.to_number(&len_v)?);
            for a in args {
                rt.set_property_key_strict(oid, PropertyKey::String(len.to_string()), a.clone(), true)?;
                len += 1;
            }
            rt.set_property_key_strict(oid, PropertyKey::from("length"), JsValue::Number(len as f64), true)?;
            Ok(JsValue::Number(len as f64))
        }),
    );

    set_method(
        rt,
        proto,
        "pop",
        0,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            if len == 0 {
                rt.set_property_key_strict(oid, PropertyKey::from("length"), JsValue::Number(0.0), true)?;
                return Ok(JsValue::Undefined);
            }
            let new_len = len - 1;
            let key = PropertyKey::String(new_len.to_string());
            let val = rt.get_property_key(oid, &key)?;
            rt.delete_property(oid, &key)?;
            rt.set_property_key_strict(oid, PropertyKey::from("length"), JsValue::Number(new_len as f64), true)?;
            Ok(val)
        }),
    );

    set_method(
        rt,
        proto,
        "shift",
        0,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            if len == 0 {
                rt.set_property_key_strict(oid, PropertyKey::from("length"), JsValue::Number(0.0), true)?;
                return Ok(JsValue::Undefined);
            }
            let first = rt.get_property(oid, "0")?;
            for k in 1..len {
                let from = PropertyKey::String(k.to_string());
                let to = PropertyKey::String((k - 1).to_string());
                if rt.has_property(oid, &from)? {
                    let v = rt.get_property_key(oid, &from)?;
                    rt.set_property_key_strict(oid, to, v, true)?;
                } else {
                    rt.delete_property(oid, &to)?;
                }
            }
            rt.delete_property(oid, &PropertyKey::String((len - 1).to_string()))?;
            rt.set_property_key_strict(oid, PropertyKey::from("length"), JsValue::Number((len - 1) as f64), true)?;
            Ok(first)
        }),
    );

    set_method(
        rt,
        proto,
        "unshift",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let ac = args.len();
            if ac > 0 {
                for k in (0..len).rev() {
                    let from = PropertyKey::String(k.to_string());
                    let to = PropertyKey::String((k + ac).to_string());
                    if rt.has_property(oid, &from)? {
                        let v = rt.get_property_key(oid, &from)?;
                        rt.set_property_key_strict(oid, to, v, true)?;
                    } else {
                        rt.delete_property(oid, &to)?;
                    }
                }
                for (i, a) in args.iter().enumerate() {
                    rt.set_property_key_strict(oid, PropertyKey::String(i.to_string()), a.clone(), true)?;
                }
            }
            let new_len = len + ac;
            rt.set_property_key_strict(oid, PropertyKey::from("length"), JsValue::Number(new_len as f64), true)?;
            Ok(JsValue::Number(new_len as f64))
        }),
    );

    set_method(
        rt,
        proto,
        "slice",
        2,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let start = resolve_index(rt, args.first(), len, 0)?;
            let end = resolve_index(rt, args.get(1), len, len)?;
            let out = rt.create_array(Vec::new());
            let mut n = 0usize;
            for k in start..end {
                let pk = PropertyKey::String(k.to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    rt.define_property(
                        out,
                        PropertyKey::String(n.to_string()),
                        PropertyDescriptor::data(v, true, true, true),
                    )?;
                }
                n += 1;
            }
            let _ = rt.set_property(out, "length", JsValue::Number(n as f64));
            Ok(JsValue::Object(out))
        }),
    );

    set_method(
        rt,
        proto,
        "splice",
        2,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let start = resolve_index(rt, args.first(), len, 0)?;
            let del_count = if args.is_empty() {
                0
            } else if args.len() == 1 {
                len - start
            } else {
                let dc = to_integer_or_infinity(rt.to_number(&args[1])?);
                dc.clamp(0.0, (len - start) as f64) as usize
            };
            let deleted = rt.create_array(Vec::new());
            for k in 0..del_count {
                let pk = PropertyKey::String((start + k).to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    let _ = rt.set_property(deleted, &k.to_string(), v);
                }
            }
            let _ = rt.set_property(deleted, "length", JsValue::Number(del_count as f64));
            let insert = if args.len() > 2 { &args[2..] } else { &[] };
            let ic = insert.len();
            if ic < del_count {
                for k in start..(len - del_count) {
                    let from = PropertyKey::String((k + del_count).to_string());
                    let to = PropertyKey::String((k + ic).to_string());
                    if rt.has_property(oid, &from)? {
                        let v = rt.get_property_key(oid, &from)?;
                        rt.set_property_key_strict(oid, to, v, true)?;
                    } else {
                        rt.delete_property(oid, &to)?;
                    }
                }
                for k in (len - del_count + ic)..len {
                    rt.delete_property(oid, &PropertyKey::String(k.to_string()))?;
                }
            } else if ic > del_count {
                for k in (start..(len - del_count)).rev() {
                    let from = PropertyKey::String((k + del_count).to_string());
                    let to = PropertyKey::String((k + ic).to_string());
                    if rt.has_property(oid, &from)? {
                        let v = rt.get_property_key(oid, &from)?;
                        rt.set_property_key_strict(oid, to, v, true)?;
                    } else {
                        rt.delete_property(oid, &to)?;
                    }
                }
            }
            for (i, item) in insert.iter().enumerate() {
                rt.set_property_key_strict(
                    oid,
                    PropertyKey::String((start + i).to_string()),
                    item.clone(),
                    true,
                )?;
            }
            rt.set_property_key_strict(
                oid,
                PropertyKey::from("length"),
                JsValue::Number((len - del_count + ic) as f64),
                true,
            )?;
            Ok(JsValue::Object(deleted))
        }),
    );

    set_method(
        rt,
        proto,
        "concat",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let mut items = vec![JsValue::Object(oid)];
            items.extend_from_slice(args);
            let out = rt.create_array(Vec::new());
            let mut n = 0usize;
            for item in items {
                if is_concat_spreadable(rt, &item)? {
                    let item_id = rt.to_object(&item)?;
                    let len_v = rt.get_property(item_id, "length")?;
                    let len = to_length(rt.to_number(&len_v)?);
                    for k in 0..len {
                        let pk = PropertyKey::String(k.to_string());
                        if rt.has_property(item_id, &pk)? {
                            let v = rt.get_property_key(item_id, &pk)?;
                            let _ = rt.set_property(out, &n.to_string(), v);
                        }
                        n += 1;
                    }
                } else {
                    let _ = rt.set_property(out, &n.to_string(), item);
                    n += 1;
                }
            }
            let _ = rt.set_property(out, "length", JsValue::Number(n as f64));
            Ok(JsValue::Object(out))
        }),
    );

    set_method(
        rt,
        proto,
        "join",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let sep = match args.first() {
                None | Some(JsValue::Undefined) => ",".to_string(),
                Some(v) => rt.to_string(v)?,
            };
            let mut out = String::new();
            for k in 0..len {
                if k > 0 {
                    out.push_str(&sep);
                }
                let el = rt.get_property(oid, &k.to_string())?;
                if !el.is_nullish() {
                    out.push_str(&rt.to_string(&el)?);
                }
            }
            Ok(JsValue::String(out))
        }),
    );

    set_method(
        rt,
        proto,
        "toString",
        0,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            let join = rt.get_property(oid, "join")?;
            if rt.is_callable(&join) {
                rt.call_function(join, JsValue::Object(oid), &[])
            } else {
                Ok(JsValue::String(crate::builtins::object::object_to_string(
                    rt,
                    &JsValue::Object(oid),
                )?))
            }
        }),
    );

    set_method(
        rt,
        proto,
        "toLocaleString",
        0,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let mut parts = Vec::with_capacity(len);
            for k in 0..len {
                let el = rt.get_property(oid, &k.to_string())?;
                if el.is_nullish() {
                    parts.push(String::new());
                } else {
                    let e_oid = rt.to_object(&el)?;
                    let fn_v = rt.get_property(e_oid, "toLocaleString")?;
                    let s = rt.call_function(fn_v, el, &[])?;
                    parts.push(rt.to_string(&s)?);
                }
            }
            Ok(JsValue::String(parts.join(",")))
        }),
    );

    set_method(
        rt,
        proto,
        "reverse",
        0,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let mid = len / 2;
            for lo in 0..mid {
                let hi = len - lo - 1;
                let lk = PropertyKey::String(lo.to_string());
                let hk = PropertyKey::String(hi.to_string());
                let l_exists = rt.has_property(oid, &lk)?;
                let h_exists = rt.has_property(oid, &hk)?;
                let lv = if l_exists {
                    Some(rt.get_property_key(oid, &lk)?)
                } else {
                    None
                };
                let hv = if h_exists {
                    Some(rt.get_property_key(oid, &hk)?)
                } else {
                    None
                };
                match (lv, hv) {
                    (Some(l), Some(h)) => {
                        rt.set_property_key_strict(oid, lk, h, true)?;
                        rt.set_property_key_strict(oid, hk, l, true)?;
                    }
                    (None, Some(h)) => {
                        rt.set_property_key_strict(oid, lk, h, true)?;
                        rt.delete_property(oid, &hk)?;
                    }
                    (Some(l), None) => {
                        rt.delete_property(oid, &lk)?;
                        rt.set_property_key_strict(oid, hk, l, true)?;
                    }
                    (None, None) => {}
                }
            }
            Ok(JsValue::Object(oid))
        }),
    );

    set_method(
        rt,
        proto,
        "sort",
        1,
        Rc::new(|rt, this, args| {
            let cmp = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !cmp.is_undefined() && !rt.is_callable(&cmp) {
                return Err(JsError::type_err("The comparison function must be a function"));
            }
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let mut items = Vec::new();
            let mut undef_count = 0usize;
            for k in 0..len {
                let pk = PropertyKey::String(k.to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    if v.is_undefined() {
                        undef_count += 1;
                    } else {
                        items.push(v);
                    }
                }
            }
            stable_merge_sort(rt, &mut items, &cmp)?;
            let mut idx = 0usize;
            for v in items {
                rt.set_property_key_strict(oid, PropertyKey::String(idx.to_string()), v, true)?;
                idx += 1;
            }
            for _ in 0..undef_count {
                rt.set_property_key_strict(
                    oid,
                    PropertyKey::String(idx.to_string()),
                    JsValue::Undefined,
                    true,
                )?;
                idx += 1;
            }
            while idx < len {
                rt.delete_property(oid, &PropertyKey::String(idx.to_string()))?;
                idx += 1;
            }
            Ok(JsValue::Object(oid))
        }),
    );

    set_method(
        rt,
        proto,
        "indexOf",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            if len == 0 {
                return Ok(JsValue::Number(-1.0));
            }
            let search = args.first().unwrap_or(&JsValue::Undefined);
            let from = if let Some(a) = args.get(1) {
                let n = to_integer_or_infinity(rt.to_number(a)?);
                if n >= len as f64 {
                    return Ok(JsValue::Number(-1.0));
                }
                if n >= 0.0 {
                    n as usize
                } else {
                    (len as f64 + n).max(0.0) as usize
                }
            } else {
                0
            };
            for k in from..len {
                let pk = PropertyKey::String(k.to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    if v.strict_eq(search) {
                        return Ok(JsValue::Number(k as f64));
                    }
                }
            }
            Ok(JsValue::Number(-1.0))
        }),
    );

    set_method(
        rt,
        proto,
        "lastIndexOf",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            if len == 0 {
                return Ok(JsValue::Number(-1.0));
            }
            let search = args.first().unwrap_or(&JsValue::Undefined);
            let start = if let Some(a) = args.get(1) {
                let n = to_integer_or_infinity(rt.to_number(a)?);
                if n >= 0.0 {
                    (n as usize).min(len - 1) as isize
                } else {
                    (len as f64 + n) as isize
                }
            } else {
                (len - 1) as isize
            };
            let mut k = start;
            while k >= 0 {
                let pk = PropertyKey::String(k.to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    if v.strict_eq(search) {
                        return Ok(JsValue::Number(k as f64));
                    }
                }
                k -= 1;
            }
            Ok(JsValue::Number(-1.0))
        }),
    );

    set_method(
        rt,
        proto,
        "includes",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            if len == 0 {
                return Ok(JsValue::Boolean(false));
            }
            let search = args.first().unwrap_or(&JsValue::Undefined);
            let from = resolve_index(rt, args.get(1), len, 0)?;
            for k in from..len {
                let v = rt.get_property(oid, &k.to_string())?;
                if v.same_value_zero(search) {
                    return Ok(JsValue::Boolean(true));
                }
            }
            Ok(JsValue::Boolean(false))
        }),
    );

    set_method(
        rt,
        proto,
        "forEach",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            for k in 0..len {
                let pk = PropertyKey::String(k.to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    rt.call_function(
                        cb.clone(),
                        this_arg.clone(),
                        &[v, JsValue::Number(k as f64), JsValue::Object(oid)],
                    )?;
                }
            }
            Ok(JsValue::Undefined)
        }),
    );

    set_method(
        rt,
        proto,
        "map",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let out = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.array_proto),
                ObjectKind::Array(ArrayData::with_len(len)),
            ));
            for k in 0..len {
                let pk = PropertyKey::String(k.to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    let mapped = rt.call_function(
                        cb.clone(),
                        this_arg.clone(),
                        &[v, JsValue::Number(k as f64), JsValue::Object(oid)],
                    )?;
                    let _ = rt.set_property(out, &k.to_string(), mapped);
                }
            }
            Ok(JsValue::Object(out))
        }),
    );

    set_method(
        rt,
        proto,
        "filter",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let mut kept = Vec::new();
            for k in 0..len {
                let pk = PropertyKey::String(k.to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    let ok = rt.call_function(
                        cb.clone(),
                        this_arg.clone(),
                        &[v.clone(), JsValue::Number(k as f64), JsValue::Object(oid)],
                    )?;
                    if ok.to_boolean() {
                        kept.push(v);
                    }
                }
            }
            Ok(JsValue::Object(rt.create_array(kept)))
        }),
    );

    set_method(
        rt,
        proto,
        "reduce",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let mut k = 0usize;
            let mut acc = if args.len() >= 2 {
                args[1].clone()
            } else {
                let mut found = None;
                while k < len {
                    let pk = PropertyKey::String(k.to_string());
                    if rt.has_property(oid, &pk)? {
                        found = Some(rt.get_property_key(oid, &pk)?);
                        k += 1;
                        break;
                    }
                    k += 1;
                }
                found.ok_or_else(|| {
                    JsError::type_err("Reduce of empty array with no initial value")
                })?
            };
            while k < len {
                let pk = PropertyKey::String(k.to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    acc = rt.call_function(
                        cb.clone(),
                        JsValue::Undefined,
                        &[acc, v, JsValue::Number(k as f64), JsValue::Object(oid)],
                    )?;
                }
                k += 1;
            }
            Ok(acc)
        }),
    );

    set_method(
        rt,
        proto,
        "reduceRight",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let mut k = len as isize - 1;
            let mut acc = if args.len() >= 2 {
                args[1].clone()
            } else {
                let mut found = None;
                while k >= 0 {
                    let pk = PropertyKey::String(k.to_string());
                    if rt.has_property(oid, &pk)? {
                        found = Some(rt.get_property_key(oid, &pk)?);
                        k -= 1;
                        break;
                    }
                    k -= 1;
                }
                found.ok_or_else(|| {
                    JsError::type_err("Reduce of empty array with no initial value")
                })?
            };
            while k >= 0 {
                let pk = PropertyKey::String(k.to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    acc = rt.call_function(
                        cb.clone(),
                        JsValue::Undefined,
                        &[acc, v, JsValue::Number(k as f64), JsValue::Object(oid)],
                    )?;
                }
                k -= 1;
            }
            Ok(acc)
        }),
    );

    set_method(
        rt,
        proto,
        "some",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            for k in 0..len {
                let pk = PropertyKey::String(k.to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    if rt
                        .call_function(
                            cb.clone(),
                            this_arg.clone(),
                            &[v, JsValue::Number(k as f64), JsValue::Object(oid)],
                        )?
                        .to_boolean()
                    {
                        return Ok(JsValue::Boolean(true));
                    }
                }
            }
            Ok(JsValue::Boolean(false))
        }),
    );

    set_method(
        rt,
        proto,
        "every",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            for k in 0..len {
                let pk = PropertyKey::String(k.to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    if !rt
                        .call_function(
                            cb.clone(),
                            this_arg.clone(),
                            &[v, JsValue::Number(k as f64), JsValue::Object(oid)],
                        )?
                        .to_boolean()
                    {
                        return Ok(JsValue::Boolean(false));
                    }
                }
            }
            Ok(JsValue::Boolean(true))
        }),
    );

    set_method(
        rt,
        proto,
        "find",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            for k in 0..len {
                let v = rt.get_property(oid, &k.to_string())?;
                if rt
                    .call_function(
                        cb.clone(),
                        this_arg.clone(),
                        &[v.clone(), JsValue::Number(k as f64), JsValue::Object(oid)],
                    )?
                    .to_boolean()
                {
                    return Ok(v);
                }
            }
            Ok(JsValue::Undefined)
        }),
    );

    set_method(
        rt,
        proto,
        "findIndex",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            for k in 0..len {
                let v = rt.get_property(oid, &k.to_string())?;
                if rt
                    .call_function(
                        cb.clone(),
                        this_arg.clone(),
                        &[v, JsValue::Number(k as f64), JsValue::Object(oid)],
                    )?
                    .to_boolean()
                {
                    return Ok(JsValue::Number(k as f64));
                }
            }
            Ok(JsValue::Number(-1.0))
        }),
    );

    set_method(
        rt,
        proto,
        "findLast",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            for k in (0..len).rev() {
                let v = rt.get_property(oid, &k.to_string())?;
                if rt
                    .call_function(
                        cb.clone(),
                        this_arg.clone(),
                        &[v.clone(), JsValue::Number(k as f64), JsValue::Object(oid)],
                    )?
                    .to_boolean()
                {
                    return Ok(v);
                }
            }
            Ok(JsValue::Undefined)
        }),
    );

    set_method(
        rt,
        proto,
        "findLastIndex",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            for k in (0..len).rev() {
                let v = rt.get_property(oid, &k.to_string())?;
                if rt
                    .call_function(
                        cb.clone(),
                        this_arg.clone(),
                        &[v, JsValue::Number(k as f64), JsValue::Object(oid)],
                    )?
                    .to_boolean()
                {
                    return Ok(JsValue::Number(k as f64));
                }
            }
            Ok(JsValue::Number(-1.0))
        }),
    );

    set_method(
        rt,
        proto,
        "flat",
        0,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let depth = match args.first() {
                None | Some(JsValue::Undefined) => 1.0,
                Some(v) => to_integer_or_infinity(rt.to_number(v)?),
            };
            let mut out = Vec::new();
            flatten_into(rt, oid, depth, &mut out)?;
            Ok(JsValue::Object(rt.create_array(out)))
        }),
    );

    set_method(
        rt,
        proto,
        "flatMap",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let mut out = Vec::new();
            for k in 0..len {
                let pk = PropertyKey::String(k.to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    let res = rt.call_function(
                        cb.clone(),
                        this_arg.clone(),
                        &[v, JsValue::Number(k as f64), JsValue::Object(oid)],
                    )?;
                    if is_array_val(rt, &res) {
                        if let JsValue::Object(rid) = res {
                            flatten_into(rt, rid, 0.0, &mut out)?;
                        }
                    } else {
                        out.push(res);
                    }
                }
            }
            Ok(JsValue::Object(rt.create_array(out)))
        }),
    );

    set_method(
        rt,
        proto,
        "fill",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let val = args.first().cloned().unwrap_or(JsValue::Undefined);
            let start = resolve_index(rt, args.get(1), len, 0)?;
            let end = resolve_index(rt, args.get(2), len, len)?;
            for k in start..end {
                rt.set_property_key_strict(oid, PropertyKey::String(k.to_string()), val.clone(), true)?;
            }
            Ok(JsValue::Object(oid))
        }),
    );

    set_method(
        rt,
        proto,
        "copyWithin",
        2,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let mut to = resolve_index(rt, args.first(), len, 0)?;
            let mut from = resolve_index(rt, args.get(1), len, 0)?;
            let end = resolve_index(rt, args.get(2), len, len)?;
            let count = end.saturating_sub(from).min(len.saturating_sub(to));
            if count > 0 {
                if from < to && to < from + count {
                    for i in (0..count).rev() {
                        let fk = PropertyKey::String((from + i).to_string());
                        let tk = PropertyKey::String((to + i).to_string());
                        if rt.has_property(oid, &fk)? {
                            let v = rt.get_property_key(oid, &fk)?;
                            rt.set_property_key_strict(oid, tk, v, true)?;
                        } else {
                            rt.delete_property(oid, &tk)?;
                        }
                    }
                } else {
                    for _ in 0..count {
                        let fk = PropertyKey::String(from.to_string());
                        let tk = PropertyKey::String(to.to_string());
                        if rt.has_property(oid, &fk)? {
                            let v = rt.get_property_key(oid, &fk)?;
                            rt.set_property_key_strict(oid, tk, v, true)?;
                        } else {
                            rt.delete_property(oid, &tk)?;
                        }
                        from += 1;
                        to += 1;
                    }
                }
            }
            Ok(JsValue::Object(oid))
        }),
    );

    set_method(
        rt,
        proto,
        "at",
        1,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let rel = to_integer_or_infinity(
                rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?,
            );
            let k = if rel >= 0.0 { rel } else { len as f64 + rel };
            if k < 0.0 || k >= len as f64 {
                Ok(JsValue::Undefined)
            } else {
                rt.get_property(oid, &(k as usize).to_string())
            }
        }),
    );

    set_method(
        rt,
        proto,
        "toReversed",
        0,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let mut out = Vec::with_capacity(len);
            for k in (0..len).rev() {
                out.push(rt.get_property(oid, &k.to_string())?);
            }
            Ok(JsValue::Object(rt.create_array(out)))
        }),
    );

    set_method(
        rt,
        proto,
        "toSorted",
        1,
        Rc::new(|rt, this, args| {
            let cmp = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !cmp.is_undefined() && !rt.is_callable(&cmp) {
                return Err(JsError::type_err("Comparison function must be a function"));
            }
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let mut items = Vec::with_capacity(len);
            let mut undefs = 0usize;
            for k in 0..len {
                let v = rt.get_property(oid, &k.to_string())?;
                if v.is_undefined() {
                    undefs += 1;
                } else {
                    items.push(v);
                }
            }
            stable_merge_sort(rt, &mut items, &cmp)?;
            for _ in 0..undefs {
                items.push(JsValue::Undefined);
            }
            Ok(JsValue::Object(rt.create_array(items)))
        }),
    );

    set_method(
        rt,
        proto,
        "toSpliced",
        2,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let start = resolve_index(rt, args.first(), len, 0)?;
            let skip = if args.is_empty() {
                0
            } else if args.len() == 1 {
                len - start
            } else {
                let dc = to_integer_or_infinity(rt.to_number(&args[1])?);
                dc.clamp(0.0, (len - start) as f64) as usize
            };
            let mut out = Vec::new();
            for k in 0..start {
                out.push(rt.get_property(oid, &k.to_string())?);
            }
            if args.len() > 2 {
                out.extend_from_slice(&args[2..]);
            }
            for k in (start + skip)..len {
                out.push(rt.get_property(oid, &k.to_string())?);
            }
            Ok(JsValue::Object(rt.create_array(out)))
        }),
    );

    set_method(
        rt,
        proto,
        "with",
        2,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let rel = to_integer_or_infinity(
                rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?,
            );
            let actual = if rel >= 0.0 { rel } else { len as f64 + rel };
            if actual < 0.0 || actual >= len as f64 {
                return Err(JsError::range("Invalid index in Array.prototype.with"));
            }
            let idx = actual as usize;
            let val = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let mut out = Vec::with_capacity(len);
            for k in 0..len {
                if k == idx {
                    out.push(val.clone());
                } else {
                    out.push(rt.get_property(oid, &k.to_string())?);
                }
            }
            Ok(JsValue::Object(rt.create_array(out)))
        }),
    );

    let values_fn = rt.create_native_function(
        "values",
        0,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            let it = rt.create_builtin_iterator(BuiltinIteratorKind::Array {
                target: oid,
                mode: IterMode::Values,
            });
            Ok(JsValue::Object(it))
        }),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::from("values"),
        PropEntry::data(JsValue::Object(values_fn), true, false, true),
    );
    let sym_iter = rt.symbols.iterator;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(sym_iter),
        PropEntry::data(JsValue::Object(values_fn), true, false, true),
    );
    set_method(
        rt,
        proto,
        "keys",
        0,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            let it = rt.create_builtin_iterator(BuiltinIteratorKind::Array {
                target: oid,
                mode: IterMode::Keys,
            });
            Ok(JsValue::Object(it))
        }),
    );
    set_method(
        rt,
        proto,
        "entries",
        0,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            let it = rt.create_builtin_iterator(BuiltinIteratorKind::Array {
                target: oid,
                mode: IterMode::Entries,
            });
            Ok(JsValue::Object(it))
        }),
    );

    let ctor = rt.create_native_constructor(
        "Array",
        1,
        proto,
        Rc::new(|rt, _, args| {
            if args.len() == 1 {
                if let JsValue::Number(n) = args[0] {
                    let u = to_uint32(n);
                    if u as f64 != n {
                        return Err(JsError::range("Invalid array length"));
                    }
                    let obj = rt.heap.alloc_object(JsObject::new(
                        Some(rt.intrinsics.array_proto),
                        ObjectKind::Array(ArrayData::with_len(u as usize)),
                    ));
                    return Ok(JsValue::Object(obj));
                }
            }
            Ok(JsValue::Object(rt.create_array(args.to_vec())))
        }),
    );
    rt.intrinsics.array_ctor = ctor;

    set_method(
        rt,
        ctor,
        "isArray",
        1,
        Rc::new(|rt, _, args| {
            let v = args.first().unwrap_or(&JsValue::Undefined);
            Ok(JsValue::Boolean(is_array_val(rt, v)))
        }),
    );

    set_method(
        rt,
        ctor,
        "of",
        0,
        Rc::new(|rt, _, args| Ok(JsValue::Object(rt.create_array(args.to_vec())))),
    );

    set_method(
        rt,
        ctor,
        "from",
        1,
        Rc::new(|rt, this, args| {
            let items_arg = args.first().unwrap_or(&JsValue::Undefined);
            let map_fn = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let this_arg = args.get(2).cloned().unwrap_or(JsValue::Undefined);
            if !map_fn.is_undefined() && !rt.is_callable(&map_fn) {
                return Err(JsError::type_err("Array.from mapfn is not a function"));
            }
            let sym_iter = PropertyKey::Symbol(rt.symbols.iterator);
            let oid = rt.to_object(items_arg)?;
            let iter_method = rt.get_property_key(oid, &sym_iter)?;
            if !iter_method.is_nullish() {
                let out_id = if rt.is_constructor(&this) && this != JsValue::Object(rt.intrinsics.array_ctor) {
                    let v = rt.construct(this, &[])?;
                    rt.to_object(&v)?
                } else {
                    rt.create_array(Vec::new())
                };
                if !rt.is_callable(&iter_method) {
                    return Err(JsError::type_err("Iterator method is not callable"));
                }
                let iter = rt.call_function(iter_method, items_arg.clone(), &[])?;
                let mut k = 0usize;
                while let Some(item) = rt.iterator_step(&iter)? {
                    let val = if !map_fn.is_undefined() {
                        match rt.call_function(
                            map_fn.clone(),
                            this_arg.clone(),
                            &[item, JsValue::Number(k as f64)],
                        ) {
                            Ok(v) => v,
                            Err(e) => {
                                rt.iterator_close(&iter);
                                return Err(e);
                            }
                        }
                    } else {
                        item
                    };
                    if !rt.define_property(
                        out_id,
                        PropertyKey::String(k.to_string()),
                        PropertyDescriptor::data(val, true, true, true),
                    )? {
                        return Err(JsError::type_err("Cannot define property"));
                    }
                    k += 1;
                }
                rt.set_property_key_strict(
                    out_id,
                    PropertyKey::from("length"),
                    JsValue::Number(k as f64),
                    true,
                )?;
                return Ok(JsValue::Object(out_id));
            }
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let out_id = if rt.is_constructor(&this) && this != JsValue::Object(rt.intrinsics.array_ctor) {
                let v = rt.construct(this, &[JsValue::Number(len as f64)])?;
                rt.to_object(&v)?
            } else {
                rt.heap.alloc_object(JsObject::new(
                    Some(rt.intrinsics.array_proto),
                    ObjectKind::Array(ArrayData::with_len(len)),
                ))
            };
            for k in 0..len {
                let item = rt.get_property(oid, &k.to_string())?;
                let val = if !map_fn.is_undefined() {
                    rt.call_function(
                        map_fn.clone(),
                        this_arg.clone(),
                        &[item, JsValue::Number(k as f64)],
                    )?
                } else {
                    item
                };
                if !rt.define_property(
                    out_id,
                    PropertyKey::String(k.to_string()),
                    PropertyDescriptor::data(val, true, true, true),
                )? {
                    return Err(JsError::type_err("Cannot define property"));
                }
            }
            rt.set_property_key_strict(
                out_id,
                PropertyKey::from("length"),
                JsValue::Number(len as f64),
                true,
            )?;
            Ok(JsValue::Object(out_id))
        }),
    );

    let sym_species = rt.symbols.species;
    let species_getter = rt.create_native_function("get [Symbol.species]", 0, Rc::new(|_, this, _| Ok(this)));
    let _ = rt.define_property(
        ctor,
        PropertyKey::Symbol(sym_species),
        PropertyDescriptor {
            value: None,
            writable: None,
            get: Some(JsValue::Object(species_getter)),
            set: None,
            enumerable: Some(false),
            configurable: Some(true),
        },
    );

    let _ = set_symbol_method;
    set_global(rt, "Array", JsValue::Object(ctor));
}

pub fn is_array_val(rt: &JsRuntime, val: &JsValue) -> bool {
    let JsValue::Object(oid) = val else {
        return false;
    };
    match &rt.heap.get_obj(*oid).kind {
        ObjectKind::Array(_) => true,
        ObjectKind::Proxy(p) => !p.revoked && is_array_val(rt, &JsValue::Object(p.target)),
        _ => false,
    }
}

fn is_concat_spreadable(rt: &mut JsRuntime, val: &JsValue) -> Result<bool, JsError> {
    let JsValue::Object(oid) = val else {
        return Ok(false);
    };
    let key = PropertyKey::Symbol(rt.symbols.is_concat_spreadable);
    let sp = rt.get_property_key(*oid, &key)?;
    if !sp.is_undefined() {
        return Ok(sp.to_boolean());
    }
    Ok(is_array_val(rt, val))
}

fn resolve_index(
    rt: &mut JsRuntime,
    arg: Option<&JsValue>,
    len: usize,
    default: usize,
) -> Result<usize, JsError> {
    match arg {
        None | Some(JsValue::Undefined) => Ok(default),
        Some(v) => {
            let n = to_integer_or_infinity(rt.to_number(v)?);
            if n < 0.0 {
                Ok((len as f64 + n).max(0.0) as usize)
            } else {
                Ok((n as usize).min(len))
            }
        }
    }
}

fn flatten_into(
    rt: &mut JsRuntime,
    src: ObjectId,
    depth: f64,
    out: &mut Vec<JsValue>,
) -> Result<(), JsError> {
    let len_v = rt.get_property(src, "length")?;
    let len = to_length(rt.to_number(&len_v)?);
    for k in 0..len {
        let pk = PropertyKey::String(k.to_string());
        if rt.has_property(src, &pk)? {
            let v = rt.get_property_key(src, &pk)?;
            if depth > 0.0 && is_array_val(rt, &v) {
                if let JsValue::Object(sub_id) = v {
                    flatten_into(rt, sub_id, depth - 1.0, out)?;
                }
            } else {
                out.push(v);
            }
        }
    }
    Ok(())
}

fn compare_sort_elements(
    rt: &mut JsRuntime,
    a: &JsValue,
    b: &JsValue,
    cmp: &JsValue,
) -> Result<std::cmp::Ordering, JsError> {
    if !cmp.is_undefined() {
        let res = rt.call_function(cmp.clone(), JsValue::Undefined, &[a.clone(), b.clone()])?;
        let n = rt.to_number(&res)?;
        if n < 0.0 {
            return Ok(std::cmp::Ordering::Less);
        } else if n > 0.0 {
            return Ok(std::cmp::Ordering::Greater);
        } else {
            return Ok(std::cmp::Ordering::Equal);
        }
    }
    let sa = rt.to_string(a)?;
    let sb = rt.to_string(b)?;
    let ua: Vec<u16> = sa.encode_utf16().collect();
    let ub: Vec<u16> = sb.encode_utf16().collect();
    Ok(ua.cmp(&ub))
}

pub fn stable_merge_sort(
    rt: &mut JsRuntime,
    arr: &mut [JsValue],
    cmp: &JsValue,
) -> Result<(), JsError> {
    let n = arr.len();
    if n <= 1 {
        return Ok(());
    }
    let mid = n / 2;
    stable_merge_sort(rt, &mut arr[..mid], cmp)?;
    stable_merge_sort(rt, &mut arr[mid..], cmp)?;
    let left = arr[..mid].to_vec();
    let right = arr[mid..].to_vec();
    let mut i = 0;
    let mut j = 0;
    let mut k = 0;
    while i < left.len() && j < right.len() {
        if compare_sort_elements(rt, &left[i], &right[j], cmp)? != std::cmp::Ordering::Greater {
            arr[k] = left[i].clone();
            i += 1;
        } else {
            arr[k] = right[j].clone();
            j += 1;
        }
        k += 1;
    }
    while i < left.len() {
        arr[k] = left[i].clone();
        i += 1;
        k += 1;
    }
    while j < right.len() {
        arr[k] = right[j].clone();
        j += 1;
        k += 1;
    }
    Ok(())
}
