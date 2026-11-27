use crate::builtins::{set_getter, set_global, set_method};
use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::rc::Rc;

const MAX_ALLOC_BYTES: usize = 128_000_000;

pub fn init(rt: &mut JsRuntime) {
    init_array_buffer(rt);
    init_shared_array_buffer(rt);
    init_data_view(rt);
    init_typed_arrays(rt);
}

fn to_index(rt: &mut JsRuntime, val: Option<&JsValue>) -> Result<usize, JsError> {
    match val {
        None | Some(JsValue::Undefined) => Ok(0),
        Some(v) => {
            let n = to_integer_or_infinity(rt.to_number(v)?);
            if n < 0.0 || !n.is_finite() || n > 9_007_199_254_740_991.0 {
                return Err(JsError::range("Invalid index"));
            }
            Ok(n as usize)
        }
    }
}

fn get_max_byte_length_option(
    rt: &mut JsRuntime,
    options: Option<&JsValue>,
) -> Result<Option<usize>, JsError> {
    let Some(JsValue::Object(opt_id)) = options else {
        return Ok(None);
    };
    let max_val = rt.get_property(*opt_id, "maxByteLength")?;
    if max_val.is_undefined() {
        return Ok(None);
    }
    Ok(Some(to_index(rt, Some(&max_val))?))
}

fn require_array_buffer(
    rt: &JsRuntime,
    this: &JsValue,
    want_shared: bool,
) -> Result<ObjectId, JsError> {
    let JsValue::Object(oid) = this else {
        return Err(JsError::type_err("Method called on non-object"));
    };
    if !matches!(rt.heap.get_obj(*oid).kind, ObjectKind::ArrayBuffer(_)) {
        return Err(JsError::type_err("Incompatible receiver"));
    }
    let is_shared = rt.heap.buffer_meta.get(oid).map(|m| m.is_shared).unwrap_or(false);
    if is_shared != want_shared {
        return Err(JsError::type_err("Incompatible buffer type"));
    }
    Ok(*oid)
}

fn is_buffer_detached(rt: &JsRuntime, buf_id: ObjectId) -> bool {
    matches!(
        &rt.heap.get_obj(buf_id).kind,
        ObjectKind::ArrayBuffer(ab) if ab.detached
    )
}

fn is_buffer_resizable(rt: &JsRuntime, buf_id: ObjectId) -> bool {
    rt.heap
        .buffer_meta
        .get(&buf_id)
        .and_then(|m| m.max_byte_length)
        .is_some()
}

fn is_buffer_immutable(rt: &JsRuntime, buf_id: ObjectId) -> bool {
    rt.heap
        .buffer_meta
        .get(&buf_id)
        .map(|m| m.immutable)
        .unwrap_or(false)
}

fn ab_transfer_impl(
    rt: &mut JsRuntime,
    this: JsValue,
    args: &[JsValue],
    preserve_resizability: bool,
    make_immutable: bool,
) -> Result<JsValue, JsError> {
    let oid = require_array_buffer(rt, &this, false)?;
    let new_len_opt = match args.first() {
        None | Some(JsValue::Undefined) => None,
        Some(v) => Some(to_index(rt, Some(v))?),
    };
    let (detached, old_len) = match &rt.heap.get_obj(oid).kind {
        ObjectKind::ArrayBuffer(ab) => (ab.detached, ab.bytes.len()),
        _ => return Err(JsError::type_err("Not an ArrayBuffer")),
    };
    if detached {
        return Err(JsError::type_err("Cannot transfer a detached ArrayBuffer"));
    }
    if is_buffer_immutable(rt, oid) {
        return Err(JsError::type_err("Cannot transfer an immutable ArrayBuffer"));
    }
    let new_len = new_len_opt.unwrap_or(old_len);
    let old_max = rt.heap.buffer_meta.get(&oid).and_then(|m| m.max_byte_length);
    let new_max = if preserve_resizability && !make_immutable {
        old_max
    } else {
        None
    };
    if let Some(mx) = new_max {
        if new_len > mx {
            return Err(JsError::range("New byte length exceeds maxByteLength"));
        }
    }
    if new_len > MAX_ALLOC_BYTES {
        return Err(JsError::range("ArrayBuffer allocation failed"));
    }
    let old_bytes = match &mut rt.heap.get_obj_mut(oid).kind {
        ObjectKind::ArrayBuffer(ab) => {
            let b = std::mem::take(&mut ab.bytes);
            ab.detached = true;
            b
        }
        _ => return Err(JsError::type_err("Not an ArrayBuffer")),
    };
    let mut new_bytes = vec![0u8; new_len];
    let copy_len = old_bytes.len().min(new_len);
    new_bytes[..copy_len].copy_from_slice(&old_bytes[..copy_len]);
    let new_ab = rt.heap.alloc_object(JsObject::new(
        Some(rt.intrinsics.array_buffer_proto),
        ObjectKind::ArrayBuffer(ArrayBufferData {
            bytes: new_bytes,
            detached: false,
        }),
    ));
    if new_max.is_some() || make_immutable {
        rt.heap.buffer_meta.insert(
            new_ab,
            BufferMeta {
                max_byte_length: new_max,
                is_shared: false,
                immutable: make_immutable,
            },
        );
    }
    Ok(JsValue::Object(new_ab))
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
            let oid = require_array_buffer(rt, &this, false)?;
            match &rt.heap.get_obj(oid).kind {
                ObjectKind::ArrayBuffer(ab) => {
                    Ok(JsValue::Number(if ab.detached { 0.0 } else { ab.bytes.len() as f64 }))
                }
                _ => Err(JsError::type_err("Not an ArrayBuffer")),
            }
        }),
    );

    set_getter(
        rt,
        proto,
        "maxByteLength",
        Rc::new(|rt, this, _| {
            let oid = require_array_buffer(rt, &this, false)?;
            match &rt.heap.get_obj(oid).kind {
                ObjectKind::ArrayBuffer(ab) => {
                    if ab.detached {
                        return Ok(JsValue::Number(0.0));
                    }
                    let mx = rt
                        .heap
                        .buffer_meta
                        .get(&oid)
                        .and_then(|m| m.max_byte_length)
                        .unwrap_or(ab.bytes.len());
                    Ok(JsValue::Number(mx as f64))
                }
                _ => Err(JsError::type_err("Not an ArrayBuffer")),
            }
        }),
    );

    set_getter(
        rt,
        proto,
        "resizable",
        Rc::new(|rt, this, _| {
            let oid = require_array_buffer(rt, &this, false)?;
            Ok(JsValue::Boolean(is_buffer_resizable(rt, oid)))
        }),
    );

    set_getter(
        rt,
        proto,
        "detached",
        Rc::new(|rt, this, _| {
            let oid = require_array_buffer(rt, &this, false)?;
            Ok(JsValue::Boolean(is_buffer_detached(rt, oid)))
        }),
    );

    set_getter(
        rt,
        proto,
        "immutable",
        Rc::new(|rt, this, _| {
            let oid = require_array_buffer(rt, &this, false)?;
            Ok(JsValue::Boolean(is_buffer_immutable(rt, oid)))
        }),
    );

    set_method(
        rt,
        proto,
        "resize",
        1,
        Rc::new(|rt, this, args| {
            let oid = require_array_buffer(rt, &this, false)?;
            let Some(max_len) = rt.heap.buffer_meta.get(&oid).and_then(|m| m.max_byte_length) else {
                return Err(JsError::type_err("ArrayBuffer is not resizable"));
            };
            let new_len = to_index(rt, args.first())?;
            let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(oid).kind else {
                return Err(JsError::type_err("Not an ArrayBuffer"));
            };
            if ab.detached {
                return Err(JsError::type_err("ArrayBuffer is detached"));
            }
            if new_len > max_len {
                return Err(JsError::range("New byte length exceeds maxByteLength"));
            }
            ab.bytes.resize(new_len, 0);
            Ok(JsValue::Undefined)
        }),
    );

    set_method(
        rt,
        proto,
        "transfer",
        0,
        Rc::new(|rt, this, args| ab_transfer_impl(rt, this, args, true, false)),
    );

    set_method(
        rt,
        proto,
        "transferToFixedLength",
        0,
        Rc::new(|rt, this, args| ab_transfer_impl(rt, this, args, false, false)),
    );

    set_method(
        rt,
        proto,
        "transferToImmutable",
        0,
        Rc::new(|rt, this, args| ab_transfer_impl(rt, this, args, false, true)),
    );

    set_method(
        rt,
        proto,
        "slice",
        2,
        Rc::new(|rt, this, args| {
            let oid = require_array_buffer(rt, &this, false)?;
            let len = match &rt.heap.get_obj(oid).kind {
                ObjectKind::ArrayBuffer(ab) => {
                    if ab.detached {
                        return Err(JsError::type_err("ArrayBuffer is detached"));
                    }
                    ab.bytes.len()
                }
                _ => return Err(JsError::type_err("Not an ArrayBuffer")),
            };
            let start = resolve_rel(rt, args.first(), len, 0)?;
            let end = resolve_rel(rt, args.get(1), len, len)?;
            let new_len = end.saturating_sub(start);
            let ctor = rt.species_constructor(oid, rt.intrinsics.array_buffer_ctor)?;
            let new_val = rt.construct(ctor, &[JsValue::Number(new_len as f64)])?;
            let new_id = require_array_buffer(rt, &new_val, false)?;
            if is_buffer_detached(rt, new_id) {
                return Err(JsError::type_err("Species constructor returned a detached ArrayBuffer"));
            }
            if is_buffer_immutable(rt, new_id) {
                return Err(JsError::type_err("Species constructor returned an immutable ArrayBuffer"));
            }
            if new_id == oid {
                return Err(JsError::type_err("Species constructor returned the same ArrayBuffer"));
            }
            let target_len = match &rt.heap.get_obj(new_id).kind {
                ObjectKind::ArrayBuffer(ab) => ab.bytes.len(),
                _ => return Err(JsError::type_err("Not an ArrayBuffer")),
            };
            if target_len < new_len {
                return Err(JsError::type_err("Species constructor returned an ArrayBuffer that is too small"));
            }
            if is_buffer_detached(rt, oid) {
                return Err(JsError::type_err("ArrayBuffer was detached during slice"));
            }
            let slice = match &rt.heap.get_obj(oid).kind {
                ObjectKind::ArrayBuffer(ab) => {
                    let actual_end = (start + new_len).min(ab.bytes.len());
                    if start < actual_end {
                        ab.bytes[start..actual_end].to_vec()
                    } else {
                        Vec::new()
                    }
                }
                _ => Vec::new(),
            };
            if let ObjectKind::ArrayBuffer(ref mut dst) = rt.heap.get_obj_mut(new_id).kind {
                let n = slice.len().min(dst.bytes.len());
                dst.bytes[..n].copy_from_slice(&slice[..n]);
            }
            Ok(JsValue::Object(new_id))
        }),
    );

    set_method(
        rt,
        proto,
        "sliceToImmutable",
        2,
        Rc::new(|rt, this, args| {
            let oid = require_array_buffer(rt, &this, false)?;
            let len = match &rt.heap.get_obj(oid).kind {
                ObjectKind::ArrayBuffer(ab) => {
                    if ab.detached {
                        return Err(JsError::type_err("ArrayBuffer is detached"));
                    }
                    ab.bytes.len()
                }
                _ => return Err(JsError::type_err("Not an ArrayBuffer")),
            };
            let start = resolve_rel(rt, args.first(), len, 0)?;
            let end = resolve_rel(rt, args.get(1), len, len)?;
            let new_len = end.saturating_sub(start);
            if is_buffer_detached(rt, oid) {
                return Err(JsError::type_err("ArrayBuffer was detached during sliceToImmutable"));
            }
            let cur_len = match &rt.heap.get_obj(oid).kind {
                ObjectKind::ArrayBuffer(ab) => ab.bytes.len(),
                _ => return Err(JsError::type_err("Not an ArrayBuffer")),
            };
            if cur_len < end {
                return Err(JsError::range("ArrayBuffer shrunk below slice end"));
            }
            let mut slice = vec![0u8; new_len];
            if let ObjectKind::ArrayBuffer(ref ab) = rt.heap.get_obj(oid).kind {
                if new_len > 0 {
                    slice.copy_from_slice(&ab.bytes[start..start + new_len]);
                }
            }
            let new_ab = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.array_buffer_proto),
                ObjectKind::ArrayBuffer(ArrayBufferData {
                    bytes: slice,
                    detached: false,
                }),
            ));
            rt.heap.buffer_meta.insert(
                new_ab,
                BufferMeta {
                    max_byte_length: None,
                    is_shared: false,
                    immutable: true,
                },
            );
            Ok(JsValue::Object(new_ab))
        }),
    );

    let ctor = rt.create_spec_constructor(
        "ArrayBuffer",
        1,
        proto,
        Rc::new(|rt, _, args| {
            rt.require_new_target("ArrayBuffer")?;
            let byte_len = to_index(rt, args.first())?;
            let max_byte_len = get_max_byte_length_option(rt, args.get(1))?;
            if let Some(mx) = max_byte_len {
                if byte_len > mx {
                    return Err(JsError::range("byteLength exceeds maxByteLength"));
                }
            }
            let obj_proto = rt.get_prototype_from_new_target(rt.intrinsics.array_buffer_proto)?;
            if let Some(mx) = max_byte_len {
                if mx > MAX_ALLOC_BYTES {
                    return Err(JsError::range("ArrayBuffer maxByteLength allocation failed"));
                }
            }
            if byte_len > MAX_ALLOC_BYTES {
                return Err(JsError::range("Invalid array buffer length"));
            }
            let obj = rt.heap.alloc_object(JsObject::new(
                Some(obj_proto),
                ObjectKind::ArrayBuffer(ArrayBufferData {
                    bytes: vec![0u8; byte_len],
                    detached: false,
                }),
            ));
            if max_byte_len.is_some() {
                rt.heap.buffer_meta.insert(
                    obj,
                    BufferMeta {
                        max_byte_length: max_byte_len,
                        is_shared: false,
                        immutable: false,
                    },
                );
            }
            Ok(JsValue::Object(obj))
        }),
    );
    rt.intrinsics.array_buffer_ctor = ctor;

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

