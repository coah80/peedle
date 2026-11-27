use crate::builtins::{set_getter, set_global, set_method};
use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::rc::Rc;

pub fn init(rt: &mut JsRuntime) {
    init_array_buffer(rt);
    init_data_view(rt);
    init_typed_arrays(rt);
}

fn init_array_buffer(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.array_buffer_proto;
    let tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(tag),
        PropEntry::data(JsValue::String("ArrayBuffer".into()), false, false, true),
    );

    set_getter(
        rt,
        proto,
        "byteLength",
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("ArrayBuffer.prototype.byteLength called on non-object"));
            };
            match &rt.heap.get_obj(oid).kind {
                ObjectKind::ArrayBuffer(ab) => {
                    Ok(JsValue::Number(if ab.detached { 0.0 } else { ab.bytes.len() as f64 }))
                }
                _ => Err(JsError::type_err("Not an ArrayBuffer")),
            }
        }),
    );

    set_method(
        rt,
        proto,
        "slice",
        2,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("ArrayBuffer.prototype.slice called on non-object"));
            };
            let bytes = match &rt.heap.get_obj(oid).kind {
                ObjectKind::ArrayBuffer(ab) => {
                    if ab.detached {
                        return Err(JsError::type_err("ArrayBuffer is detached"));
                    }
                    ab.bytes.clone()
                }
                _ => return Err(JsError::type_err("Not an ArrayBuffer")),
            };
            let len = bytes.len();
            let start = resolve_rel(rt, args.first(), len, 0)?;
            let end = resolve_rel(rt, args.get(1), len, len)?;
            let slice = if start < end {
                bytes[start..end].to_vec()
            } else {
                Vec::new()
            };
            let new_ab = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.array_buffer_proto),
                ObjectKind::ArrayBuffer(ArrayBufferData {
                    bytes: slice,
                    detached: false,
                }),
            ));
            Ok(JsValue::Object(new_ab))
        }),
    );

    let ctor = rt.create_native_constructor(
        "ArrayBuffer",
        1,
        proto,
        Rc::new(|rt, _, args| {
            let n = to_integer_or_infinity(
                rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?,
            );
            if n < 0.0 || n > 128_000_000.0 || !n.is_finite() {
                return Err(JsError::range("Invalid array buffer length"));
            }
            let obj = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.array_buffer_proto),
                ObjectKind::ArrayBuffer(ArrayBufferData {
                    bytes: vec![0u8; n as usize],
                    detached: false,
                }),
            ));
            Ok(JsValue::Object(obj))
        }),
    );

    set_method(
        rt,
        ctor,
        "isView",
        1,
        Rc::new(|rt, _, args| {
            let b = matches!(
                args.first(),
                Some(JsValue::Object(oid))
                    if matches!(
                        rt.heap.get_obj(*oid).kind,
                        ObjectKind::TypedArray(_) | ObjectKind::DataView(_)
                    )
            );
            Ok(JsValue::Boolean(b))
        }),
    );

    let sym_species = rt.symbols.species;
    let sp_get = rt.create_native_function("get [Symbol.species]", 0, Rc::new(|_, this, _| Ok(this)));
    let _ = rt.define_property(
        ctor,
        PropertyKey::Symbol(sym_species),
        PropertyDescriptor {
            value: None,
            writable: None,
            get: Some(JsValue::Object(sp_get)),
            set: None,
            enumerable: Some(false),
            configurable: Some(true),
        },
    );

    set_global(rt, "ArrayBuffer", JsValue::Object(ctor));
}

fn dv_data(rt: &JsRuntime, this: &JsValue) -> Result<DataViewData, JsError> {
    let JsValue::Object(oid) = this else {
        return Err(JsError::type_err("DataView method called on non-object"));
    };
    match &rt.heap.get_obj(*oid).kind {
        ObjectKind::DataView(dv) => Ok(dv.clone()),
        _ => Err(JsError::type_err("Not a DataView")),
    }
}

