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
            const MAX_SAFE: usize = 9_007_199_254_740_991;
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let mut len = to_length(rt.to_number(&len_v)?);
            if len.saturating_add(args.len()) > MAX_SAFE {
                return Err(JsError::type_err("Invalid array length"));
            }
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
            const MAX_SAFE: usize = 9_007_199_254_740_991;
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let ac = args.len();
            if ac > 0 {
                if len.saturating_add(ac) > MAX_SAFE {
                    return Err(JsError::type_err("Invalid array length"));
                }
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
            let count = end.saturating_sub(start);
            let out = array_species_create(rt, oid, count)?;
            let mut n = 0usize;
            for k in start..end {
                let pk = PropertyKey::String(k.to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    create_data_property_or_throw(rt, out, PropertyKey::String(n.to_string()), v)?;
                }
                n += 1;
            }
            rt.set_property_key_strict(out, PropertyKey::from("length"), JsValue::Number(n as f64), true)?;
            Ok(JsValue::Object(out))
        }),
    );

    set_method(
        rt,
        proto,
        "splice",
        2,
        Rc::new(|rt, this, args| {
            const MAX_SAFE: usize = 9_007_199_254_740_991;
            let oid = rt.to_object(&this)?;
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let start = resolve_index(rt, args.first(), len, 0)?;
            let (ic, del_count) = if args.is_empty() {
                (0usize, 0usize)
            } else if args.len() == 1 {
                (0usize, len - start)
            } else {
                let dc = to_integer_or_infinity(rt.to_number(&args[1])?);
                (args.len() - 2, dc.clamp(0.0, (len - start) as f64) as usize)
            };
            if (len - del_count).saturating_add(ic) > MAX_SAFE {
                return Err(JsError::type_err("Invalid array length"));
            }
            let deleted = array_species_create(rt, oid, del_count)?;
            for k in 0..del_count {
                let pk = PropertyKey::String((start + k).to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    create_data_property_or_throw(rt, deleted, PropertyKey::String(k.to_string()), v)?;
                }
            }
            rt.set_property_key_strict(
                deleted,
                PropertyKey::from("length"),
                JsValue::Number(del_count as f64),
                true,
            )?;
            let insert = if args.len() > 2 { &args[2..] } else { &[] };
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
            const MAX_SAFE: usize = 9_007_199_254_740_991;
            let oid = rt.to_object(&this)?;
            let out = array_species_create(rt, oid, 0)?;
            let mut items = vec![JsValue::Object(oid)];
            items.extend_from_slice(args);
            let mut n = 0usize;
            for item in items {
                if is_concat_spreadable(rt, &item)? {
                    let item_id = rt.to_object(&item)?;
                    let len_v = rt.get_property(item_id, "length")?;
                    let len = to_length(rt.to_number(&len_v)?);
                    if n.saturating_add(len) > MAX_SAFE {
                        return Err(JsError::type_err("Invalid array length"));
                    }
                    for k in 0..len {
                        let pk = PropertyKey::String(k.to_string());
                        if rt.has_property(item_id, &pk)? {
                            let v = rt.get_property_key(item_id, &pk)?;
                            create_data_property_or_throw(rt, out, PropertyKey::String(n.to_string()), v)?;
                        }
                        n += 1;
                    }
                } else {
                    if n >= MAX_SAFE {
                        return Err(JsError::type_err("Invalid array length"));
                    }
                    create_data_property_or_throw(rt, out, PropertyKey::String(n.to_string()), item)?;
                    n += 1;
                }
            }
            rt.set_property_key_strict(out, PropertyKey::from("length"), JsValue::Number(n as f64), true)?;
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
                let lv = if l_exists {
                    Some(rt.get_property_key(oid, &lk)?)
                } else {
                    None
                };
                let h_exists = rt.has_property(oid, &hk)?;
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
            let out = array_species_create(rt, oid, len)?;
            for k in 0..len {
                let pk = PropertyKey::String(k.to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    let mapped = rt.call_function(
                        cb.clone(),
                        this_arg.clone(),
                        &[v, JsValue::Number(k as f64), JsValue::Object(oid)],
                    )?;
                    create_data_property_or_throw(rt, out, pk, mapped)?;
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
            let out = array_species_create(rt, oid, 0)?;
            let mut to = 0usize;
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
                        create_data_property_or_throw(rt, out, PropertyKey::String(to.to_string()), v)?;
                        to += 1;
                    }
                }
            }
            Ok(JsValue::Object(out))
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
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let depth = match args.first() {
                None | Some(JsValue::Undefined) => 1.0,
                Some(v) => to_integer_or_infinity(rt.to_number(v)?),
            };
            let out = array_species_create(rt, oid, 0)?;
            flatten_into_array(rt, out, oid, len, 0, depth, None, &JsValue::Undefined)?;
            Ok(JsValue::Object(out))
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
            let out = array_species_create(rt, oid, 0)?;
            flatten_into_array(rt, out, oid, len, 0, 1.0, Some(&cb), &this_arg)?;
            Ok(JsValue::Object(out))
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
                        } else if !rt.delete_property(oid, &tk)? {
                            return Err(JsError::type_err("Cannot delete property"));
                        }
                    }
                } else {
                    for _ in 0..count {
                        let fk = PropertyKey::String(from.to_string());
                        let tk = PropertyKey::String(to.to_string());
                        if rt.has_property(oid, &fk)? {
                            let v = rt.get_property_key(oid, &fk)?;
                            rt.set_property_key_strict(oid, tk, v, true)?;
                        } else if !rt.delete_property(oid, &tk)? {
                            return Err(JsError::type_err("Cannot delete property"));
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
            if len > u32::MAX as usize {
                return Err(JsError::range("Invalid array length"));
            }
            let mut out = Vec::with_capacity(len.min(1024));
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
            if len > u32::MAX as usize {
                return Err(JsError::range("Invalid array length"));
            }
            let mut items = Vec::with_capacity(len.min(1024));
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
            let (insert_count, skip) = if args.is_empty() {
                (0usize, 0usize)
            } else if args.len() == 1 {
                (0usize, len - start)
            } else {
                let dc = to_integer_or_infinity(rt.to_number(&args[1])?);
                (args.len() - 2, dc.clamp(0.0, (len - start) as f64) as usize)
            };
            let new_len = (len - skip).saturating_add(insert_count);
            if new_len > 9_007_199_254_740_991 {
                return Err(JsError::type_err("Invalid array length"));
            }
            if new_len > u32::MAX as usize {
                return Err(JsError::range("Invalid array length"));
            }
            let mut out = Vec::with_capacity(new_len.min(1024));
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
            if len > u32::MAX as usize {
                return Err(JsError::range("Invalid array length"));
            }
            let idx = actual as usize;
            let val = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let mut out = Vec::with_capacity(len.min(1024));
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

    let unscopables = rt.create_object_with_proto(None);
    for name in [
        "at",
        "copyWithin",
        "entries",
        "fill",
        "find",
        "findIndex",
        "findLast",
        "findLastIndex",
        "flat",
        "flatMap",
        "includes",
        "keys",
        "toReversed",
        "toSorted",
        "toSpliced",
        "values",
    ] {
        rt.heap.get_obj_mut(unscopables).set_own_prop(
            PropertyKey::from(name),
            PropEntry::data(JsValue::Boolean(true), true, true, true),
        );
    }
    let sym_unscopables = rt.symbols.unscopables;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(sym_unscopables),
        PropEntry::data(JsValue::Object(unscopables), false, false, true),
    );

    let ctor = rt.create_spec_constructor(
        "Array",
        1,
        proto,
        Rc::new(|rt, _, args| {
            let obj_proto = rt.get_prototype_from_new_target(rt.intrinsics.array_proto)?;
            if args.len() == 1 {
                if let JsValue::Number(n) = args[0] {
                    let u = to_uint32(n);
                    if u as f64 != n {
                        return Err(JsError::range("Invalid array length"));
                    }
                    let obj = rt.heap.alloc_object(JsObject::new(
                        Some(obj_proto),
                        ObjectKind::Array(ArrayData::with_len(u as usize)),
                    ));
                    return Ok(JsValue::Object(obj));
                }
            }
            let obj = rt.heap.alloc_object(JsObject::new(
                Some(obj_proto),
                ObjectKind::Array(ArrayData::from_vec(args.to_vec())),
            ));
            Ok(JsValue::Object(obj))
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
            Ok(JsValue::Boolean(is_array_val_checked(rt, v)?))
        }),
    );

    set_method(
        rt,
        ctor,
        "of",
        0,
        Rc::new(|rt, this, args| {
            let len = args.len();
            let out_id = if rt.is_constructor(&this) {
                let v = rt.construct(this, &[JsValue::Number(len as f64)])?;
                rt.to_object(&v)?
            } else {
                rt.heap.alloc_object(JsObject::new(
                    Some(rt.intrinsics.array_proto),
                    ObjectKind::Array(ArrayData::with_len(len)),
                ))
            };
            for (k, item) in args.iter().enumerate() {
                create_data_property_or_throw(
                    rt,
                    out_id,
                    PropertyKey::String(k.to_string()),
                    item.clone(),
                )?;
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
                    if let Err(e) = create_data_property_or_throw(
                        rt,
                        out_id,
                        PropertyKey::String(k.to_string()),
                        val,
                    ) {
                        rt.iterator_close(&iter);
                        return Err(e);
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
                create_data_property_or_throw(
                    rt,
                    out_id,
                    PropertyKey::String(k.to_string()),
                    val,
                )?;
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

    set_method(
        rt,
        ctor,
        "fromAsync",
        1,
        Rc::new(|rt, this, args| {
            let promise = rt.create_promise();
            let async_items = args.first().cloned().unwrap_or(JsValue::Undefined);
            let map_fn = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let this_arg = args.get(2).cloned().unwrap_or(JsValue::Undefined);
            if let Err(e) = array_from_async_start(rt, this, async_items, map_fn, this_arg, promise) {
                let ev = rt.error_to_js_value(e);
                rt.reject_promise(promise, ev);
            }
            Ok(JsValue::Object(promise))
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

pub fn is_array_val_checked(rt: &JsRuntime, val: &JsValue) -> Result<bool, JsError> {
    let JsValue::Object(oid) = val else {
        return Ok(false);
    };
    match &rt.heap.get_obj(*oid).kind {
        ObjectKind::Array(_) => Ok(true),
        ObjectKind::Proxy(p) => {
            if p.revoked {
                return Err(JsError::type_err("Cannot perform 'IsArray' on a proxy that has been revoked"));
            }
            let target = p.target;
            is_array_val_checked(rt, &JsValue::Object(target))
        }
        _ => Ok(false),
    }
}

pub fn is_array_val(rt: &JsRuntime, val: &JsValue) -> bool {
    is_array_val_checked(rt, val).unwrap_or(false)
}

pub fn create_data_property_or_throw(
    rt: &mut JsRuntime,
    oid: ObjectId,
    key: PropertyKey,
    val: JsValue,
) -> Result<(), JsError> {
    if !rt.define_property(oid, key, PropertyDescriptor::data(val, true, true, true))? {
        return Err(JsError::type_err("Cannot define property"));
    }
    Ok(())
}

pub fn array_species_create(
    rt: &mut JsRuntime,
    orig: ObjectId,
    length: usize,
) -> Result<ObjectId, JsError> {
    let is_arr = is_array_val_checked(rt, &JsValue::Object(orig))?;
    if !is_arr {
        if length > u32::MAX as usize {
            return Err(JsError::range("Invalid array length"));
        }
        return Ok(rt.heap.alloc_object(JsObject::new(
            Some(rt.intrinsics.array_proto),
            ObjectKind::Array(ArrayData::with_len(length)),
        )));
    }
    let mut ctor = rt.get_property(orig, "constructor")?;
    if let JsValue::Object(cid) = ctor {
        let mut is_other_realm_array = false;
        if rt.is_constructor(&JsValue::Object(cid)) {
            let realm_c = rt.get_function_realm(cid)?;
            if realm_c != rt.current_realm_id
                && realm_c < rt.realms.len()
                && cid == rt.realms[realm_c].intrinsics.array_ctor
            {
                is_other_realm_array = true;
            }
        }
        if is_other_realm_array {
            ctor = JsValue::Undefined;
        } else {
            let sym_species = PropertyKey::Symbol(rt.symbols.species);
            let sp = rt.get_property_key(cid, &sym_species)?;
            ctor = if sp.is_null() { JsValue::Undefined } else { sp };
        }
    }
    if ctor.is_undefined() {
        if length > u32::MAX as usize {
            return Err(JsError::range("Invalid array length"));
        }
        return Ok(rt.heap.alloc_object(JsObject::new(
            Some(rt.intrinsics.array_proto),
            ObjectKind::Array(ArrayData::with_len(length)),
        )));
    }
    if !rt.is_constructor(&ctor) {
        return Err(JsError::type_err("Array species constructor is not a constructor"));
    }
    let res = rt.construct(ctor, &[JsValue::Number(length as f64)])?;
    rt.to_object(&res)
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
    is_array_val_checked(rt, val)
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

fn flatten_into_array(
    rt: &mut JsRuntime,
    target: ObjectId,
    src: ObjectId,
    src_len: usize,
    start: usize,
    depth: f64,
    mapper: Option<&JsValue>,
    this_arg: &JsValue,
) -> Result<usize, JsError> {
    let mut target_index = start;
    for k in 0..src_len {
        let pk = PropertyKey::String(k.to_string());
        if rt.has_property(src, &pk)? {
            let mut elem = rt.get_property_key(src, &pk)?;
            if let Some(m) = mapper {
                elem = rt.call_function(
                    m.clone(),
                    this_arg.clone(),
                    &[elem, JsValue::Number(k as f64), JsValue::Object(src)],
                )?;
            }
            let should_flatten = if depth > 0.0 {
                is_array_val_checked(rt, &elem)?
            } else {
                false
            };
            if should_flatten {
                if let JsValue::Object(sub_id) = elem {
                    let sub_len_v = rt.get_property(sub_id, "length")?;
                    let sub_len = to_length(rt.to_number(&sub_len_v)?);
                    let next_depth = if depth.is_infinite() { depth } else { depth - 1.0 };
                    target_index = flatten_into_array(
                        rt,
                        target,
                        sub_id,
                        sub_len,
                        target_index,
                        next_depth,
                        None,
                        &JsValue::Undefined,
                    )?;
                }
            } else {
                create_data_property_or_throw(
                    rt,
                    target,
                    PropertyKey::String(target_index.to_string()),
                    elem,
                )?;
                target_index += 1;
            }
        }
    }
    Ok(target_index)
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

struct FromAsyncIterState {
    iter: JsValue,
    next_fn: JsValue,
    is_async_iter: bool,
    out_id: ObjectId,
    map_fn: Option<JsValue>,
    this_arg: JsValue,
    k: usize,
    promise: ObjectId,
}

fn close_from_async_iter(
    rt: &mut JsRuntime,
    iter: JsValue,
    is_async_iter: bool,
    reason: JsValue,
    promise: ObjectId,
) {
    let ret_res = (|| -> Result<Option<JsValue>, JsError> {
        let JsValue::Object(iid) = iter else {
            return Ok(None);
        };
        let ret_fn = rt.get_property(iid, "return")?;
        if ret_fn.is_nullish() {
            return Ok(None);
        }
        if !rt.is_callable(&ret_fn) {
            return Err(JsError::type_err("Iterator return is not callable"));
        }
        let r = rt.call_function(ret_fn, iter, &[])?;
        Ok(Some(r))
    })();
    if is_async_iter {
        if let Ok(Some(ret_val)) = ret_res {
            let p = rt.promise_resolve_value(ret_val);
            let r1 = reason.clone();
            let on_fin1 = rt.create_native_function(
                "",
                1,
                Rc::new(move |rt, _, _| {
                    rt.reject_promise(promise, r1.clone());
                    Ok(JsValue::Undefined)
                }),
            );
            let on_fin2 = rt.create_native_function(
                "",
                1,
                Rc::new(move |rt, _, _| {
                    rt.reject_promise(promise, reason.clone());
                    Ok(JsValue::Undefined)
                }),
            );
            let _ = rt.promise_then(
                p,
                Some(JsValue::Object(on_fin1)),
                Some(JsValue::Object(on_fin2)),
            );
            return;
        }
    }
    rt.reject_promise(promise, reason);
}

fn array_from_async_start(
    rt: &mut JsRuntime,
    this: JsValue,
    async_items: JsValue,
    map_fn: JsValue,
    this_arg: JsValue,
    promise: ObjectId,
) -> Result<(), JsError> {
    let mapping = if map_fn.is_undefined() {
        None
    } else if rt.is_callable(&map_fn) {
        Some(map_fn)
    } else {
        return Err(JsError::type_err("Array.fromAsync mapfn is not a function"));
    };
    let items_oid = rt.to_object(&async_items)?;
    let async_sym = PropertyKey::Symbol(rt.symbols.async_iterator);
    let using_async = rt.get_property_key(items_oid, &async_sym)?;
    let (using_method, is_async_iter) = if !using_async.is_nullish() {
        if !rt.is_callable(&using_async) {
            return Err(JsError::type_err("Async iterator method is not callable"));
        }
        (Some(using_async), true)
    } else {
        let sync_sym = PropertyKey::Symbol(rt.symbols.iterator);
        let using_sync = rt.get_property_key(items_oid, &sync_sym)?;
        if !using_sync.is_nullish() {
            if !rt.is_callable(&using_sync) {
                return Err(JsError::type_err("Iterator method is not callable"));
            }
            (Some(using_sync), false)
        } else {
            (None, false)
        }
    };

    if let Some(method) = using_method {
        let iter = rt.call_function(method, async_items, &[])?;
        let JsValue::Object(iter_oid) = iter else {
            return Err(JsError::type_err("Iterator must be an object"));
        };
        let next_fn = rt.get_property(iter_oid, "next")?;
        let out_id = if rt.is_constructor(&this) && this != JsValue::Object(rt.intrinsics.array_ctor) {
            let v = rt.construct(this, &[])?;
            rt.to_object(&v)?
        } else {
            rt.create_array(Vec::new())
        };
        let state = Rc::new(std::cell::RefCell::new(FromAsyncIterState {
            iter,
            next_fn,
            is_async_iter,
            out_id,
            map_fn: mapping,
            this_arg,
            k: 0,
            promise,
        }));
        step_from_async_iter(rt, state);
        return Ok(());
    }

    let len_v = rt.get_property(items_oid, "length")?;
    let len = to_length(rt.to_number(&len_v)?);
    let out_id = if rt.is_constructor(&this) && this != JsValue::Object(rt.intrinsics.array_ctor) {
        let v = rt.construct(this, &[JsValue::Number(len as f64)])?;
        rt.to_object(&v)?
    } else {
        if len > u32::MAX as usize {
            return Err(JsError::range("Invalid array length"));
        }
        rt.heap.alloc_object(JsObject::new(
            Some(rt.intrinsics.array_proto),
            ObjectKind::Array(ArrayData::with_len(len)),
        ))
    };
    step_from_async_array_like(rt, items_oid, len, 0, out_id, mapping, this_arg, promise);
    Ok(())
}

fn step_from_async_iter(rt: &mut JsRuntime, state: Rc<std::cell::RefCell<FromAsyncIterState>>) {
    let (iter, next_fn, is_async_iter, promise) = {
        let st = state.borrow();
        (st.iter.clone(), st.next_fn.clone(), st.is_async_iter, st.promise)
    };
    let next_res = match rt.call_function(next_fn, iter, &[]) {
        Ok(v) => v,
        Err(e) => {
            let ev = rt.error_to_js_value(e);
            rt.reject_promise(promise, ev);
            return;
        }
    };
    if is_async_iter {
        let p = rt.promise_resolve_value(next_res);
        let st_ok = state.clone();
        let on_ok = rt.create_native_function(
            "",
            1,
            Rc::new(move |rt, _, a| {
                let res = a.first().cloned().unwrap_or(JsValue::Undefined);
                handle_from_async_iter_result(rt, st_ok.clone(), res);
                Ok(JsValue::Undefined)
            }),
        );
        let on_err = rt.create_native_function(
            "",
            1,
            Rc::new(move |rt, _, a| {
                let r = a.first().cloned().unwrap_or(JsValue::Undefined);
                rt.reject_promise(promise, r);
                Ok(JsValue::Undefined)
            }),
        );
        let _ = rt.promise_then(p, Some(JsValue::Object(on_ok)), Some(JsValue::Object(on_err)));
    } else {
        handle_from_async_iter_result(rt, state, next_res);
    }
}

fn handle_from_async_iter_result(
    rt: &mut JsRuntime,
    state: Rc<std::cell::RefCell<FromAsyncIterState>>,
    iter_result: JsValue,
) {
    let (iter, is_async_iter, out_id, k, promise) = {
        let st = state.borrow();
        (st.iter.clone(), st.is_async_iter, st.out_id, st.k, st.promise)
    };
    let JsValue::Object(res_oid) = iter_result else {
        let err = rt.error_to_js_value(JsError::type_err("Iterator result is not an object"));
        rt.reject_promise(promise, err);
        return;
    };
    let done = match rt.get_property(res_oid, "done") {
        Ok(v) => v.to_boolean(),
        Err(e) => {
            let ev = rt.error_to_js_value(e);
            rt.reject_promise(promise, ev);
            return;
        }
    };
    if done {
        if let Err(e) = rt.set_property_key_strict(
            out_id,
            PropertyKey::from("length"),
            JsValue::Number(k as f64),
            true,
        ) {
            let ev = rt.error_to_js_value(e);
            rt.reject_promise(promise, ev);
        } else {
            rt.resolve_promise(promise, JsValue::Object(out_id));
        }
        return;
    }
    let next_val = match rt.get_property(res_oid, "value") {
        Ok(v) => v,
        Err(e) => {
            let ev = rt.error_to_js_value(e);
            rt.reject_promise(promise, ev);
            return;
        }
    };
    if is_async_iter {
        apply_from_async_iter_value(rt, state, next_val);
    } else {
        let p = rt.promise_resolve_value(next_val);
        let st_ok = state.clone();
        let on_ok = rt.create_native_function(
            "",
            1,
            Rc::new(move |rt, _, a| {
                let v = a.first().cloned().unwrap_or(JsValue::Undefined);
                apply_from_async_iter_value(rt, st_ok.clone(), v);
                Ok(JsValue::Undefined)
            }),
        );
        let on_err = rt.create_native_function(
            "",
            1,
            Rc::new(move |rt, _, a| {
                let r = a.first().cloned().unwrap_or(JsValue::Undefined);
                close_from_async_iter(rt, iter.clone(), false, r, promise);
                Ok(JsValue::Undefined)
            }),
        );
        let _ = rt.promise_then(p, Some(JsValue::Object(on_ok)), Some(JsValue::Object(on_err)));
    }
}

fn apply_from_async_iter_value(
    rt: &mut JsRuntime,
    state: Rc<std::cell::RefCell<FromAsyncIterState>>,
    val: JsValue,
) {
    let (iter, is_async_iter, map_fn, this_arg, k, promise) = {
        let st = state.borrow();
        (
            st.iter.clone(),
            st.is_async_iter,
            st.map_fn.clone(),
            st.this_arg.clone(),
            st.k,
            st.promise,
        )
    };
    if k >= 9_007_199_254_740_991 {
        let ev = rt.error_to_js_value(JsError::type_err("Array.fromAsync length exceeded"));
        close_from_async_iter(rt, iter, is_async_iter, ev, promise);
        return;
    }
    if let Some(mfn) = map_fn {
        let mapped = match rt.call_function(mfn, this_arg, &[val, JsValue::Number(k as f64)]) {
            Ok(v) => v,
            Err(e) => {
                let ev = rt.error_to_js_value(e);
                close_from_async_iter(rt, iter, is_async_iter, ev, promise);
                return;
            }
        };
        let p = rt.promise_resolve_value(mapped);
        let st_ok = state.clone();
        let on_ok = rt.create_native_function(
            "",
            1,
            Rc::new(move |rt, _, a| {
                let v = a.first().cloned().unwrap_or(JsValue::Undefined);
                finish_from_async_iter_elem(rt, st_ok.clone(), v);
                Ok(JsValue::Undefined)
            }),
        );
        let on_err = rt.create_native_function(
            "",
            1,
            Rc::new(move |rt, _, a| {
                let r = a.first().cloned().unwrap_or(JsValue::Undefined);
                close_from_async_iter(rt, iter.clone(), is_async_iter, r, promise);
                Ok(JsValue::Undefined)
            }),
        );
        let _ = rt.promise_then(p, Some(JsValue::Object(on_ok)), Some(JsValue::Object(on_err)));
    } else {
        finish_from_async_iter_elem(rt, state, val);
    }
}

fn finish_from_async_iter_elem(
    rt: &mut JsRuntime,
    state: Rc<std::cell::RefCell<FromAsyncIterState>>,
    val: JsValue,
) {
    let (iter, is_async_iter, out_id, k, promise) = {
        let st = state.borrow();
        (st.iter.clone(), st.is_async_iter, st.out_id, st.k, st.promise)
    };
    if let Err(e) = create_data_property_or_throw(rt, out_id, PropertyKey::String(k.to_string()), val) {
        let ev = rt.error_to_js_value(e);
        close_from_async_iter(rt, iter, is_async_iter, ev, promise);
        return;
    }
    state.borrow_mut().k = k + 1;
    step_from_async_iter(rt, state);
}

fn step_from_async_array_like(
    rt: &mut JsRuntime,
    items_oid: ObjectId,
    len: usize,
    k: usize,
    out_id: ObjectId,
    map_fn: Option<JsValue>,
    this_arg: JsValue,
    promise: ObjectId,
) {
    if k >= len {
        if let Err(e) = rt.set_property_key_strict(
            out_id,
            PropertyKey::from("length"),
            JsValue::Number(len as f64),
            true,
        ) {
            let ev = rt.error_to_js_value(e);
            rt.reject_promise(promise, ev);
        } else {
            rt.resolve_promise(promise, JsValue::Object(out_id));
        }
        return;
    }
    let raw = match rt.get_property(items_oid, &k.to_string()) {
        Ok(v) => v,
        Err(e) => {
            let ev = rt.error_to_js_value(e);
            rt.reject_promise(promise, ev);
            return;
        }
    };
    let p = rt.promise_resolve_value(raw);
    let mfn_c = map_fn.clone();
    let ta_c = this_arg.clone();
    let on_ok = rt.create_native_function(
        "",
        1,
        Rc::new(move |rt, _, a| {
            let val = a.first().cloned().unwrap_or(JsValue::Undefined);
            if let Some(ref mfn) = mfn_c {
                let mapped = match rt.call_function(
                    mfn.clone(),
                    ta_c.clone(),
                    &[val, JsValue::Number(k as f64)],
                ) {
                    Ok(v) => v,
                    Err(e) => {
                        let ev = rt.error_to_js_value(e);
                        rt.reject_promise(promise, ev);
                        return Ok(JsValue::Undefined);
                    }
                };
                let mp = rt.promise_resolve_value(mapped);
                let mfn2 = mfn_c.clone();
                let ta2 = ta_c.clone();
                let on_m_ok = rt.create_native_function(
                    "",
                    1,
                    Rc::new(move |rt, _, ma| {
                        let mv = ma.first().cloned().unwrap_or(JsValue::Undefined);
                        if let Err(e) = create_data_property_or_throw(
                            rt,
                            out_id,
                            PropertyKey::String(k.to_string()),
                            mv,
                        ) {
                            let ev = rt.error_to_js_value(e);
                            rt.reject_promise(promise, ev);
                            return Ok(JsValue::Undefined);
                        }
                        step_from_async_array_like(
                            rt,
                            items_oid,
                            len,
                            k + 1,
                            out_id,
                            mfn2.clone(),
                            ta2.clone(),
                            promise,
                        );
                        Ok(JsValue::Undefined)
                    }),
                );
                let on_m_err = rt.create_native_function(
                    "",
                    1,
                    Rc::new(move |rt, _, ma| {
                        let r = ma.first().cloned().unwrap_or(JsValue::Undefined);
                        rt.reject_promise(promise, r);
                        Ok(JsValue::Undefined)
                    }),
                );
                let _ = rt.promise_then(
                    mp,
                    Some(JsValue::Object(on_m_ok)),
                    Some(JsValue::Object(on_m_err)),
                );
            } else {
                if let Err(e) = create_data_property_or_throw(
                    rt,
                    out_id,
                    PropertyKey::String(k.to_string()),
                    val,
                ) {
                    let ev = rt.error_to_js_value(e);
                    rt.reject_promise(promise, ev);
                    return Ok(JsValue::Undefined);
                }
                step_from_async_array_like(
                    rt,
                    items_oid,
                    len,
                    k + 1,
                    out_id,
                    None,
                    ta_c.clone(),
                    promise,
                );
            }
            Ok(JsValue::Undefined)
        }),
    );
    let on_err = rt.create_native_function(
        "",
        1,
        Rc::new(move |rt, _, a| {
            let r = a.first().cloned().unwrap_or(JsValue::Undefined);
            rt.reject_promise(promise, r);
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.promise_then(p, Some(JsValue::Object(on_ok)), Some(JsValue::Object(on_err)));
}