fn init_shared_array_buffer(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.shared_array_buffer_proto;
    let tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(tag),
        PropEntry::data(JsValue::String("SharedArrayBuffer".into()), false, false, true),
    );

    set_getter(
        rt,
        proto,
        "byteLength",
        Rc::new(|rt, this, _| {
            let oid = require_array_buffer(rt, &this, true)?;
            match &rt.heap.get_obj(oid).kind {
                ObjectKind::ArrayBuffer(ab) => Ok(JsValue::Number(ab.bytes.len() as f64)),
                _ => Err(JsError::type_err("Not a SharedArrayBuffer")),
            }
        }),
    );

    set_getter(
        rt,
        proto,
        "maxByteLength",
        Rc::new(|rt, this, _| {
            let oid = require_array_buffer(rt, &this, true)?;
            match &rt.heap.get_obj(oid).kind {
                ObjectKind::ArrayBuffer(ab) => {
                    let mx = rt
                        .heap
                        .buffer_meta
                        .get(&oid)
                        .and_then(|m| m.max_byte_length)
                        .unwrap_or(ab.bytes.len());
                    Ok(JsValue::Number(mx as f64))
                }
                _ => Err(JsError::type_err("Not a SharedArrayBuffer")),
            }
        }),
    );

    set_getter(
        rt,
        proto,
        "growable",
        Rc::new(|rt, this, _| {
            let oid = require_array_buffer(rt, &this, true)?;
            Ok(JsValue::Boolean(is_buffer_resizable(rt, oid)))
        }),
    );

    set_method(
        rt,
        proto,
        "grow",
        1,
        Rc::new(|rt, this, args| {
            let oid = require_array_buffer(rt, &this, true)?;
            let Some(max_len) = rt.heap.buffer_meta.get(&oid).and_then(|m| m.max_byte_length) else {
                return Err(JsError::type_err("SharedArrayBuffer is not growable"));
            };
            let new_len = to_index(rt, args.first())?;
            let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(oid).kind else {
                return Err(JsError::type_err("Not a SharedArrayBuffer"));
            };
            if new_len < ab.bytes.len() || new_len > max_len {
                return Err(JsError::range("Invalid grow byteLength"));
            }
            ab.bytes.resize(new_len, 0);
            Ok(JsValue::Undefined)
        }),
    );

    set_method(
        rt,
        proto,
        "slice",
        2,
        Rc::new(|rt, this, args| {
            let oid = require_array_buffer(rt, &this, true)?;
            let len = match &rt.heap.get_obj(oid).kind {
                ObjectKind::ArrayBuffer(ab) => ab.bytes.len(),
                _ => return Err(JsError::type_err("Not a SharedArrayBuffer")),
            };
            let start = resolve_rel(rt, args.first(), len, 0)?;
            let end = resolve_rel(rt, args.get(1), len, len)?;
            let new_len = end.saturating_sub(start);
            let ctor = rt.species_constructor(oid, rt.intrinsics.shared_array_buffer_ctor)?;
            let new_val = rt.construct(ctor, &[JsValue::Number(new_len as f64)])?;
            let new_id = require_array_buffer(rt, &new_val, true)?;
            if new_id == oid {
                return Err(JsError::type_err("Species constructor returned the same SharedArrayBuffer"));
            }
            let target_len = match &rt.heap.get_obj(new_id).kind {
                ObjectKind::ArrayBuffer(ab) => ab.bytes.len(),
                _ => return Err(JsError::type_err("Not a SharedArrayBuffer")),
            };
            if target_len < new_len {
                return Err(JsError::type_err("Species constructor returned a SharedArrayBuffer that is too small"));
            }
            let slice = match &rt.heap.get_obj(oid).kind {
                ObjectKind::ArrayBuffer(ab) => {
                    let actual_end = (start + new_len).min(ab.bytes.len());
                    if start < actual_end {
                        ab.bytes[start..actual_end].to_vec()
                    } else {
                        Vec::new()
                    }
                }
                _ => Vec::new(),
            };
            if let ObjectKind::ArrayBuffer(ref mut dst) = rt.heap.get_obj_mut(new_id).kind {
                let n = slice.len().min(dst.bytes.len());
                dst.bytes[..n].copy_from_slice(&slice[..n]);
            }
            Ok(JsValue::Object(new_id))
        }),
    );

    let ctor = rt.create_spec_constructor(
        "SharedArrayBuffer",
        1,
        proto,
        Rc::new(|rt, _, args| {
            rt.require_new_target("SharedArrayBuffer")?;
            let byte_len = to_index(rt, args.first())?;
            let max_byte_len = get_max_byte_length_option(rt, args.get(1))?;
            if let Some(mx) = max_byte_len {
                if byte_len > mx {
                    return Err(JsError::range("byteLength exceeds maxByteLength"));
                }
                if mx > MAX_ALLOC_BYTES {
                    return Err(JsError::range("SharedArrayBuffer maxByteLength allocation failed"));
                }
            }
            if byte_len > MAX_ALLOC_BYTES {
                return Err(JsError::range("Invalid SharedArrayBuffer length"));
            }
            let obj_proto =
                rt.get_prototype_from_new_target(rt.intrinsics.shared_array_buffer_proto)?;
            let obj = rt.heap.alloc_object(JsObject::new(
                Some(obj_proto),
                ObjectKind::ArrayBuffer(ArrayBufferData {
                    bytes: vec![0u8; byte_len],
                    detached: false,
                }),
            ));
            rt.heap.buffer_meta.insert(
                obj,
                BufferMeta {
                    max_byte_length: max_byte_len,
                    is_shared: true,
                    immutable: false,
                },
            );
            Ok(JsValue::Object(obj))
        }),
    );
    rt.intrinsics.shared_array_buffer_ctor = ctor;

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

    set_global(rt, "SharedArrayBuffer", JsValue::Object(ctor));
}