fn init_data_view(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.data_view_proto;
    let tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(tag),
        PropEntry::data(JsValue::String("DataView".into()), false, false, true),
    );

    set_getter(
        rt,
        proto,
        "buffer",
        Rc::new(|rt, this, _| Ok(JsValue::Object(dv_data(rt, &this)?.buffer))),
    );
    set_getter(
        rt,
        proto,
        "byteOffset",
        Rc::new(|rt, this, _| Ok(JsValue::Number(dv_data(rt, &this)?.byte_offset as f64))),
    );
    set_getter(
        rt,
        proto,
        "byteLength",
        Rc::new(|rt, this, _| Ok(JsValue::Number(dv_data(rt, &this)?.byte_length as f64))),
    );

    macro_rules! dv_get {
        ($name:expr, $size:expr, $from_le:expr, $from_be:expr, $wrap:expr) => {
            set_method(
                rt,
                proto,
                $name,
                1,
                Rc::new(|rt, this, args| {
                    let dv = dv_data(rt, &this)?;
                    let off = to_integer_or_infinity(
                        rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?,
                    );
                    if off < 0.0 || (off as usize) + $size > dv.byte_length {
                        return Err(JsError::range("Offset is outside the bounds of the DataView"));
                    }
                    let le = args.get(1).map(|v| v.to_boolean()).unwrap_or(false);
                    let ObjectKind::ArrayBuffer(ref ab) = rt.heap.get_obj(dv.buffer).kind else {
                        return Err(JsError::type_err("Invalid buffer"));
                    };
                    let pos = dv.byte_offset + off as usize;
                    let slice: [u8; $size] = ab.bytes[pos..pos + $size].try_into().unwrap();
                    let v = if le { ($from_le)(slice) } else { ($from_be)(slice) };
                    Ok(($wrap)(v))
                }),
            );
        };
    }

    dv_get!("getInt8", 1, |b: [u8; 1]| b[0] as i8 as f64, |b: [u8; 1]| b[0] as i8 as f64, JsValue::Number);
    dv_get!("getUint8", 1, |b: [u8; 1]| b[0] as f64, |b: [u8; 1]| b[0] as f64, JsValue::Number);
    dv_get!("getInt16", 2, |b| i16::from_le_bytes(b) as f64, |b| i16::from_be_bytes(b) as f64, JsValue::Number);
    dv_get!("getUint16", 2, |b| u16::from_le_bytes(b) as f64, |b| u16::from_be_bytes(b) as f64, JsValue::Number);
    dv_get!("getInt32", 4, |b| i32::from_le_bytes(b) as f64, |b| i32::from_be_bytes(b) as f64, JsValue::Number);
    dv_get!("getUint32", 4, |b| u32::from_le_bytes(b) as f64, |b| u32::from_be_bytes(b) as f64, JsValue::Number);
    dv_get!("getFloat32", 4, |b| f32::from_le_bytes(b) as f64, |b| f32::from_be_bytes(b) as f64, JsValue::Number);
    dv_get!("getFloat64", 8, f64::from_le_bytes, f64::from_be_bytes, JsValue::Number);
    dv_get!("getBigInt64", 8, |b| i64::from_le_bytes(b) as i128, |b| i64::from_be_bytes(b) as i128, JsValue::BigInt);
    dv_get!("getBigUint64", 8, |b| u64::from_le_bytes(b) as i128, |b| u64::from_be_bytes(b) as i128, JsValue::BigInt);

    macro_rules! dv_set_num {
        ($name:expr, $size:expr, $to_le:expr, $to_be:expr) => {
            set_method(
                rt,
                proto,
                $name,
                2,
                Rc::new(|rt, this, args| {
                    let dv = dv_data(rt, &this)?;
                    let off = to_integer_or_infinity(
                        rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?,
                    );
                    let num = rt.to_number(args.get(1).unwrap_or(&JsValue::Undefined))?;
                    let le = args.get(2).map(|v| v.to_boolean()).unwrap_or(false);
                    if off < 0.0 || (off as usize) + $size > dv.byte_length {
                        return Err(JsError::range("Offset is outside the bounds of the DataView"));
                    }
                    let bytes: [u8; $size] = if le { ($to_le)(num) } else { ($to_be)(num) };
                    let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(dv.buffer).kind else {
                        return Err(JsError::type_err("Invalid buffer"));
                    };
                    let pos = dv.byte_offset + off as usize;
                    ab.bytes[pos..pos + $size].copy_from_slice(&bytes);
                    Ok(JsValue::Undefined)
                }),
            );
        };
    }

    dv_set_num!("setInt8", 1, |n| [to_int32(n) as i8 as u8], |n| [to_int32(n) as i8 as u8]);
    dv_set_num!("setUint8", 1, |n| [to_uint32(n) as u8], |n| [to_uint32(n) as u8]);
    dv_set_num!("setInt16", 2, |n| (to_int32(n) as i16).to_le_bytes(), |n| (to_int32(n) as i16).to_be_bytes());
    dv_set_num!("setUint16", 2, |n| (to_uint32(n) as u16).to_le_bytes(), |n| (to_uint32(n) as u16).to_be_bytes());
    dv_set_num!("setInt32", 4, |n| to_int32(n).to_le_bytes(), |n| to_int32(n).to_be_bytes());
    dv_set_num!("setUint32", 4, |n| to_uint32(n).to_le_bytes(), |n| to_uint32(n).to_be_bytes());
    dv_set_num!("setFloat32", 4, |n| (n as f32).to_le_bytes(), |n| (n as f32).to_be_bytes());
    dv_set_num!("setFloat64", 8, |n: f64| n.to_le_bytes(), |n: f64| n.to_be_bytes());

    set_method(
        rt,
        proto,
        "setBigInt64",
        2,
        Rc::new(|rt, this, args| {
            let dv = dv_data(rt, &this)?;
            let off = to_integer_or_infinity(rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?);
            let bi = rt.to_bigint(args.get(1).unwrap_or(&JsValue::Undefined))? as i64;
            let le = args.get(2).map(|v| v.to_boolean()).unwrap_or(false);
            if off < 0.0 || (off as usize) + 8 > dv.byte_length {
                return Err(JsError::range("Offset is outside the bounds of the DataView"));
            }
            let bytes = if le { bi.to_le_bytes() } else { bi.to_be_bytes() };
            if let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(dv.buffer).kind {
                let pos = dv.byte_offset + off as usize;
                ab.bytes[pos..pos + 8].copy_from_slice(&bytes);
            }
            Ok(JsValue::Undefined)
        }),
    );

    set_method(
        rt,
        proto,
        "setBigUint64",
        2,
        Rc::new(|rt, this, args| {
            let dv = dv_data(rt, &this)?;
            let off = to_integer_or_infinity(rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?);
            let bi = rt.to_bigint(args.get(1).unwrap_or(&JsValue::Undefined))? as u64;
            let le = args.get(2).map(|v| v.to_boolean()).unwrap_or(false);
            if off < 0.0 || (off as usize) + 8 > dv.byte_length {
                return Err(JsError::range("Offset is outside the bounds of the DataView"));
            }
            let bytes = if le { bi.to_le_bytes() } else { bi.to_be_bytes() };
            if let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(dv.buffer).kind {
                let pos = dv.byte_offset + off as usize;
                ab.bytes[pos..pos + 8].copy_from_slice(&bytes);
            }
            Ok(JsValue::Undefined)
        }),
    );

    let ctor = rt.create_native_constructor(
        "DataView",
        1,
        proto,
        Rc::new(|rt, _, args| {
            let Some(JsValue::Object(buf_id)) = args.first() else {
                return Err(JsError::type_err("First argument to DataView constructor must be an ArrayBuffer"));
            };
            let buf_len = match &rt.heap.get_obj(*buf_id).kind {
                ObjectKind::ArrayBuffer(ab) => ab.bytes.len(),
                _ => return Err(JsError::type_err("First argument to DataView constructor must be an ArrayBuffer")),
            };
            let offset = match args.get(1) {
                None | Some(JsValue::Undefined) => 0usize,
                Some(v) => {
                    let n = to_integer_or_infinity(rt.to_number(v)?);
                    if n < 0.0 || n as usize > buf_len {
                        return Err(JsError::range("Start offset is outside the bounds of the buffer"));
                    }
                    n as usize
                }
            };
            let byte_length = match args.get(2) {
                None | Some(JsValue::Undefined) => buf_len - offset,
                Some(v) => {
                    let n = to_integer_or_infinity(rt.to_number(v)?);
                    if n < 0.0 || offset + n as usize > buf_len {
                        return Err(JsError::range("Invalid DataView length"));
                    }
                    n as usize
                }
            };
            let dv = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.data_view_proto),
                ObjectKind::DataView(DataViewData {
                    buffer: *buf_id,
                    byte_offset: offset,
                    byte_length,
                }),
            ));
            Ok(JsValue::Object(dv))
        }),
    );
    set_global(rt, "DataView", JsValue::Object(ctor));
}