fn f16_to_f64(bits: u16) -> f64 {
    let sign = ((bits >> 15) & 1) as u64;
    let exp = ((bits >> 10) & 0x1F) as i32;
    let mant = (bits & 0x3FF) as u64;
    if exp == 0 {
        if mant == 0 {
            return f64::from_bits(sign << 63);
        }
        let val = (mant as f64) * 2.0_f64.powi(-24);
        return if sign == 1 { -val } else { val };
    }
    if exp == 31 {
        if mant == 0 {
            return if sign == 1 {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            };
        }
        return f64::NAN;
    }
    let val = (1.0 + (mant as f64) / 1024.0) * 2.0_f64.powi(exp - 15);
    if sign == 1 { -val } else { val }
}

fn f64_to_f16(val: f64) -> u16 {
    if val.is_nan() {
        return 0x7E00;
    }
    let bits = val.to_bits();
    let sign = ((bits >> 63) as u16) << 15;
    if val.is_infinite() {
        return sign | 0x7C00;
    }
    if val == 0.0 {
        return sign;
    }
    let abs = val.abs();
    if abs >= 65520.0 {
        return sign | 0x7C00;
    }
    if abs < 2.0_f64.powi(-14) {
        let scaled = abs * 2.0_f64.powi(24);
        let rounded = round_ties_to_even(scaled) as u16;
        if rounded >= 1024 {
            return sign | 0x0400;
        }
        return sign | rounded;
    }
    let exp_f64 = (((bits >> 52) & 0x7FF) as i32) - 1023;
    let mant_f64 = bits & 0x000F_FFFF_FFFF_FFFF;
    let shift = 42;
    let half = 1u64 << (shift - 1);
    let mask = (1u64 << shift) - 1;
    let rem = mant_f64 & mask;
    let mut mant = (mant_f64 >> shift) as u16;
    if rem > half || (rem == half && (mant & 1) == 1) {
        mant += 1;
    }
    let mut exp = exp_f64 + 15;
    if mant >= 1024 {
        mant = 0;
        exp += 1;
    }
    if exp >= 31 {
        return sign | 0x7C00;
    }
    sign | ((exp as u16) << 10) | (mant & 0x3FF)
}

fn round_ties_to_even(x: f64) -> f64 {
    let f = x.floor();
    let frac = x - f;
    if frac < 0.5 {
        f
    } else if frac > 0.5 {
        f + 1.0
    } else if (f as u64) % 2 == 1 {
        f + 1.0
    } else {
        f
    }
}