fn ta_data(rt: &JsRuntime, this: &JsValue) -> Result<TypedArrayData, JsError> {
    let JsValue::Object(oid) = this else {
        return Err(JsError::type_err("TypedArray method called on non-object"));
    };
    match &rt.heap.get_obj(*oid).kind {
        ObjectKind::TypedArray(ta) => Ok(ta.clone()),
        _ => Err(JsError::type_err("Not a TypedArray")),
    }
}

fn init_typed_arrays(rt: &mut JsRuntime) {
    let base_proto = rt.intrinsics.typed_array_proto;

    set_getter(
        rt,
        base_proto,
        "buffer",
        Rc::new(|rt, this, _| Ok(JsValue::Object(ta_data(rt, &this)?.buffer))),
    );
    set_getter(
        rt,
        base_proto,
        "byteLength",
        Rc::new(|rt, this, _| {
            let ta = ta_data(rt, &this)?;
            Ok(JsValue::Number((ta.length * ta.kind.element_size()) as f64))
        }),
    );
    set_getter(
        rt,
        base_proto,
        "byteOffset",
        Rc::new(|rt, this, _| Ok(JsValue::Number(ta_data(rt, &this)?.byte_offset as f64))),
    );
    set_getter(
        rt,
        base_proto,
        "length",
        Rc::new(|rt, this, _| Ok(JsValue::Number(ta_data(rt, &this)?.length as f64))),
    );

    let sym_tag = rt.symbols.to_string_tag;
    let tag_getter = rt.create_native_function(
        "get [Symbol.toStringTag]",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Ok(JsValue::Undefined);
            };
            match &rt.heap.get_obj(oid).kind {
                ObjectKind::TypedArray(ta) => Ok(JsValue::String(ta.kind.name().into())),
                _ => Ok(JsValue::Undefined),
            }
        }),
    );
    let _ = rt.define_property(
        base_proto,
        PropertyKey::Symbol(sym_tag),
        PropertyDescriptor {
            value: None,
            writable: None,
            get: Some(JsValue::Object(tag_getter)),
            set: None,
            enumerable: Some(false),
            configurable: Some(true),
        },
    );

    set_method(
        rt,
        base_proto,
        "set",
        1,
        Rc::new(|rt, this, args| {
            let ta = ta_data(rt, &this)?;
            let src = args.first().unwrap_or(&JsValue::Undefined);
            let offset = match args.get(1) {
                None | Some(JsValue::Undefined) => 0usize,
                Some(v) => {
                    let n = to_integer_or_infinity(rt.to_number(v)?);
                    if n < 0.0 {
                        return Err(JsError::range("Offset is out of bounds"));
                    }
                    n as usize
                }
            };
            let src_oid = rt.to_object(src)?;
            let len_v = rt.get_property(src_oid, "length")?;
            let src_len = to_length(rt.to_number(&len_v)?);
            if offset + src_len > ta.length {
                return Err(JsError::range("Source is too large"));
            }
            let mut vals = Vec::with_capacity(src_len);
            for i in 0..src_len {
                vals.push(rt.get_property(src_oid, &i.to_string())?);
            }
            for (i, v) in vals.iter().enumerate() {
                rt.typed_array_set(&ta, offset + i, v)?;
            }
            Ok(JsValue::Undefined)
        }),
    );

    set_method(
        rt,
        base_proto,
        "subarray",
        2,
        Rc::new(|rt, this, args| {
            let ta = ta_data(rt, &this)?;
            let begin = resolve_rel(rt, args.first(), ta.length, 0)?;
            let end = resolve_rel(rt, args.get(1), ta.length, ta.length)?;
            let new_len = end.saturating_sub(begin);
            let proto = typed_array_proto_for(rt, ta.kind);
            let obj = rt.heap.alloc_object(JsObject::new(
                Some(proto),
                ObjectKind::TypedArray(TypedArrayData {
                    kind: ta.kind,
                    buffer: ta.buffer,
                    byte_offset: ta.byte_offset + begin * ta.kind.element_size(),
                    length: new_len,
                }),
            ));
            Ok(JsValue::Object(obj))
        }),
    );

    set_method(
        rt,
        base_proto,
        "slice",
        2,
        Rc::new(|rt, this, args| {
            let ta = ta_data(rt, &this)?;
            let begin = resolve_rel(rt, args.first(), ta.length, 0)?;
            let end = resolve_rel(rt, args.get(1), ta.length, ta.length)?;
            let count = end.saturating_sub(begin);
            let new_ta = alloc_typed_array(rt, ta.kind, count)?;
            let new_data = ta_data(rt, &JsValue::Object(new_ta))?;
            for i in 0..count {
                let v = rt.typed_array_get(&ta, begin + i)?;
                rt.typed_array_set(&new_data, i, &v)?;
            }
            Ok(JsValue::Object(new_ta))
        }),
    );

    set_method(
        rt,
        base_proto,
        "fill",
        1,
        Rc::new(|rt, this, args| {
            let ta = ta_data(rt, &this)?;
            let val = args.first().cloned().unwrap_or(JsValue::Undefined);
            let start = resolve_rel(rt, args.get(1), ta.length, 0)?;
            let end = resolve_rel(rt, args.get(2), ta.length, ta.length)?;
            for i in start..end {
                rt.typed_array_set(&ta, i, &val)?;
            }
            Ok(this)
        }),
    );

    set_method(
        rt,
        base_proto,
        "map",
        1,
        Rc::new(|rt, this, args| {
            let ta = ta_data(rt, &this)?;
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let out = alloc_typed_array(rt, ta.kind, ta.length)?;
            let out_data = ta_data(rt, &JsValue::Object(out))?;
            for i in 0..ta.length {
                let v = rt.typed_array_get(&ta, i)?;
                let m = rt.call_function(
                    cb.clone(),
                    this_arg.clone(),
                    &[v, JsValue::Number(i as f64), this.clone()],
                )?;
                rt.typed_array_set(&out_data, i, &m)?;
            }
            Ok(JsValue::Object(out))
        }),
    );

    set_method(
        rt,
        base_proto,
        "forEach",
        1,
        Rc::new(|rt, this, args| {
            let ta = ta_data(rt, &this)?;
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            for i in 0..ta.length {
                let v = rt.typed_array_get(&ta, i)?;
                rt.call_function(
                    cb.clone(),
                    this_arg.clone(),
                    &[v, JsValue::Number(i as f64), this.clone()],
                )?;
            }
            Ok(JsValue::Undefined)
        }),
    );

    set_method(
        rt,
        base_proto,
        "reduce",
        1,
        Rc::new(|rt, this, args| {
            let ta = ta_data(rt, &this)?;
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            let mut idx = 0usize;
            let mut acc = if args.len() >= 2 {
                args[1].clone()
            } else if ta.length > 0 {
                idx = 1;
                rt.typed_array_get(&ta, 0)?
            } else {
                return Err(JsError::type_err(
                    "Reduce of empty array with no initial value",
                ));
            };
            while idx < ta.length {
                let v = rt.typed_array_get(&ta, idx)?;
                acc = rt.call_function(
                    cb.clone(),
                    JsValue::Undefined,
                    &[acc, v, JsValue::Number(idx as f64), this.clone()],
                )?;
                idx += 1;
            }
            Ok(acc)
        }),
    );

    set_method(
        rt,
        base_proto,
        "filter",
        1,
        Rc::new(|rt, this, args| {
            let ta = ta_data(rt, &this)?;
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let mut kept = Vec::new();
            for i in 0..ta.length {
                let v = rt.typed_array_get(&ta, i)?;
                if rt
                    .call_function(
                        cb.clone(),
                        this_arg.clone(),
                        &[v.clone(), JsValue::Number(i as f64), this.clone()],
                    )?
                    .to_boolean()
                {
                    kept.push(v);
                }
            }
            let out = alloc_typed_array(rt, ta.kind, kept.len())?;
            let out_data = ta_data(rt, &JsValue::Object(out))?;
            for (i, v) in kept.iter().enumerate() {
                rt.typed_array_set(&out_data, i, v)?;
            }
            Ok(JsValue::Object(out))
        }),
    );

    set_method(
        rt,
        base_proto,
        "join",
        1,
        Rc::new(|rt, this, args| {
            let ta = ta_data(rt, &this)?;
            let sep = match args.first() {
                None | Some(JsValue::Undefined) => ",".to_string(),
                Some(v) => rt.to_string(v)?,
            };
            let mut parts = Vec::with_capacity(ta.length);
            for i in 0..ta.length {
                let v = rt.typed_array_get(&ta, i)?;
                parts.push(rt.to_string(&v)?);
            }
            Ok(JsValue::String(parts.join(&sep)))
        }),
    );

    set_method(
        rt,
        base_proto,
        "toString",
        0,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            let join = rt.get_property(oid, "join")?;
            rt.call_function(join, this, &[])
        }),
    );

    set_method(
        rt,
        base_proto,
        "indexOf",
        1,
        Rc::new(|rt, this, args| {
            let ta = ta_data(rt, &this)?;
            let target = args.first().unwrap_or(&JsValue::Undefined);
            let from = resolve_rel(rt, args.get(1), ta.length, 0)?;
            for i in from..ta.length {
                if rt.typed_array_get(&ta, i)?.strict_eq(target) {
                    return Ok(JsValue::Number(i as f64));
                }
            }
            Ok(JsValue::Number(-1.0))
        }),
    );

    set_method(
        rt,
        base_proto,
        "includes",
        1,
        Rc::new(|rt, this, args| {
            let ta = ta_data(rt, &this)?;
            let target = args.first().unwrap_or(&JsValue::Undefined);
            let from = resolve_rel(rt, args.get(1), ta.length, 0)?;
            for i in from..ta.length {
                if rt.typed_array_get(&ta, i)?.same_value_zero(target) {
                    return Ok(JsValue::Boolean(true));
                }
            }
            Ok(JsValue::Boolean(false))
        }),
    );

    set_method(
        rt,
        base_proto,
        "at",
        1,
        Rc::new(|rt, this, args| {
            let ta = ta_data(rt, &this)?;
            let rel = to_integer_or_infinity(
                rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?,
            );
            let k = if rel >= 0.0 {
                rel
            } else {
                ta.length as f64 + rel
            };
            if k < 0.0 || k >= ta.length as f64 {
                Ok(JsValue::Undefined)
            } else {
                rt.typed_array_get(&ta, k as usize)
            }
        }),
    );

    set_method(
        rt,
        base_proto,
        "reverse",
        0,
        Rc::new(|rt, this, _| {
            let ta = ta_data(rt, &this)?;
            for i in 0..(ta.length / 2) {
                let j = ta.length - 1 - i;
                let a = rt.typed_array_get(&ta, i)?;
                let b = rt.typed_array_get(&ta, j)?;
                rt.typed_array_set(&ta, i, &b)?;
                rt.typed_array_set(&ta, j, &a)?;
            }
            Ok(this)
        }),
    );

    set_method(
        rt,
        base_proto,
        "sort",
        1,
        Rc::new(|rt, this, args| {
            let ta = ta_data(rt, &this)?;
            let cmp = args.first().cloned().unwrap_or(JsValue::Undefined);
            let mut vals = Vec::with_capacity(ta.length);
            for i in 0..ta.length {
                vals.push(rt.typed_array_get(&ta, i)?);
            }
            if cmp.is_undefined() {
                if ta.kind.is_bigint() {
                    vals.sort_by(|a, b| match (a, b) {
                        (JsValue::BigInt(x), JsValue::BigInt(y)) => x.cmp(y),
                        _ => std::cmp::Ordering::Equal,
                    });
                } else {
                    vals.sort_by(|a, b| match (a, b) {
                        (JsValue::Number(x), JsValue::Number(y)) => {
                            if x.is_nan() && y.is_nan() {
                                std::cmp::Ordering::Equal
                            } else if x.is_nan() {
                                std::cmp::Ordering::Greater
                            } else if y.is_nan() {
                                std::cmp::Ordering::Less
                            } else {
                                x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
                            }
                        }
                        _ => std::cmp::Ordering::Equal,
                    });
                }
            } else {
                crate::builtins::array::stable_merge_sort(rt, &mut vals, &cmp)?;
            }
            for (i, v) in vals.iter().enumerate() {
                rt.typed_array_set(&ta, i, v)?;
            }
            Ok(this)
        }),
    );

    let values_fn = rt.create_native_function(
        "values",
        0,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            Ok(JsValue::Object(rt.create_builtin_iterator(
                BuiltinIteratorKind::Array {
                    target: oid,
                    mode: IterMode::Values,
                },
            )))
        }),
    );
    rt.heap.get_obj_mut(base_proto).set_own_prop(
        PropertyKey::from("values"),
        PropEntry::data(JsValue::Object(values_fn), true, false, true),
    );
    let sym_iter = rt.symbols.iterator;
    rt.heap.get_obj_mut(base_proto).set_own_prop(
        PropertyKey::Symbol(sym_iter),
        PropEntry::data(JsValue::Object(values_fn), true, false, true),
    );
    set_method(
        rt,
        base_proto,
        "keys",
        0,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            Ok(JsValue::Object(rt.create_builtin_iterator(
                BuiltinIteratorKind::Array {
                    target: oid,
                    mode: IterMode::Keys,
                },
            )))
        }),
    );
    set_method(
        rt,
        base_proto,
        "entries",
        0,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            Ok(JsValue::Object(rt.create_builtin_iterator(
                BuiltinIteratorKind::Array {
                    target: oid,
                    mode: IterMode::Entries,
                },
            )))
        }),
    );

    let base_ctor = rt.create_native_constructor(
        "TypedArray",
        0,
        base_proto,
        Rc::new(|_, _, _| Err(JsError::type_err("Abstract class TypedArray not directly constructable"))),
    );
    let sym_species = rt.symbols.species;
    let sp_get = rt.create_native_function("get [Symbol.species]", 0, Rc::new(|_, this, _| Ok(this)));
    let _ = rt.define_property(
        base_ctor,
        PropertyKey::Symbol(sym_species),
        PropertyDescriptor {
            value: None,
            writable: None,
            get: Some(JsValue::Object(sp_get)),
            set: None,
            enumerable: Some(false),
            configurable: Some(true),
        },
    );
    set_method(
        rt,
        base_ctor,
        "from",
        1,
        Rc::new(|rt, this, args| {
            if !rt.is_constructor(&this) {
                return Err(JsError::type_err("%TypedArray%.from requires a constructor"));
            }
            let src = args.first().unwrap_or(&JsValue::Undefined);
            let map_fn = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let this_arg = args.get(2).cloned().unwrap_or(JsValue::Undefined);
            if !map_fn.is_undefined() && !rt.is_callable(&map_fn) {
                return Err(JsError::type_err("TypedArray.from mapfn is not a function"));
            }
            let sym_iter = PropertyKey::Symbol(rt.symbols.iterator);
            let oid = rt.to_object(src)?;
            let iter_method = rt.get_property_key(oid, &sym_iter)?;
            let values = if !iter_method.is_nullish() {
                if !rt.is_callable(&iter_method) {
                    return Err(JsError::type_err("Iterator method is not callable"));
                }
                let iter = rt.call_function(iter_method, src.clone(), &[])?;
                let mut out = Vec::new();
                while let Some(v) = rt.iterator_step(&iter)? {
                    out.push(v);
                }
                out
            } else {
                let len_v = rt.get_property(oid, "length")?;
                let len = to_length(rt.to_number(&len_v)?);
                let mut out = Vec::with_capacity(len);
                for i in 0..len {
                    out.push(rt.get_property(oid, &i.to_string())?);
                }
                out
            };
            let len = values.len();
            let target = rt.construct(this, &[JsValue::Number(len as f64)])?;
            let JsValue::Object(tid) = target else {
                return Err(JsError::type_err("TypedArray constructor did not return an object"));
            };
            if !matches!(rt.heap.get_obj(tid).kind, ObjectKind::TypedArray(_)) {
                return Err(JsError::type_err("TypedArray constructor did not return a TypedArray"));
            }
            for (i, item) in values.into_iter().enumerate() {
                let val = if !map_fn.is_undefined() {
                    rt.call_function(
                        map_fn.clone(),
                        this_arg.clone(),
                        &[item, JsValue::Number(i as f64)],
                    )?
                } else {
                    item
                };
                rt.set_property_key_strict(tid, PropertyKey::String(i.to_string()), val, true)?;
            }
            Ok(JsValue::Object(tid))
        }),
    );
    set_method(
        rt,
        base_ctor,
        "of",
        0,
        Rc::new(|rt, this, args| {
            if !rt.is_constructor(&this) {
                return Err(JsError::type_err("%TypedArray%.of requires a constructor"));
            }
            let target = rt.construct(this, &[JsValue::Number(args.len() as f64)])?;
            let JsValue::Object(tid) = target else {
                return Err(JsError::type_err("TypedArray constructor did not return an object"));
            };
            if !matches!(rt.heap.get_obj(tid).kind, ObjectKind::TypedArray(_)) {
                return Err(JsError::type_err("TypedArray constructor did not return a TypedArray"));
            }
            for (i, v) in args.iter().enumerate() {
                rt.set_property_key_strict(tid, PropertyKey::String(i.to_string()), v.clone(), true)?;
            }
            Ok(JsValue::Object(tid))
        }),
    );

    for (idx, kind) in TypedArrayKind::ALL.iter().copied().enumerate() {
        let proto = rt.intrinsics.typed_array_protos[idx];
        rt.heap.get_obj_mut(proto).set_own_prop(
            PropertyKey::from("BYTES_PER_ELEMENT"),
            PropEntry::data(JsValue::Number(kind.element_size() as f64), false, false, false),
        );
        let ctor = rt.create_native_constructor(
            kind.name(),
            3,
            proto,
            Rc::new(move |rt, _, args| construct_typed_array(rt, kind, args)),
        );
        rt.heap.get_obj_mut(ctor).prototype = Some(base_ctor);
        rt.heap.get_obj_mut(ctor).set_own_prop(
            PropertyKey::from("BYTES_PER_ELEMENT"),
            PropEntry::data(JsValue::Number(kind.element_size() as f64), false, false, false),
        );
        set_global(rt, kind.name(), JsValue::Object(ctor));
    }
}

pub fn typed_array_proto_for(rt: &JsRuntime, kind: TypedArrayKind) -> ObjectId {
    let idx = TypedArrayKind::ALL
        .iter()
        .position(|k| *k == kind)
        .unwrap_or(0);
    rt.intrinsics.typed_array_protos[idx]
}

pub fn alloc_typed_array(
    rt: &mut JsRuntime,
    kind: TypedArrayKind,
    length: usize,
) -> Result<ObjectId, JsError> {
    let byte_len = length
        .checked_mul(kind.element_size())
        .ok_or_else(|| JsError::range("TypedArray length overflow"))?;
    let buf = rt.heap.alloc_object(JsObject::new(
        Some(rt.intrinsics.array_buffer_proto),
        ObjectKind::ArrayBuffer(ArrayBufferData {
            bytes: vec![0u8; byte_len],
            detached: false,
        }),
    ));
    let proto = typed_array_proto_for(rt, kind);
    Ok(rt.heap.alloc_object(JsObject::new(
        Some(proto),
        ObjectKind::TypedArray(TypedArrayData {
            kind,
            buffer: buf,
            byte_offset: 0,
            length,
        }),
    )))
}

fn construct_typed_array(
    rt: &mut JsRuntime,
    kind: TypedArrayKind,
    args: &[JsValue],
) -> Result<JsValue, JsError> {
    let Some(first) = args.first() else {
        return Ok(JsValue::Object(alloc_typed_array(rt, kind, 0)?));
    };
    if let JsValue::Object(oid) = first {
        if let ObjectKind::ArrayBuffer(ref ab) = rt.heap.get_obj(*oid).kind {
            let buf_len = ab.bytes.len();
            let es = kind.element_size();
            let offset = match args.get(1) {
                None | Some(JsValue::Undefined) => 0usize,
                Some(v) => {
                    let n = to_integer_or_infinity(rt.to_number(v)?);
                    if n < 0.0 || (n as usize) % es != 0 || (n as usize) > buf_len {
                        return Err(JsError::range("Invalid TypedArray byteOffset"));
                    }
                    n as usize
                }
            };
            let length = match args.get(2) {
                None | Some(JsValue::Undefined) => {
                    if (buf_len - offset) % es != 0 {
                        return Err(JsError::range("ArrayBuffer byteLength not multiple of element size"));
                    }
                    (buf_len - offset) / es
                }
                Some(v) => {
                    let n = to_integer_or_infinity(rt.to_number(v)?);
                    if n < 0.0 || offset + (n as usize) * es > buf_len {
                        return Err(JsError::range("Invalid TypedArray length"));
                    }
                    n as usize
                }
            };
            let proto = typed_array_proto_for(rt, kind);
            let obj = rt.heap.alloc_object(JsObject::new(
                Some(proto),
                ObjectKind::TypedArray(TypedArrayData {
                    kind,
                    buffer: *oid,
                    byte_offset: offset,
                    length,
                }),
            ));
            return Ok(JsValue::Object(obj));
        }
        let sym_iter = PropertyKey::Symbol(rt.symbols.iterator);
        let iter_method = rt.get_property_key(*oid, &sym_iter)?;
        let items = if !iter_method.is_nullish() {
            rt.collect_iterable(first)?
        } else {
            let len_v = rt.get_property(*oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let mut out = Vec::with_capacity(len);
            for i in 0..len {
                out.push(rt.get_property(*oid, &i.to_string())?);
            }
            out
        };
        let obj = alloc_typed_array(rt, kind, items.len())?;
        let data = ta_data(rt, &JsValue::Object(obj))?;
        for (i, v) in items.iter().enumerate() {
            rt.typed_array_set(&data, i, v)?;
        }
        return Ok(JsValue::Object(obj));
    }
    let len = to_integer_or_infinity(rt.to_number(first)?);
    if len < 0.0 || !len.is_finite() {
        return Err(JsError::range("Invalid typed array length"));
    }
    Ok(JsValue::Object(alloc_typed_array(rt, kind, len as usize)?))
}

fn resolve_rel(
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