fn dv_data(rt: &JsRuntime, this: &JsValue) -> Result<(ObjectId, DataViewData), JsError> {
    let JsValue::Object(oid) = this else {
        return Err(JsError::type_err("DataView method called on non-object"));
    };
    match &rt.heap.get_obj(*oid).kind {
        ObjectKind::DataView(dv) => Ok((*oid, dv.clone())),
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
        Rc::new(|rt, this, _| Ok(JsValue::Object(dv_data(rt, &this)?.1.buffer))),
    );
    set_getter(
        rt,
        proto,
        "byteOffset",
        Rc::new(|rt, this, _| {
            let (oid, dv) = dv_data(rt, &this)?;
            if rt.data_view_effective_byte_length(oid).is_none() {
                return Ok(JsValue::Number(0.0));
            }
            Ok(JsValue::Number(dv.byte_offset as f64))
        }),
    );
    set_getter(
        rt,
        proto,
        "byteLength",
        Rc::new(|rt, this, _| {
            let (oid, _) = dv_data(rt, &this)?;
            let eff = rt.data_view_effective_byte_length(oid).unwrap_or(0);
            Ok(JsValue::Number(eff as f64))
        }),
    );

    macro_rules! dv_get {
        ($name:expr, $size:expr, $from_le:expr, $from_be:expr, $wrap:expr) => {
            set_method(
                rt,
                proto,
                $name,
                1,
                Rc::new(|rt, this, args| {
                    let (oid, dv) = dv_data(rt, &this)?;
                    let off = to_index(rt, args.first())?;
                    let le = args.get(1).map(|v| v.to_boolean()).unwrap_or(false);
                    let Some(view_len) = rt.data_view_effective_byte_length(oid) else {
                        return Err(JsError::type_err("DataView buffer is detached or out of bounds"));
                    };
                    let end = off.checked_add($size).ok_or_else(|| {
                        JsError::range("Offset is outside the bounds of the DataView")
                    })?;
                    if end > view_len {
                        return Err(JsError::range("Offset is outside the bounds of the DataView"));
                    }
                    let ObjectKind::ArrayBuffer(ref ab) = rt.heap.get_obj(dv.buffer).kind else {
                        return Err(JsError::type_err("Invalid buffer"));
                    };
                    let pos = dv.byte_offset + off;
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
    dv_get!("getFloat16", 2, |b| f16_to_f64(u16::from_le_bytes(b)), |b| f16_to_f64(u16::from_be_bytes(b)), JsValue::Number);
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
                    let (oid, dv) = dv_data(rt, &this)?;
                    if is_buffer_immutable(rt, dv.buffer) {
                        return Err(JsError::type_err("ArrayBuffer is immutable"));
                    }
                    let off = to_index(rt, args.first())?;
                    let num = rt.to_number(args.get(1).unwrap_or(&JsValue::Undefined))?;
                    let le = args.get(2).map(|v| v.to_boolean()).unwrap_or(false);
                    let Some(view_len) = rt.data_view_effective_byte_length(oid) else {
                        return Err(JsError::type_err("DataView buffer is detached or out of bounds"));
                    };
                    let end = off.checked_add($size).ok_or_else(|| {
                        JsError::range("Offset is outside the bounds of the DataView")
                    })?;
                    if end > view_len {
                        return Err(JsError::range("Offset is outside the bounds of the DataView"));
                    }
                    let bytes: [u8; $size] = if le { ($to_le)(num) } else { ($to_be)(num) };
                    let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(dv.buffer).kind else {
                        return Err(JsError::type_err("Invalid buffer"));
                    };
                    let pos = dv.byte_offset + off;
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
    dv_set_num!("setFloat16", 2, |n| f64_to_f16(n).to_le_bytes(), |n| f64_to_f16(n).to_be_bytes());
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
            let (oid, dv) = dv_data(rt, &this)?;
            if is_buffer_immutable(rt, dv.buffer) {
                return Err(JsError::type_err("ArrayBuffer is immutable"));
            }
            let off = to_index(rt, args.first())?;
            let bi = rt.to_bigint(args.get(1).unwrap_or(&JsValue::Undefined))? as i64;
            let le = args.get(2).map(|v| v.to_boolean()).unwrap_or(false);
            let Some(view_len) = rt.data_view_effective_byte_length(oid) else {
                return Err(JsError::type_err("DataView buffer is detached or out of bounds"));
            };
            let end = off.checked_add(8).ok_or_else(|| {
                JsError::range("Offset is outside the bounds of the DataView")
            })?;
            if end > view_len {
                return Err(JsError::range("Offset is outside the bounds of the DataView"));
            }
            let bytes = if le { bi.to_le_bytes() } else { bi.to_be_bytes() };
            if let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(dv.buffer).kind {
                let pos = dv.byte_offset + off;
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
            let (oid, dv) = dv_data(rt, &this)?;
            if is_buffer_immutable(rt, dv.buffer) {
                return Err(JsError::type_err("ArrayBuffer is immutable"));
            }
            let off = to_index(rt, args.first())?;
            let bi = rt.to_bigint(args.get(1).unwrap_or(&JsValue::Undefined))? as u64;
            let le = args.get(2).map(|v| v.to_boolean()).unwrap_or(false);
            let Some(view_len) = rt.data_view_effective_byte_length(oid) else {
                return Err(JsError::type_err("DataView buffer is detached or out of bounds"));
            };
            let end = off.checked_add(8).ok_or_else(|| {
                JsError::range("Offset is outside the bounds of the DataView")
            })?;
            if end > view_len {
                return Err(JsError::range("Offset is outside the bounds of the DataView"));
            }
            let bytes = if le { bi.to_le_bytes() } else { bi.to_be_bytes() };
            if let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(dv.buffer).kind {
                let pos = dv.byte_offset + off;
                ab.bytes[pos..pos + 8].copy_from_slice(&bytes);
            }
            Ok(JsValue::Undefined)
        }),
    );

    let ctor = rt.create_spec_constructor(
        "DataView",
        1,
        proto,
        Rc::new(|rt, _, args| {
            rt.require_new_target("DataView")?;
            let Some(JsValue::Object(buf_id)) = args.first() else {
                return Err(JsError::type_err("First argument to DataView constructor must be an ArrayBuffer"));
            };
            if !matches!(&rt.heap.get_obj(*buf_id).kind, ObjectKind::ArrayBuffer(_)) {
                return Err(JsError::type_err("First argument to DataView constructor must be an ArrayBuffer"));
            }
            let offset = to_index(rt, args.get(1))?;
            let (detached, buf_len) = match &rt.heap.get_obj(*buf_id).kind {
                ObjectKind::ArrayBuffer(ab) => (ab.detached, ab.bytes.len()),
                _ => return Err(JsError::type_err("First argument to DataView constructor must be an ArrayBuffer")),
            };
            if detached {
                return Err(JsError::type_err("ArrayBuffer is detached"));
            }
            if offset > buf_len {
                return Err(JsError::range("Start offset is outside the bounds of the buffer"));
            }
            let (auto_len, byte_length) = match args.get(2) {
                None | Some(JsValue::Undefined) => {
                    let is_res = is_buffer_resizable(rt, *buf_id);
                    (is_res, buf_len - offset)
                }
                Some(v) => {
                    let n = to_index(rt, Some(v))?;
                    if offset.checked_add(n).map_or(true, |end| end > buf_len) {
                        return Err(JsError::range("Invalid DataView length"));
                    }
                    (false, n)
                }
            };
            let obj_proto = rt.get_prototype_from_new_target(rt.intrinsics.data_view_proto)?;
            let (detached2, buf_len2) = match &rt.heap.get_obj(*buf_id).kind {
                ObjectKind::ArrayBuffer(ab) => (ab.detached, ab.bytes.len()),
                _ => return Err(JsError::type_err("Invalid buffer")),
            };
            if detached2 {
                return Err(JsError::type_err("ArrayBuffer is detached"));
            }
            if offset > buf_len2 || (!auto_len && offset.checked_add(byte_length).map_or(true, |e| e > buf_len2)) {
                return Err(JsError::range("Offset is outside the bounds of the buffer"));
            }
            let final_len = if auto_len { buf_len2 - offset } else { byte_length };
            let dv = rt.heap.alloc_object(JsObject::new(
                Some(obj_proto),
                ObjectKind::DataView(DataViewData {
                    buffer: *buf_id,
                    byte_offset: offset,
                    byte_length: final_len,
                }),
            ));
            if auto_len {
                rt.heap.auto_length_views.insert(dv);
            }
            Ok(JsValue::Object(dv))
        }),
    );
    set_global(rt, "DataView", JsValue::Object(ctor));
}

fn ta_data(rt: &JsRuntime, this: &JsValue) -> Result<(ObjectId, TypedArrayData), JsError> {
    let JsValue::Object(oid) = this else {
        return Err(JsError::type_err("TypedArray method called on non-object"));
    };
    match &rt.heap.get_obj(*oid).kind {
        ObjectKind::TypedArray(ta) => Ok((*oid, ta.clone())),
        _ => Err(JsError::type_err("Not a TypedArray")),
    }
}

fn validate_typed_array(
    rt: &JsRuntime,
    this: &JsValue,
) -> Result<(ObjectId, TypedArrayData, usize), JsError> {
    let (oid, ta) = ta_data(rt, this)?;
    let Some(len) = rt.typed_array_effective_length(oid) else {
        return Err(JsError::type_err("TypedArray buffer is detached or out of bounds"));
    };
    Ok((oid, ta, len))
}

fn ta_species_create(
    rt: &mut JsRuntime,
    oid: ObjectId,
    kind: TypedArrayKind,
    args: &[JsValue],
) -> Result<(ObjectId, TypedArrayData), JsError> {
    let default_ctor = typed_array_ctor_for(rt, kind);
    let ctor = rt.species_constructor(oid, default_ctor)?;
    let res = rt.construct(ctor, args)?;
    let (new_oid, new_ta, new_len) = validate_typed_array(rt, &res)?;
    if args.len() == 1 {
        if let JsValue::Number(n) = args[0] {
            if (new_len as f64) < n {
                return Err(JsError::type_err("Derived TypedArray constructor created too small array"));
            }
        }
    }
    Ok((new_oid, new_ta))
}

fn init_typed_arrays(rt: &mut JsRuntime) {
    let base_proto = rt.intrinsics.typed_array_proto;

    set_getter(
        rt,
        base_proto,
        "buffer",
        Rc::new(|rt, this, _| Ok(JsValue::Object(ta_data(rt, &this)?.1.buffer))),
    );
    set_getter(
        rt,
        base_proto,
        "byteLength",
        Rc::new(|rt, this, _| {
            let (oid, ta) = ta_data(rt, &this)?;
            let eff = rt.typed_array_effective_length(oid).unwrap_or(0);
            Ok(JsValue::Number((eff * ta.kind.element_size()) as f64))
        }),
    );
    set_getter(
        rt,
        base_proto,
        "byteOffset",
        Rc::new(|rt, this, _| {
            let (oid, ta) = ta_data(rt, &this)?;
            if rt.typed_array_effective_length(oid).is_none() {
                return Ok(JsValue::Number(0.0));
            }
            Ok(JsValue::Number(ta.byte_offset as f64))
        }),
    );
    set_getter(
        rt,
        base_proto,
        "length",
        Rc::new(|rt, this, _| {
            let (oid, _) = ta_data(rt, &this)?;
            let eff = rt.typed_array_effective_length(oid).unwrap_or(0);
            Ok(JsValue::Number(eff as f64))
        }),
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
            let (oid, ta) = ta_data(rt, &this)?;
            let src = args.first().unwrap_or(&JsValue::Undefined);
            let offset = match args.get(1) {
                None | Some(JsValue::Undefined) => 0usize,
                Some(v) => {
                    let n = to_integer_or_infinity(rt.to_number(v)?);
                    if n < 0.0 {
                        return Err(JsError::range("Offset is out of bounds"));
                    }
                    if !n.is_finite() {
                        return Err(JsError::range("Offset is out of bounds"));
                    }
                    n as usize
                }
            };
            let Some(target_len) = rt.typed_array_effective_length(oid) else {
                return Err(JsError::type_err("TypedArray buffer is detached or out of bounds"));
            };
            let is_src_ta = matches!(
                src,
                JsValue::Object(sid) if matches!(rt.heap.get_obj(*sid).kind, ObjectKind::TypedArray(_))
            );
            if is_src_ta {
                let JsValue::Object(src_oid) = *src else { unreachable!() };
                let (_, src_ta, src_len) = validate_typed_array(rt, src)?;
                if src_ta.kind.is_bigint() != ta.kind.is_bigint() {
                    return Err(JsError::type_err("Cannot mix BigInt and other TypedArrays"));
                }
                if offset.checked_add(src_len).map_or(true, |e| e > target_len) {
                    return Err(JsError::range("Source is too large"));
                }
                let mut vals = Vec::with_capacity(src_len);
                for i in 0..src_len {
                    vals.push(rt.typed_array_get(&src_ta, i)?);
                }
                for (i, v) in vals.iter().enumerate() {
                    rt.typed_array_set(&ta, offset + i, v)?;
                }
                let _ = src_oid;
                return Ok(JsValue::Undefined);
            }
            let src_oid = rt.to_object(src)?;
            let len_v = rt.get_property(src_oid, "length")?;
            let src_len = to_length(rt.to_number(&len_v)?);
            if offset.checked_add(src_len).map_or(true, |e| e > target_len) {
                return Err(JsError::range("Source is too large"));
            }
            for i in 0..src_len {
                let v = rt.get_property(src_oid, &i.to_string())?;
                let converted = if ta.kind.is_bigint() {
                    JsValue::BigInt(rt.to_bigint(&v)?)
                } else {
                    JsValue::Number(rt.to_number(&v)?)
                };
                if let Some(cur_len) = rt.typed_array_effective_length(oid) {
                    if offset + i < cur_len {
                        rt.typed_array_set_raw(&ta, offset + i, &converted)?;
                    }
                }
            }
            Ok(JsValue::Undefined)
        }),
    );

    set_method(
        rt,
        base_proto,
        "copyWithin",
        2,
        Rc::new(|rt, this, args| {
            let (oid, ta, len) = validate_typed_array(rt, &this)?;
            if is_buffer_immutable(rt, ta.buffer) {
                return Err(JsError::type_err("TypedArray buffer is immutable"));
            }
            let to = resolve_rel(rt, args.first(), len, 0)?;
            let from = resolve_rel(rt, args.get(1), len, 0)?;
            let end = resolve_rel(rt, args.get(2), len, len)?;
            let count = end.saturating_sub(from).min(len.saturating_sub(to));
            if count > 0 {
                let Some(cur_len) = rt.typed_array_effective_length(oid) else {
                    return Err(JsError::type_err("TypedArray buffer is detached or out of bounds"));
                };
                let max_from = cur_len.saturating_sub(from);
                let max_to = cur_len.saturating_sub(to);
                let actual_count = count.min(max_from).min(max_to);
                if actual_count > 0 {
                    let es = ta.kind.element_size();
                    let src_start = ta.byte_offset + from * es;
                    let dst_start = ta.byte_offset + to * es;
                    let byte_count = actual_count * es;
                    if let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(ta.buffer).kind {
                        if !ab.detached
                            && src_start + byte_count <= ab.bytes.len()
                            && dst_start + byte_count <= ab.bytes.len()
                        {
                            ab.bytes.copy_within(src_start..src_start + byte_count, dst_start);
                        }
                    }
                }
            }
            Ok(this)
        }),
    );

    set_method(
        rt,
        base_proto,
        "subarray",
        2,
        Rc::new(|rt, this, args| {
            let (oid, ta) = ta_data(rt, &this)?;
            let src_len = rt.typed_array_effective_length(oid).unwrap_or(0);
            let begin = resolve_rel(rt, args.first(), src_len, 0)?;
            let end_arg = args.get(1);
            let is_auto = rt.heap.auto_length_views.contains(&oid)
                && matches!(end_arg, None | Some(JsValue::Undefined));
            let es = ta.kind.element_size();
            let begin_byte_offset = ta.byte_offset + begin * es;
            let ctor_args = if is_auto {
                vec![JsValue::Object(ta.buffer), JsValue::Number(begin_byte_offset as f64)]
            } else {
                let end = resolve_rel(rt, end_arg, src_len, src_len)?;
                let new_len = end.saturating_sub(begin);
                vec![
                    JsValue::Object(ta.buffer),
                    JsValue::Number(begin_byte_offset as f64),
                    JsValue::Number(new_len as f64),
                ]
            };
            let (new_oid, _) = ta_species_create(rt, oid, ta.kind, &ctor_args)?;
            Ok(JsValue::Object(new_oid))
        }),
    );

    set_method(
        rt,
        base_proto,
        "slice",
        2,
        Rc::new(|rt, this, args| {
            let (oid, ta, len) = validate_typed_array(rt, &this)?;
            let begin = resolve_rel(rt, args.first(), len, 0)?;
            let end = resolve_rel(rt, args.get(1), len, len)?;
            let count = end.saturating_sub(begin);
            let (new_oid, new_data) =
                ta_species_create(rt, oid, ta.kind, &[JsValue::Number(count as f64)])?;
            if count > 0 {
                let Some(cur_len) = rt.typed_array_effective_length(oid) else {
                    return Err(JsError::type_err("TypedArray buffer is detached or out of bounds"));
                };
                if is_buffer_immutable(rt, new_data.buffer) {
                    return Err(JsError::type_err("TypedArray buffer is immutable"));
                }
                if new_data.kind.is_bigint() != ta.kind.is_bigint() {
                    return Err(JsError::type_err("Cannot mix BigInt and other TypedArrays"));
                }
                let actual = count.min(cur_len.saturating_sub(begin));
                for i in 0..actual {
                    let v = rt.typed_array_get(&ta, begin + i)?;
                    rt.typed_array_set(&new_data, i, &v)?;
                }
            }
            Ok(JsValue::Object(new_oid))
        }),
    );

    set_method(
        rt,
        base_proto,
        "fill",
        1,
        Rc::new(|rt, this, args| {
            let (oid, ta, len) = validate_typed_array(rt, &this)?;
            if is_buffer_immutable(rt, ta.buffer) {
                return Err(JsError::type_err("TypedArray buffer is immutable"));
            }
            let val_arg = args.first().unwrap_or(&JsValue::Undefined);
            let converted = if ta.kind.is_bigint() {
                JsValue::BigInt(rt.to_bigint(val_arg)?)
            } else {
                JsValue::Number(rt.to_number(val_arg)?)
            };
            let start = resolve_rel(rt, args.get(1), len, 0)?;
            let end = resolve_rel(rt, args.get(2), len, len)?;
            let Some(cur_len) = rt.typed_array_effective_length(oid) else {
                return Err(JsError::type_err("TypedArray buffer is detached or out of bounds"));
            };
            let final_end = end.min(cur_len);
            for i in start..final_end {
                rt.typed_array_set_raw(&ta, i, &converted)?;
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
            let (oid, ta, len) = validate_typed_array(rt, &this)?;
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let (out, out_data) =
                ta_species_create(rt, oid, ta.kind, &[JsValue::Number(len as f64)])?;
            for i in 0..len {
                let v = rt.get_property(oid, &i.to_string())?;
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
            let (oid, _, len) = validate_typed_array(rt, &this)?;
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            for i in 0..len {
                let v = rt.get_property(oid, &i.to_string())?;
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
        "every",
        1,
        Rc::new(|rt, this, args| {
            let (oid, _, len) = validate_typed_array(rt, &this)?;
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            for i in 0..len {
                let v = rt.get_property(oid, &i.to_string())?;
                if !rt
                    .call_function(
                        cb.clone(),
                        this_arg.clone(),
                        &[v, JsValue::Number(i as f64), this.clone()],
                    )?
                    .to_boolean()
                {
                    return Ok(JsValue::Boolean(false));
                }
            }
            Ok(JsValue::Boolean(true))
        }),
    );

    set_method(
        rt,
        base_proto,
        "some",
        1,
        Rc::new(|rt, this, args| {
            let (oid, _, len) = validate_typed_array(rt, &this)?;
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            for i in 0..len {
                let v = rt.get_property(oid, &i.to_string())?;
                if rt
                    .call_function(
                        cb.clone(),
                        this_arg.clone(),
                        &[v, JsValue::Number(i as f64), this.clone()],
                    )?
                    .to_boolean()
                {
                    return Ok(JsValue::Boolean(true));
                }
            }
            Ok(JsValue::Boolean(false))
        }),
    );

    set_method(
        rt,
        base_proto,
        "find",
        1,
        Rc::new(|rt, this, args| {
            let (oid, _, len) = validate_typed_array(rt, &this)?;
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            for i in 0..len {
                let v = rt.get_property(oid, &i.to_string())?;
                if rt
                    .call_function(
                        cb.clone(),
                        this_arg.clone(),
                        &[v.clone(), JsValue::Number(i as f64), this.clone()],
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
        base_proto,
        "findIndex",
        1,
        Rc::new(|rt, this, args| {
            let (oid, _, len) = validate_typed_array(rt, &this)?;
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            for i in 0..len {
                let v = rt.get_property(oid, &i.to_string())?;
                if rt
                    .call_function(
                        cb.clone(),
                        this_arg.clone(),
                        &[v, JsValue::Number(i as f64), this.clone()],
                    )?
                    .to_boolean()
                {
                    return Ok(JsValue::Number(i as f64));
                }
            }
            Ok(JsValue::Number(-1.0))
        }),
    );

    set_method(
        rt,
        base_proto,
        "findLast",
        1,
        Rc::new(|rt, this, args| {
            let (oid, _, len) = validate_typed_array(rt, &this)?;
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            for i in (0..len).rev() {
                let v = rt.get_property(oid, &i.to_string())?;
                if rt
                    .call_function(
                        cb.clone(),
                        this_arg.clone(),
                        &[v.clone(), JsValue::Number(i as f64), this.clone()],
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
        base_proto,
        "findLastIndex",
        1,
        Rc::new(|rt, this, args| {
            let (oid, _, len) = validate_typed_array(rt, &this)?;
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            for i in (0..len).rev() {
                let v = rt.get_property(oid, &i.to_string())?;
                if rt
                    .call_function(
                        cb.clone(),
                        this_arg.clone(),
                        &[v, JsValue::Number(i as f64), this.clone()],
                    )?
                    .to_boolean()
                {
                    return Ok(JsValue::Number(i as f64));
                }
            }
            Ok(JsValue::Number(-1.0))
        }),
    );

    set_method(
        rt,
        base_proto,
        "reduce",
        1,
        Rc::new(|rt, this, args| {
            let (oid, _, len) = validate_typed_array(rt, &this)?;
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let mut idx = 0usize;
            let mut acc = if args.len() >= 2 {
                args[1].clone()
            } else if len > 0 {
                idx = 1;
                rt.get_property(oid, "0")?
            } else {
                return Err(JsError::type_err(
                    "Reduce of empty array with no initial value",
                ));
            };
            while idx < len {
                let v = rt.get_property(oid, &idx.to_string())?;
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
        "reduceRight",
        1,
        Rc::new(|rt, this, args| {
            let (oid, _, len) = validate_typed_array(rt, &this)?;
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let mut idx = len as isize - 1;
            let mut acc = if args.len() >= 2 {
                args[1].clone()
            } else if len > 0 {
                idx = len as isize - 2;
                rt.get_property(oid, &(len - 1).to_string())?
            } else {
                return Err(JsError::type_err(
                    "Reduce of empty array with no initial value",
                ));
            };
            while idx >= 0 {
                let v = rt.get_property(oid, &idx.to_string())?;
                acc = rt.call_function(
                    cb.clone(),
                    JsValue::Undefined,
                    &[acc, v, JsValue::Number(idx as f64), this.clone()],
                )?;
                idx -= 1;
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
            let (oid, ta, len) = validate_typed_array(rt, &this)?;
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let mut kept = Vec::new();
            for i in 0..len {
                let v = rt.get_property(oid, &i.to_string())?;
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
            let (out, out_data) =
                ta_species_create(rt, oid, ta.kind, &[JsValue::Number(kept.len() as f64)])?;
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
            let (oid, _, len) = validate_typed_array(rt, &this)?;
            let sep = match args.first() {
                None | Some(JsValue::Undefined) => ",".to_string(),
                Some(v) => rt.to_string(v)?,
            };
            let mut parts = Vec::with_capacity(len);
            for i in 0..len {
                let v = rt.get_property(oid, &i.to_string())?;
                if v.is_nullish() {
                    parts.push(String::new());
                } else {
                    parts.push(rt.to_string(&v)?);
                }
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
            if rt.is_callable(&join) {
                rt.call_function(join, this, &[])
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
        base_proto,
        "toLocaleString",
        0,
        Rc::new(|rt, this, _| {
            let (oid, _, len) = validate_typed_array(rt, &this)?;
            let mut parts = Vec::with_capacity(len);
            for i in 0..len {
                let v = rt.get_property(oid, &i.to_string())?;
                if v.is_nullish() {
                    parts.push(String::new());
                } else {
                    let e_oid = rt.to_object(&v)?;
                    let fn_v = rt.get_property(e_oid, "toLocaleString")?;
                    let s = rt.call_function(fn_v, v, &[])?;
                    parts.push(rt.to_string(&s)?);
                }
            }
            Ok(JsValue::String(parts.join(",")))
        }),
    );

    set_method(
        rt,
        base_proto,
        "indexOf",
        1,
        Rc::new(|rt, this, args| {
            let (oid, _, len) = validate_typed_array(rt, &this)?;
            if len == 0 {
                return Ok(JsValue::Number(-1.0));
            }
            let target = args.first().unwrap_or(&JsValue::Undefined);
            let from = match args.get(1) {
                None => 0,
                Some(a) => {
                    let n = to_integer_or_infinity(rt.to_number(a)?);
                    if n >= len as f64 {
                        return Ok(JsValue::Number(-1.0));
                    }
                    if n >= 0.0 {
                        n as usize
                    } else {
                        (len as f64 + n).max(0.0) as usize
                    }
                }
            };
            for i in from..len {
                let pk = PropertyKey::String(i.to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    if v.strict_eq(target) {
                        return Ok(JsValue::Number(i as f64));
                    }
                }
            }
            Ok(JsValue::Number(-1.0))
        }),
    );

    set_method(
        rt,
        base_proto,
        "lastIndexOf",
        1,
        Rc::new(|rt, this, args| {
            let (oid, _, len) = validate_typed_array(rt, &this)?;
            if len == 0 {
                return Ok(JsValue::Number(-1.0));
            }
            let target = args.first().unwrap_or(&JsValue::Undefined);
            let start = match args.get(1) {
                None => (len - 1) as isize,
                Some(a) => {
                    let n = to_integer_or_infinity(rt.to_number(a)?);
                    if n >= 0.0 {
                        (n as usize).min(len - 1) as isize
                    } else {
                        (len as f64 + n) as isize
                    }
                }
            };
            let mut k = start;
            while k >= 0 {
                let pk = PropertyKey::String(k.to_string());
                if rt.has_property(oid, &pk)? {
                    let v = rt.get_property_key(oid, &pk)?;
                    if v.strict_eq(target) {
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
        base_proto,
        "includes",
        1,
        Rc::new(|rt, this, args| {
            let (oid, _, len) = validate_typed_array(rt, &this)?;
            if len == 0 {
                return Ok(JsValue::Boolean(false));
            }
            let target = args.first().unwrap_or(&JsValue::Undefined);
            let from = resolve_rel(rt, args.get(1), len, 0)?;
            for i in from..len {
                let v = rt.get_property(oid, &i.to_string())?;
                if v.same_value_zero(target) {
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
            let (oid, _, len) = validate_typed_array(rt, &this)?;
            let rel = to_integer_or_infinity(
                rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?,
            );
            let k = if rel >= 0.0 {
                rel
            } else {
                len as f64 + rel
            };
            if k < 0.0 || k >= len as f64 {
                Ok(JsValue::Undefined)
            } else {
                rt.get_property(oid, &(k as usize).to_string())
            }
        }),
    );

    set_method(
        rt,
        base_proto,
        "reverse",
        0,
        Rc::new(|rt, this, _| {
            let (oid, _, len) = validate_typed_array(rt, &this)?;
            for i in 0..(len / 2) {
                let j = len - 1 - i;
                let a = rt.get_property(oid, &i.to_string())?;
                let b = rt.get_property(oid, &j.to_string())?;
                rt.set_property_key_strict(oid, PropertyKey::String(i.to_string()), b, true)?;
                rt.set_property_key_strict(oid, PropertyKey::String(j.to_string()), a, true)?;
            }
            Ok(this)
        }),
    );

    set_method(
        rt,
        base_proto,
        "toReversed",
        0,
        Rc::new(|rt, this, _| {
            let (oid, ta, len) = validate_typed_array(rt, &this)?;
            let out = alloc_typed_array(rt, ta.kind, len)?;
            let out_data = ta_data(rt, &JsValue::Object(out))?.1;
            for i in 0..len {
                let v = rt.get_property(oid, &(len - 1 - i).to_string())?;
                rt.typed_array_set(&out_data, i, &v)?;
            }
            Ok(JsValue::Object(out))
        }),
    );

    set_method(
        rt,
        base_proto,
        "sort",
        1,
        Rc::new(|rt, this, args| {
            let cmp = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !cmp.is_undefined() && !rt.is_callable(&cmp) {
                return Err(JsError::type_err("Comparison function must be a function"));
            }
            let (oid, ta, len) = validate_typed_array(rt, &this)?;
            let mut vals = Vec::with_capacity(len);
            for i in 0..len {
                vals.push(rt.typed_array_get(&ta, i)?);
            }
            sort_typed_values(rt, &mut vals, ta.kind.is_bigint(), &cmp)?;
            if let Some(cur_len) = rt.typed_array_effective_length(oid) {
                let write_len = vals.len().min(cur_len);
                for (i, v) in vals.iter().take(write_len).enumerate() {
                    rt.typed_array_set_raw(&ta, i, v)?;
                }
            }
            Ok(this)
        }),
    );

    set_method(
        rt,
        base_proto,
        "toSorted",
        1,
        Rc::new(|rt, this, args| {
            let cmp = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !cmp.is_undefined() && !rt.is_callable(&cmp) {
                return Err(JsError::type_err("Comparison function must be a function"));
            }
            let (_, ta, len) = validate_typed_array(rt, &this)?;
            let mut vals = Vec::with_capacity(len);
            for i in 0..len {
                vals.push(rt.typed_array_get(&ta, i)?);
            }
            sort_typed_values(rt, &mut vals, ta.kind.is_bigint(), &cmp)?;
            let out = alloc_typed_array(rt, ta.kind, len)?;
            let out_data = ta_data(rt, &JsValue::Object(out))?.1;
            for (i, v) in vals.iter().enumerate() {
                rt.typed_array_set_raw(&out_data, i, v)?;
            }
            Ok(JsValue::Object(out))
        }),
    );

    set_method(
        rt,
        base_proto,
        "with",
        2,
        Rc::new(|rt, this, args| {
            let (oid, ta, len) = validate_typed_array(rt, &this)?;
            let rel = to_integer_or_infinity(
                rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?,
            );
            let val_arg = args.get(1).unwrap_or(&JsValue::Undefined);
            let converted = if ta.kind.is_bigint() {
                JsValue::BigInt(rt.to_bigint(val_arg)?)
            } else {
                JsValue::Number(rt.to_number(val_arg)?)
            };
            let actual = if rel >= 0.0 { rel } else { len as f64 + rel };
            let cur_len = rt.typed_array_effective_length(oid).unwrap_or(0);
            if actual < 0.0 || actual >= len as f64 || actual >= cur_len as f64 {
                return Err(JsError::range("Invalid index in TypedArray.prototype.with"));
            }
            let idx = actual as usize;
            let out = alloc_typed_array(rt, ta.kind, len)?;
            let out_data = ta_data(rt, &JsValue::Object(out))?.1;
            for i in 0..len {
                if i == idx {
                    rt.typed_array_set_raw(&out_data, i, &converted)?;
                } else {
                    let v = rt.get_property(oid, &i.to_string())?;
                    rt.typed_array_set(&out_data, i, &v)?;
                }
            }
            Ok(JsValue::Object(out))
        }),
    );

    let values_fn = rt.create_native_function(
        "values",
        0,
        Rc::new(|rt, this, _| {
            let (oid, _, _) = validate_typed_array(rt, &this)?;
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
            let (oid, _, _) = validate_typed_array(rt, &this)?;
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
            let (oid, _, _) = validate_typed_array(rt, &this)?;
            Ok(JsValue::Object(rt.create_builtin_iterator(
                BuiltinIteratorKind::Array {
                    target: oid,
                    mode: IterMode::Entries,
                },
            )))
        }),
    );

    let base_ctor = rt.create_spec_constructor(
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
            if !iter_method.is_nullish() {
                if !rt.is_callable(&iter_method) {
                    return Err(JsError::type_err("Iterator method is not callable"));
                }
                let iter = rt.call_function(iter_method, src.clone(), &[])?;
                let mut values = Vec::new();
                while let Some(v) = rt.iterator_step(&iter)? {
                    values.push(v);
                }
                let len = values.len();
                let target = rt.construct(this, &[JsValue::Number(len as f64)])?;
                let (tid, _, target_len) = validate_typed_array(rt, &target)?;
                if target_len < len {
                    return Err(JsError::type_err("TypedArray constructor created too small array"));
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
                return Ok(JsValue::Object(tid));
            }
            let len_v = rt.get_property(oid, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            let target = rt.construct(this, &[JsValue::Number(len as f64)])?;
            let (tid, _, target_len) = validate_typed_array(rt, &target)?;
            if target_len < len {
                return Err(JsError::type_err("TypedArray constructor created too small array"));
            }
            for i in 0..len {
                let item = rt.get_property(oid, &i.to_string())?;
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
            let (tid, _, target_len) = validate_typed_array(rt, &target)?;
            if target_len < args.len() {
                return Err(JsError::type_err("TypedArray constructor created too small array"));
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
        let ctor = rt.create_spec_constructor(
            kind.name(),
            3,
            proto,
            Rc::new(move |rt, _, args| construct_typed_array(rt, kind, args)),
        );
        rt.intrinsics.typed_array_ctors[idx] = ctor;
        rt.heap.get_obj_mut(ctor).prototype = Some(base_ctor);
        rt.heap.get_obj_mut(ctor).set_own_prop(
            PropertyKey::from("BYTES_PER_ELEMENT"),
            PropEntry::data(JsValue::Number(kind.element_size() as f64), false, false, false),
        );
        if kind == TypedArrayKind::Uint8 {
            init_uint8_encoding(rt, ctor, proto);
        }
        set_global(rt, kind.name(), JsValue::Object(ctor));
    }
}

fn require_uint8_array(rt: &JsRuntime, val: &JsValue) -> Result<(ObjectId, TypedArrayData, usize), JsError> {
    let JsValue::Object(oid) = val else {
        return Err(JsError::type_err("Receiver is not a Uint8Array"));
    };
    let ta = match &rt.heap.get_obj(*oid).kind {
        ObjectKind::TypedArray(t) if t.kind == TypedArrayKind::Uint8 => t.clone(),
        _ => return Err(JsError::type_err("Receiver is not a Uint8Array")),
    };
    let Some(len) = rt.typed_array_effective_length(*oid) else {
        return Err(JsError::type_err("Uint8Array buffer is detached or out of bounds"));
    };
    Ok((*oid, ta, len))
}

fn parse_base64_options(
    rt: &mut JsRuntime,
    arg: Option<&JsValue>,
) -> Result<(bool, u8), JsError> {
    let mut is_url = false;
    let mut last_chunk: u8 = 0;
    let Some(v) = arg else {
        return Ok((is_url, last_chunk));
    };
    if v.is_undefined() {
        return Ok((is_url, last_chunk));
    }
    let JsValue::Object(opt_id) = v else {
        return Err(JsError::type_err("Options must be an object or undefined"));
    };
    let alpha = rt.get_property(*opt_id, "alphabet")?;
    if !alpha.is_undefined() {
        match alpha {
            JsValue::String(ref s) if s == "base64" => is_url = false,
            JsValue::String(ref s) if s == "base64url" => is_url = true,
            _ => return Err(JsError::type_err("Invalid base64 alphabet option")),
        }
    }
    let lch = rt.get_property(*opt_id, "lastChunkHandling")?;
    if !lch.is_undefined() {
        match lch {
            JsValue::String(ref s) if s == "loose" => last_chunk = 0,
            JsValue::String(ref s) if s == "strict" => last_chunk = 1,
            JsValue::String(ref s) if s == "stop-before-partial" => last_chunk = 2,
            _ => return Err(JsError::type_err("Invalid lastChunkHandling option")),
        }
    }
    Ok((is_url, last_chunk))
}

fn decode_b64_char(b: u8, is_url: bool) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' if !is_url => Some(62),
        b'/' if !is_url => Some(63),
        b'-' if is_url => Some(62),
        b'_' if is_url => Some(63),
        _ => None,
    }
}

fn is_b64_ws(b: u8) -> bool {
    matches!(b, 0x09 | 0x0A | 0x0C | 0x0D | 0x20)
}

fn from_base64_impl(
    s: &str,
    is_url: bool,
    last_chunk: u8,
    max_len: usize,
) -> (Vec<u8>, usize, Option<JsError>) {
    let raw = s.as_bytes();
    let len = raw.len();
    let mut out = Vec::new();
    let mut read = 0usize;
    let mut idx = 0usize;
    let mut chunk = [0u8; 4];
    let mut chunk_len = 0usize;

    if max_len == 0 {
        return (out, 0, None);
    }

    let skip_ws = |pos: &mut usize| {
        while *pos < len && is_b64_ws(raw[*pos]) {
            *pos += 1;
        }
    };

    loop {
        skip_ws(&mut idx);
        if idx == len {
            if chunk_len > 0 {
                if last_chunk == 2 {
                    return (out, read, None);
                }
                if last_chunk == 1 {
                    return (
                        out,
                        read,
                        Some(JsError::syntax("Invalid base64 padding in strict mode")),
                    );
                }
                if chunk_len == 1 {
                    return (
                        out,
                        read,
                        Some(JsError::syntax("Invalid base64 trailing single character")),
                    );
                }
                if chunk_len == 2 {
                    out.push((chunk[0] << 2) | (chunk[1] >> 4));
                } else if chunk_len == 3 {
                    out.push((chunk[0] << 2) | (chunk[1] >> 4));
                    out.push((chunk[1] << 4) | (chunk[2] >> 2));
                }
                read = len;
            } else {
                read = len;
            }
            return (out, read, None);
        }

        let b = raw[idx];
        idx += 1;
        if b == b'=' {
            if chunk_len < 2 {
                return (out, read, Some(JsError::syntax("Unexpected '=' in base64")));
            }
            skip_ws(&mut idx);
            if chunk_len == 2 {
                if idx == len {
                    if last_chunk == 2 {
                        return (out, read, None);
                    }
                    return (out, read, Some(JsError::syntax("Incomplete base64 padding")));
                }
                if raw[idx] != b'=' {
                    return (out, read, Some(JsError::syntax("Invalid base64 padding")));
                }
                idx += 1;
                skip_ws(&mut idx);
            }
            if idx < len {
                return (
                    out,
                    read,
                    Some(JsError::syntax("Unexpected trailing data after base64 padding")),
                );
            }
            if chunk_len == 2 {
                if last_chunk == 1 && (chunk[1] & 0x0F) != 0 {
                    return (
                        out,
                        read,
                        Some(JsError::syntax("Non-zero padding bits in strict base64")),
                    );
                }
                out.push((chunk[0] << 2) | (chunk[1] >> 4));
            } else {
                if last_chunk == 1 && (chunk[2] & 0x03) != 0 {
                    return (
                        out,
                        read,
                        Some(JsError::syntax("Non-zero padding bits in strict base64")),
                    );
                }
                out.push((chunk[0] << 2) | (chunk[1] >> 4));
                out.push((chunk[1] << 4) | (chunk[2] >> 2));
            }
            read = len;
            return (out, read, None);
        }

        let Some(v) = decode_b64_char(b, is_url) else {
            return (out, read, Some(JsError::syntax("Invalid character in base64")));
        };
        let remaining = max_len - out.len();
        if (remaining == 1 && chunk_len == 2) || (remaining == 2 && chunk_len == 3) {
            return (out, read, None);
        }
        chunk[chunk_len] = v;
        chunk_len += 1;
        if chunk_len == 4 {
            out.push((chunk[0] << 2) | (chunk[1] >> 4));
            out.push((chunk[1] << 4) | (chunk[2] >> 2));
            out.push((chunk[2] << 6) | chunk[3]);
            chunk_len = 0;
            read = idx;
            if out.len() == max_len {
                return (out, read, None);
            }
        }
    }
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn init_uint8_encoding(rt: &mut JsRuntime, ctor: ObjectId, proto: ObjectId) {
    set_method(
        rt,
        ctor,
        "fromBase64",
        1,
        Rc::new(|rt, _, args| {
            let Some(JsValue::String(s)) = args.first() else {
                return Err(JsError::type_err("Uint8Array.fromBase64 requires a string"));
            };
            let s = s.clone();
            let (is_url, last_chunk) = parse_base64_options(rt, args.get(1))?;
            let (bytes, _, err) = from_base64_impl(&s, is_url, last_chunk, usize::MAX);
            if let Some(e) = err {
                return Err(e);
            }
            let obj = alloc_typed_array(rt, TypedArrayKind::Uint8, bytes.len())?;
            if let ObjectKind::TypedArray(ref ta) = rt.heap.get_obj(obj).kind {
                let buf = ta.buffer;
                if let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(buf).kind {
                    ab.bytes.copy_from_slice(&bytes);
                }
            }
            Ok(JsValue::Object(obj))
        }),
    );

    set_method(
        rt,
        ctor,
        "fromHex",
        1,
        Rc::new(|rt, _, args| {
            let Some(JsValue::String(s)) = args.first() else {
                return Err(JsError::type_err("Uint8Array.fromHex requires a string"));
            };
            let raw = s.as_bytes();
            if raw.len() % 2 != 0 {
                return Err(JsError::syntax("Hex string must have an even length"));
            }
            let mut bytes = Vec::with_capacity(raw.len() / 2);
            let mut i = 0;
            while i < raw.len() {
                let Some(hi) = hex_nibble(raw[i]) else {
                    return Err(JsError::syntax("Invalid hex character"));
                };
                let Some(lo) = hex_nibble(raw[i + 1]) else {
                    return Err(JsError::syntax("Invalid hex character"));
                };
                bytes.push((hi << 4) | lo);
                i += 2;
            }
            let obj = alloc_typed_array(rt, TypedArrayKind::Uint8, bytes.len())?;
            if let ObjectKind::TypedArray(ref ta) = rt.heap.get_obj(obj).kind {
                let buf = ta.buffer;
                if let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(buf).kind {
                    ab.bytes.copy_from_slice(&bytes);
                }
            }
            Ok(JsValue::Object(obj))
        }),
    );

    set_method(
        rt,
        proto,
        "toBase64",
        0,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Receiver is not a Uint8Array"));
            };
            let ta = match &rt.heap.get_obj(oid).kind {
                ObjectKind::TypedArray(t) if t.kind == TypedArrayKind::Uint8 => t.clone(),
                _ => return Err(JsError::type_err("Receiver is not a Uint8Array")),
            };
            let mut is_url = false;
            let mut omit_padding = false;
            if let Some(v) = args.first() {
                if !v.is_undefined() {
                    let JsValue::Object(opt_id) = v else {
                        return Err(JsError::type_err("Options must be an object or undefined"));
                    };
                    let alpha = rt.get_property(*opt_id, "alphabet")?;
                    if !alpha.is_undefined() {
                        match alpha {
                            JsValue::String(ref s) if s == "base64" => is_url = false,
                            JsValue::String(ref s) if s == "base64url" => is_url = true,
                            _ => return Err(JsError::type_err("Invalid base64 alphabet option")),
                        }
                    }
                    let op = rt.get_property(*opt_id, "omitPadding")?;
                    omit_padding = op.to_boolean();
                }
            }
            let Some(len) = rt.typed_array_effective_length(oid) else {
                return Err(JsError::type_err("Uint8Array buffer is detached or out of bounds"));
            };
            let slice = match &rt.heap.get_obj(ta.buffer).kind {
                ObjectKind::ArrayBuffer(ab) => ab.bytes[ta.byte_offset..ta.byte_offset + len].to_vec(),
                _ => return Err(JsError::type_err("Invalid buffer")),
            };
            let table: &[u8; 64] = if is_url {
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
            } else {
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
            };
            let mut out = String::with_capacity((slice.len() + 2) / 3 * 4);
            let mut i = 0;
            while i + 3 <= slice.len() {
                let (b0, b1, b2) = (slice[i] as usize, slice[i + 1] as usize, slice[i + 2] as usize);
                out.push(table[b0 >> 2] as char);
                out.push(table[((b0 & 3) << 4) | (b1 >> 4)] as char);
                out.push(table[((b1 & 15) << 2) | (b2 >> 6)] as char);
                out.push(table[b2 & 63] as char);
                i += 3;
            }
            let rem = slice.len() - i;
            if rem == 1 {
                let b0 = slice[i] as usize;
                out.push(table[b0 >> 2] as char);
                out.push(table[(b0 & 3) << 4] as char);
                if !omit_padding {
                    out.push('=');
                    out.push('=');
                }
            } else if rem == 2 {
                let (b0, b1) = (slice[i] as usize, slice[i + 1] as usize);
                out.push(table[b0 >> 2] as char);
                out.push(table[((b0 & 3) << 4) | (b1 >> 4)] as char);
                out.push(table[(b1 & 15) << 2] as char);
                if !omit_padding {
                    out.push('=');
                }
            }
            Ok(JsValue::String(out))
        }),
    );

    set_method(
        rt,
        proto,
        "toHex",
        0,
        Rc::new(|rt, this, _| {
            let (_, ta, len) = require_uint8_array(rt, &this)?;
            let slice = match &rt.heap.get_obj(ta.buffer).kind {
                ObjectKind::ArrayBuffer(ab) => &ab.bytes[ta.byte_offset..ta.byte_offset + len],
                _ => return Err(JsError::type_err("Invalid buffer")),
            };
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let mut out = String::with_capacity(slice.len() * 2);
            for &b in slice {
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0x0F) as usize] as char);
            }
            Ok(JsValue::String(out))
        }),
    );

    set_method(
        rt,
        proto,
        "setFromBase64",
        1,
        Rc::new(|rt, this, args| {
            let (oid, ta, _) = require_uint8_array(rt, &this)?;
            if is_buffer_immutable(rt, ta.buffer) {
                return Err(JsError::type_err("Cannot modify an immutable ArrayBuffer"));
            }
            let Some(JsValue::String(s)) = args.first() else {
                return Err(JsError::type_err("setFromBase64 requires a string"));
            };
            let s = s.clone();
            let (is_url, last_chunk) = parse_base64_options(rt, args.get(1))?;
            let Some(len) = rt.typed_array_effective_length(oid) else {
                return Err(JsError::type_err("Uint8Array buffer is detached or out of bounds"));
            };
            let (bytes, read, err) = from_base64_impl(&s, is_url, last_chunk, len);
            let written = bytes.len();
            if written > 0 {
                if let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(ta.buffer).kind {
                    ab.bytes[ta.byte_offset..ta.byte_offset + written].copy_from_slice(&bytes);
                }
            }
            if let Some(e) = err {
                return Err(e);
            }
            let res = rt.create_object();
            let _ = rt.set_property(res, "read", JsValue::Number(read as f64));
            let _ = rt.set_property(res, "written", JsValue::Number(written as f64));
            Ok(JsValue::Object(res))
        }),
    );

    set_method(
        rt,
        proto,
        "setFromHex",
        1,
        Rc::new(|rt, this, args| {
            let (oid, ta, len) = require_uint8_array(rt, &this)?;
            let Some(JsValue::String(s)) = args.first() else {
                return Err(JsError::type_err("setFromHex requires a string"));
            };
            if is_buffer_immutable(rt, ta.buffer) {
                return Err(JsError::type_err("Cannot modify an immutable ArrayBuffer"));
            }
            let raw = s.as_bytes();
            if raw.len() % 2 != 0 {
                return Err(JsError::syntax("Hex string must have an even length"));
            }
            let Some(cur_len) = rt.typed_array_effective_length(oid) else {
                return Err(JsError::type_err("Uint8Array buffer is detached or out of bounds"));
            };
            let max_bytes = cur_len.min(len);
            let mut bytes = Vec::with_capacity((raw.len() / 2).min(max_bytes));
            let mut read = 0usize;
            let mut err: Option<JsError> = None;
            while read + 2 <= raw.len() && bytes.len() < max_bytes {
                let (Some(hi), Some(lo)) = (hex_nibble(raw[read]), hex_nibble(raw[read + 1])) else {
                    err = Some(JsError::syntax("Invalid hex character"));
                    break;
                };
                bytes.push((hi << 4) | lo);
                read += 2;
            }
            let written = bytes.len();
            if written > 0 {
                if let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(ta.buffer).kind {
                    ab.bytes[ta.byte_offset..ta.byte_offset + written].copy_from_slice(&bytes);
                }
            }
            if let Some(e) = err {
                return Err(e);
            }
            let res = rt.create_object();
            let _ = rt.set_property(res, "read", JsValue::Number(read as f64));
            let _ = rt.set_property(res, "written", JsValue::Number(written as f64));
            Ok(JsValue::Object(res))
        }),
    );
}

fn sort_typed_values(
    rt: &mut JsRuntime,
    vals: &mut [JsValue],
    is_bigint: bool,
    cmp: &JsValue,
) -> Result<(), JsError> {
    if cmp.is_undefined() {
        if is_bigint {
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
                    } else if *x == 0.0 && *y == 0.0 {
                        match (x.is_sign_negative(), y.is_sign_negative()) {
                            (true, false) => std::cmp::Ordering::Less,
                            (false, true) => std::cmp::Ordering::Greater,
                            _ => std::cmp::Ordering::Equal,
                        }
                    } else {
                        x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
                    }
                }
                _ => std::cmp::Ordering::Equal,
            });
        }
    } else {
        crate::builtins::array::stable_merge_sort(rt, vals, cmp)?;
    }
    Ok(())
}

pub fn typed_array_proto_for(rt: &JsRuntime, kind: TypedArrayKind) -> ObjectId {
    let idx = TypedArrayKind::ALL
        .iter()
        .position(|k| *k == kind)
        .unwrap_or(0);
    rt.intrinsics.typed_array_protos[idx]
}

pub fn typed_array_ctor_for(rt: &JsRuntime, kind: TypedArrayKind) -> ObjectId {
    let idx = TypedArrayKind::ALL
        .iter()
        .position(|k| *k == kind)
        .unwrap_or(0);
    rt.intrinsics.typed_array_ctors[idx]
}

pub fn alloc_typed_array(
    rt: &mut JsRuntime,
    kind: TypedArrayKind,
    length: usize,
) -> Result<ObjectId, JsError> {
    let proto = typed_array_proto_for(rt, kind);
    alloc_typed_array_with_proto(rt, kind, length, proto)
}

fn alloc_typed_array_with_proto(
    rt: &mut JsRuntime,
    kind: TypedArrayKind,
    length: usize,
    proto: ObjectId,
) -> Result<ObjectId, JsError> {
    let byte_len = length
        .checked_mul(kind.element_size())
        .ok_or_else(|| JsError::range("TypedArray length overflow"))?;
    if byte_len > MAX_ALLOC_BYTES {
        return Err(JsError::range("TypedArray allocation failed"));
    }
    let buf = rt.heap.alloc_object(JsObject::new(
        Some(rt.intrinsics.array_buffer_proto),
        ObjectKind::ArrayBuffer(ArrayBufferData {
            bytes: vec![0u8; byte_len],
            detached: false,
        }),
    ));
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
    rt.require_new_target(kind.name())?;
    let default_proto = typed_array_proto_for(rt, kind);
    let obj_proto = rt.get_prototype_from_new_target(default_proto)?;
    let Some(first) = args.first() else {
        return Ok(JsValue::Object(alloc_typed_array_with_proto(rt, kind, 0, obj_proto)?));
    };
    if let JsValue::Object(oid) = first {
        if matches!(rt.heap.get_obj(*oid).kind, ObjectKind::ArrayBuffer(_)) {
            let es = kind.element_size();
            let offset = to_index(rt, args.get(1))?;
            if offset % es != 0 {
                return Err(JsError::range("Invalid TypedArray byteOffset"));
            }
            let length_arg = args.get(2);
            let explicit_len = match length_arg {
                None | Some(JsValue::Undefined) => None,
                Some(v) => Some(to_index(rt, Some(v))?),
            };
            let (detached, buf_len) = match &rt.heap.get_obj(*oid).kind {
                ObjectKind::ArrayBuffer(ab) => (ab.detached, ab.bytes.len()),
                _ => return Err(JsError::type_err("Invalid buffer")),
            };
            if detached {
                return Err(JsError::type_err("ArrayBuffer is detached"));
            }
            let is_res = is_buffer_resizable(rt, *oid);
            let (auto_len, length) = match explicit_len {
                None => {
                    if offset > buf_len {
                        return Err(JsError::range("Start offset is outside the bounds of the buffer"));
                    }
                    if is_res {
                        (true, (buf_len - offset) / es)
                    } else {
                        if (buf_len - offset) % es != 0 {
                            return Err(JsError::range("ArrayBuffer byteLength not multiple of element size"));
                        }
                        (false, (buf_len - offset) / es)
                    }
                }
                Some(len) => {
                    let needed = len
                        .checked_mul(es)
                        .and_then(|b| offset.checked_add(b))
                        .ok_or_else(|| JsError::range("Invalid TypedArray length"))?;
                    if needed > buf_len {
                        return Err(JsError::range("Invalid TypedArray length"));
                    }
                    (false, len)
                }
            };
            let obj = rt.heap.alloc_object(JsObject::new(
                Some(obj_proto),
                ObjectKind::TypedArray(TypedArrayData {
                    kind,
                    buffer: *oid,
                    byte_offset: offset,
                    length,
                }),
            ));
            if auto_len {
                rt.heap.auto_length_views.insert(obj);
            }
            return Ok(JsValue::Object(obj));
        }
        if matches!(rt.heap.get_obj(*oid).kind, ObjectKind::TypedArray(_)) {
            let (_, src_ta, src_len) = validate_typed_array(rt, first)?;
            if src_ta.kind.is_bigint() != kind.is_bigint() {
                return Err(JsError::type_err("Cannot mix BigInt and other TypedArrays"));
            }
            let obj = alloc_typed_array_with_proto(rt, kind, src_len, obj_proto)?;
            let data = ta_data(rt, &JsValue::Object(obj))?.1;
            for i in 0..src_len {
                let v = rt.typed_array_get(&src_ta, i)?;
                rt.typed_array_set(&data, i, &v)?;
            }
            return Ok(JsValue::Object(obj));
        }
        let sym_iter = PropertyKey::Symbol(rt.symbols.iterator);
        let iter_method = rt.get_property_key(*oid, &sym_iter)?;
        if !iter_method.is_nullish() {
            let items = rt.collect_iterable(first)?;
            let obj = alloc_typed_array_with_proto(rt, kind, items.len(), obj_proto)?;
            let data = ta_data(rt, &JsValue::Object(obj))?.1;
            for (i, v) in items.iter().enumerate() {
                rt.typed_array_set(&data, i, v)?;
            }
            return Ok(JsValue::Object(obj));
        }
        let len_v = rt.get_property(*oid, "length")?;
        let len = to_length(rt.to_number(&len_v)?);
        let obj = alloc_typed_array_with_proto(rt, kind, len, obj_proto)?;
        let data = ta_data(rt, &JsValue::Object(obj))?.1;
        for i in 0..len {
            let v = rt.get_property(*oid, &i.to_string())?;
            rt.typed_array_set(&data, i, &v)?;
        }
        return Ok(JsValue::Object(obj));
    }
    let len = to_index(rt, Some(first))?;
    Ok(JsValue::Object(alloc_typed_array_with_proto(rt, kind, len, obj_proto)?))
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
            if n == f64::NEG_INFINITY {
                Ok(0)
            } else if n < 0.0 {
                Ok((len as f64 + n).max(0.0) as usize)
            } else if n >= len as f64 {
                Ok(len)
            } else {
                Ok(n as usize)
            }
        }
    }
}
