use crate::builtins::{set_global, set_method};
use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::rc::Rc;

pub fn init(rt: &mut JsRuntime) {
    let atomics = rt.create_object();
    let tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(atomics).set_own_prop(
        PropertyKey::Symbol(tag),
        PropEntry::data(JsValue::String("Atomics".into()), false, false, true),
    );

    set_method(
        rt,
        atomics,
        "add",
        3,
        Rc::new(|rt, _, args| atomic_rmw(rt, args, RmwOp::Add)),
    );
    set_method(
        rt,
        atomics,
        "sub",
        3,
        Rc::new(|rt, _, args| atomic_rmw(rt, args, RmwOp::Sub)),
    );
    set_method(
        rt,
        atomics,
        "and",
        3,
        Rc::new(|rt, _, args| atomic_rmw(rt, args, RmwOp::And)),
    );
    set_method(
        rt,
        atomics,
        "or",
        3,
        Rc::new(|rt, _, args| atomic_rmw(rt, args, RmwOp::Or)),
    );
    set_method(
        rt,
        atomics,
        "xor",
        3,
        Rc::new(|rt, _, args| atomic_rmw(rt, args, RmwOp::Xor)),
    );
    set_method(
        rt,
        atomics,
        "exchange",
        3,
        Rc::new(|rt, _, args| atomic_rmw(rt, args, RmwOp::Exchange)),
    );
    set_method(
        rt,
        atomics,
        "compareExchange",
        4,
        Rc::new(|rt, _, args| atomic_compare_exchange(rt, args)),
    );
    set_method(
        rt,
        atomics,
        "load",
        2,
        Rc::new(|rt, _, args| atomic_load(rt, args)),
    );
    set_method(
        rt,
        atomics,
        "store",
        3,
        Rc::new(|rt, _, args| atomic_store(rt, args)),
    );
    set_method(
        rt,
        atomics,
        "isLockFree",
        1,
        Rc::new(|rt, _, args| {
            let n = to_integer_or_infinity(
                rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?,
            );
            let res = n == 1.0 || n == 2.0 || n == 4.0 || n == 8.0;
            Ok(JsValue::Boolean(res))
        }),
    );
    set_method(
        rt,
        atomics,
        "notify",
        3,
        Rc::new(|rt, _, args| atomic_notify(rt, args)),
    );
    set_method(
        rt,
        atomics,
        "wait",
        4,
        Rc::new(|rt, _, args| atomic_wait(rt, args, false)),
    );
    set_method(
        rt,
        atomics,
        "waitAsync",
        4,
        Rc::new(|rt, _, args| atomic_wait(rt, args, true)),
    );
    set_method(
        rt,
        atomics,
        "pause",
        0,
        Rc::new(|_, _, args| {
            if let Some(arg) = args.first() {
                if !arg.is_undefined() {
                    match arg {
                        JsValue::Number(n) if n.is_finite() && n.fract() == 0.0 => {}
                        _ => {
                            return Err(JsError::type_err(
                                "Atomics.pause argument must be an integral number or undefined",
                            ));
                        }
                    }
                }
            }
            Ok(JsValue::Undefined)
        }),
    );

    set_global(rt, "Atomics", JsValue::Object(atomics));
}

#[derive(Clone, Copy)]
enum RmwOp {
    Add,
    Sub,
    And,
    Or,
    Xor,
    Exchange,
}

fn validate_integer_typed_array(
    rt: &JsRuntime,
    val: &JsValue,
    waitable: bool,
) -> Result<(ObjectId, TypedArrayData), JsError> {
    let JsValue::Object(oid) = val else {
        return Err(JsError::type_err("Atomics argument is not a TypedArray"));
    };
    let ta = match &rt.heap.get_obj(*oid).kind {
        ObjectKind::TypedArray(t) => t.clone(),
        _ => return Err(JsError::type_err("Atomics argument is not a TypedArray")),
    };
    if waitable {
        if !matches!(ta.kind, TypedArrayKind::Int32 | TypedArrayKind::BigInt64) {
            return Err(JsError::type_err(
                "Atomics waitable operation requires Int32Array or BigInt64Array",
            ));
        }
    } else if matches!(
        ta.kind,
        TypedArrayKind::Uint8Clamped | TypedArrayKind::Float32 | TypedArrayKind::Float64
    ) {
        return Err(JsError::type_err(
            "Atomics operation requires an integer TypedArray",
        ));
    }
    if rt.typed_array_effective_length(*oid).is_none() {
        return Err(JsError::type_err("TypedArray buffer is detached or out of bounds"));
    }
    Ok((*oid, ta))
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

fn validate_atomic_access(
    rt: &mut JsRuntime,
    oid: ObjectId,
    ta: &TypedArrayData,
    index_arg: Option<&JsValue>,
) -> Result<(usize, usize), JsError> {
    let Some(len) = rt.typed_array_effective_length(oid) else {
        return Err(JsError::type_err("TypedArray buffer is detached or out of bounds"));
    };
    let idx = to_index(rt, index_arg)?;
    if idx >= len {
        return Err(JsError::range("Atomics index out of range"));
    }
    let byte_index = ta.byte_offset + idx * ta.kind.element_size();
    Ok((idx, byte_index))
}

fn check_still_in_bounds(
    rt: &JsRuntime,
    oid: ObjectId,
    idx: usize,
) -> Result<(), JsError> {
    let Some(len) = rt.typed_array_effective_length(oid) else {
        return Err(JsError::type_err("TypedArray buffer is detached or out of bounds"));
    };
    if idx >= len {
        return Err(JsError::type_err("TypedArray became out of bounds"));
    }
    Ok(())
}

fn read_int_at(bytes: &[u8], offset: usize, kind: TypedArrayKind) -> i128 {
    match kind {
        TypedArrayKind::Int8 => bytes[offset] as i8 as i128,
        TypedArrayKind::Uint8 => bytes[offset] as i128,
        TypedArrayKind::Int16 => {
            i16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as i128
        }
        TypedArrayKind::Uint16 => {
            u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as i128
        }
        TypedArrayKind::Int32 => i32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as i128,
        TypedArrayKind::Uint32 => u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as i128,
        TypedArrayKind::BigInt64 => i64::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as i128,
        TypedArrayKind::BigUint64 => u64::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as i128,
        _ => 0,
    }
}

fn write_int_at(bytes: &mut [u8], offset: usize, kind: TypedArrayKind, val: i128) {
    match kind {
        TypedArrayKind::Int8 => bytes[offset] = (val as i8) as u8,
        TypedArrayKind::Uint8 => bytes[offset] = val as u8,
        TypedArrayKind::Int16 => {
            let b = (val as i16).to_le_bytes();
            bytes[offset..offset + 2].copy_from_slice(&b);
        }
        TypedArrayKind::Uint16 => {
            let b = (val as u16).to_le_bytes();
            bytes[offset..offset + 2].copy_from_slice(&b);
        }
        TypedArrayKind::Int32 => {
            let b = (val as i32).to_le_bytes();
            bytes[offset..offset + 4].copy_from_slice(&b);
        }
        TypedArrayKind::Uint32 => {
            let b = (val as u32).to_le_bytes();
            bytes[offset..offset + 4].copy_from_slice(&b);
        }
        TypedArrayKind::BigInt64 => {
            let b = (val as i64).to_le_bytes();
            bytes[offset..offset + 8].copy_from_slice(&b);
        }
        TypedArrayKind::BigUint64 => {
            let b = (val as u64).to_le_bytes();
            bytes[offset..offset + 8].copy_from_slice(&b);
        }
        _ => {}
    }
}

fn int_to_js(val: i128, kind: TypedArrayKind) -> JsValue {
    if kind.is_bigint() {
        JsValue::BigInt(crate::bigint::JsBigInt::from_i128(val))
    } else {
        JsValue::Number(val as f64)
    }
}

fn to_atomic_int(
    rt: &mut JsRuntime,
    val: &JsValue,
    kind: TypedArrayKind,
) -> Result<(i128, JsValue), JsError> {
    if kind.is_bigint() {
        let bi = rt.to_bigint(val)?;
        let raw = bi.to_i128_wrapping();
        Ok((raw, JsValue::BigInt(bi)))
    } else {
        let n = to_integer_or_infinity(rt.to_number(val)?);
        let norm = if n == 0.0 { 0.0 } else { n };
        let i = if n.is_finite() {
            to_int32(n) as i128
        } else {
            0
        };
        Ok((i, JsValue::Number(norm)))
    }
}

fn apply_rmw(op: RmwOp, old: i128, arg: i128, kind: TypedArrayKind) -> i128 {
    match kind {
        TypedArrayKind::Int8 => {
            let (a, b) = (old as i8, arg as i8);
            (match op {
                RmwOp::Add => a.wrapping_add(b),
                RmwOp::Sub => a.wrapping_sub(b),
                RmwOp::And => a & b,
                RmwOp::Or => a | b,
                RmwOp::Xor => a ^ b,
                RmwOp::Exchange => b,
            }) as i128
        }
        TypedArrayKind::Uint8 => {
            let (a, b) = (old as u8, arg as u8);
            (match op {
                RmwOp::Add => a.wrapping_add(b),
                RmwOp::Sub => a.wrapping_sub(b),
                RmwOp::And => a & b,
                RmwOp::Or => a | b,
                RmwOp::Xor => a ^ b,
                RmwOp::Exchange => b,
            }) as i128
        }
        TypedArrayKind::Int16 => {
            let (a, b) = (old as i16, arg as i16);
            (match op {
                RmwOp::Add => a.wrapping_add(b),
                RmwOp::Sub => a.wrapping_sub(b),
                RmwOp::And => a & b,
                RmwOp::Or => a | b,
                RmwOp::Xor => a ^ b,
                RmwOp::Exchange => b,
            }) as i128
        }
        TypedArrayKind::Uint16 => {
            let (a, b) = (old as u16, arg as u16);
            (match op {
                RmwOp::Add => a.wrapping_add(b),
                RmwOp::Sub => a.wrapping_sub(b),
                RmwOp::And => a & b,
                RmwOp::Or => a | b,
                RmwOp::Xor => a ^ b,
                RmwOp::Exchange => b,
            }) as i128
        }
        TypedArrayKind::Int32 => {
            let (a, b) = (old as i32, arg as i32);
            (match op {
                RmwOp::Add => a.wrapping_add(b),
                RmwOp::Sub => a.wrapping_sub(b),
                RmwOp::And => a & b,
                RmwOp::Or => a | b,
                RmwOp::Xor => a ^ b,
                RmwOp::Exchange => b,
            }) as i128
        }
        TypedArrayKind::Uint32 => {
            let (a, b) = (old as u32, arg as u32);
            (match op {
                RmwOp::Add => a.wrapping_add(b),
                RmwOp::Sub => a.wrapping_sub(b),
                RmwOp::And => a & b,
                RmwOp::Or => a | b,
                RmwOp::Xor => a ^ b,
                RmwOp::Exchange => b,
            }) as i128
        }
        TypedArrayKind::BigInt64 => {
            let (a, b) = (old as i64, arg as i64);
            (match op {
                RmwOp::Add => a.wrapping_add(b),
                RmwOp::Sub => a.wrapping_sub(b),
                RmwOp::And => a & b,
                RmwOp::Or => a | b,
                RmwOp::Xor => a ^ b,
                RmwOp::Exchange => b,
            }) as i128
        }
        TypedArrayKind::BigUint64 => {
            let (a, b) = (old as u64, arg as u64);
            (match op {
                RmwOp::Add => a.wrapping_add(b),
                RmwOp::Sub => a.wrapping_sub(b),
                RmwOp::And => a & b,
                RmwOp::Or => a | b,
                RmwOp::Xor => a ^ b,
                RmwOp::Exchange => b,
            }) as i128
        }
        _ => 0,
    }
}

fn truncate_to_kind(val: i128, kind: TypedArrayKind) -> i128 {
    match kind {
        TypedArrayKind::Int8 => (val as i8) as i128,
        TypedArrayKind::Uint8 => (val as u8) as i128,
        TypedArrayKind::Int16 => (val as i16) as i128,
        TypedArrayKind::Uint16 => (val as u16) as i128,
        TypedArrayKind::Int32 => (val as i32) as i128,
        TypedArrayKind::Uint32 => (val as u32) as i128,
        TypedArrayKind::BigInt64 => (val as i64) as i128,
        TypedArrayKind::BigUint64 => (val as u64) as i128,
        _ => 0,
    }
}

fn check_buffer_mutable(rt: &JsRuntime, buf_id: ObjectId) -> Result<(), JsError> {
    if rt
        .heap
        .buffer_meta
        .get(&buf_id)
        .map(|m| m.immutable)
        .unwrap_or(false)
    {
        return Err(JsError::type_err(
            "Cannot perform atomic write on an immutable ArrayBuffer",
        ));
    }
    Ok(())
}

fn atomic_rmw(rt: &mut JsRuntime, args: &[JsValue], op: RmwOp) -> Result<JsValue, JsError> {
    let (oid, ta) = validate_integer_typed_array(rt, args.first().unwrap_or(&JsValue::Undefined), false)?;
    check_buffer_mutable(rt, ta.buffer)?;
    let (idx, byte_idx) = validate_atomic_access(rt, oid, &ta, args.get(1))?;
    let (arg_int, _) = to_atomic_int(rt, args.get(2).unwrap_or(&JsValue::Undefined), ta.kind)?;
    check_still_in_bounds(rt, oid, idx)?;
    if let Some(sb) = rt.shared_buffer_backing(ta.buffer) {
        let mut guard = sb.data.lock().unwrap();
        let old = read_int_at(&guard, byte_idx, ta.kind);
        let new = apply_rmw(op, old, arg_int, ta.kind);
        write_int_at(&mut guard, byte_idx, ta.kind, new);
        return Ok(int_to_js(old, ta.kind));
    }
    let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(ta.buffer).kind else {
        return Err(JsError::type_err("Invalid buffer"));
    };
    let old = read_int_at(&ab.bytes, byte_idx, ta.kind);
    let new = apply_rmw(op, old, arg_int, ta.kind);
    write_int_at(&mut ab.bytes, byte_idx, ta.kind, new);
    Ok(int_to_js(old, ta.kind))
}

fn atomic_compare_exchange(rt: &mut JsRuntime, args: &[JsValue]) -> Result<JsValue, JsError> {
    let (oid, ta) = validate_integer_typed_array(rt, args.first().unwrap_or(&JsValue::Undefined), false)?;
    check_buffer_mutable(rt, ta.buffer)?;
    let (idx, byte_idx) = validate_atomic_access(rt, oid, &ta, args.get(1))?;
    let (exp_int, _) = to_atomic_int(rt, args.get(2).unwrap_or(&JsValue::Undefined), ta.kind)?;
    let (rep_int, _) = to_atomic_int(rt, args.get(3).unwrap_or(&JsValue::Undefined), ta.kind)?;
    check_still_in_bounds(rt, oid, idx)?;
    let exp_trunc = truncate_to_kind(exp_int, ta.kind);
    if let Some(sb) = rt.shared_buffer_backing(ta.buffer) {
        let mut guard = sb.data.lock().unwrap();
        let old = read_int_at(&guard, byte_idx, ta.kind);
        if old == exp_trunc {
            write_int_at(&mut guard, byte_idx, ta.kind, rep_int);
        }
        return Ok(int_to_js(old, ta.kind));
    }
    let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(ta.buffer).kind else {
        return Err(JsError::type_err("Invalid buffer"));
    };
    let old = read_int_at(&ab.bytes, byte_idx, ta.kind);
    if old == exp_trunc {
        write_int_at(&mut ab.bytes, byte_idx, ta.kind, rep_int);
    }
    Ok(int_to_js(old, ta.kind))
}

fn atomic_load(rt: &mut JsRuntime, args: &[JsValue]) -> Result<JsValue, JsError> {
    let (oid, ta) = validate_integer_typed_array(rt, args.first().unwrap_or(&JsValue::Undefined), false)?;
    let (idx, byte_idx) = validate_atomic_access(rt, oid, &ta, args.get(1))?;
    check_still_in_bounds(rt, oid, idx)?;
    if let Some(sb) = rt.shared_buffer_backing(ta.buffer) {
        let guard = sb.data.lock().unwrap();
        let val = read_int_at(&guard, byte_idx, ta.kind);
        return Ok(int_to_js(val, ta.kind));
    }
    let ObjectKind::ArrayBuffer(ref ab) = rt.heap.get_obj(ta.buffer).kind else {
        return Err(JsError::type_err("Invalid buffer"));
    };
    let val = read_int_at(&ab.bytes, byte_idx, ta.kind);
    Ok(int_to_js(val, ta.kind))
}

fn atomic_store(rt: &mut JsRuntime, args: &[JsValue]) -> Result<JsValue, JsError> {
    let (oid, ta) = validate_integer_typed_array(rt, args.first().unwrap_or(&JsValue::Undefined), false)?;
    check_buffer_mutable(rt, ta.buffer)?;
    let (idx, byte_idx) = validate_atomic_access(rt, oid, &ta, args.get(1))?;
    let (arg_int, ret_val) = to_atomic_int(rt, args.get(2).unwrap_or(&JsValue::Undefined), ta.kind)?;
    check_still_in_bounds(rt, oid, idx)?;
    if let Some(sb) = rt.shared_buffer_backing(ta.buffer) {
        let mut guard = sb.data.lock().unwrap();
        write_int_at(&mut guard, byte_idx, ta.kind, arg_int);
        return Ok(ret_val);
    }
    let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(ta.buffer).kind else {
        return Err(JsError::type_err("Invalid buffer"));
    };
    write_int_at(&mut ab.bytes, byte_idx, ta.kind, arg_int);
    Ok(ret_val)
}

fn atomic_notify(rt: &mut JsRuntime, args: &[JsValue]) -> Result<JsValue, JsError> {
    let (oid, ta) = validate_integer_typed_array(rt, args.first().unwrap_or(&JsValue::Undefined), true)?;
    let (_idx, byte_idx) = validate_atomic_access(rt, oid, &ta, args.get(1))?;
    let count = match args.get(2) {
        None | Some(JsValue::Undefined) => usize::MAX,
        Some(c) => {
            let n = to_integer_or_infinity(rt.to_number(c)?).max(0.0);
            if n.is_infinite() {
                usize::MAX
            } else {
                n as usize
            }
        }
    };
    let Some(sb) = rt.shared_buffer_backing(ta.buffer) else {
        return Ok(JsValue::Number(0.0));
    };
    if count == 0 {
        return Ok(JsValue::Number(0.0));
    }
    let _data_guard = sb.data.lock().unwrap();
    let mut waiters_map = sb.waiters.lock().unwrap();
    let mut woken = 0usize;
    if let Some(q) = waiters_map.get_mut(&byte_idx) {
        while woken < count {
            let Some(entry) = q.pop_front() else {
                break;
            };
            let mut st = entry.state.lock().unwrap();
            if *st == WaiterState::Waiting {
                *st = WaiterState::Notified;
                entry.cv.notify_one();
                woken += 1;
            }
        }
    }
    Ok(JsValue::Number(woken as f64))
}

fn atomic_wait(rt: &mut JsRuntime, args: &[JsValue], is_async: bool) -> Result<JsValue, JsError> {
    let (oid, ta) = validate_integer_typed_array(rt, args.first().unwrap_or(&JsValue::Undefined), true)?;
    let Some(sb) = rt.shared_buffer_backing(ta.buffer) else {
        return Err(JsError::type_err(
            "Atomics.wait requires a SharedArrayBuffer",
        ));
    };
    let (_idx, byte_idx) = validate_atomic_access(rt, oid, &ta, args.get(1))?;
    let val_arg = args.get(2).unwrap_or(&JsValue::Undefined);
    let expected = if ta.kind == TypedArrayKind::BigInt64 {
        rt.to_bigint(val_arg)?.to_i64_wrapping() as i128
    } else {
        to_int32(rt.to_number(val_arg)?) as i128
    };
    let q = match args.get(3) {
        None | Some(JsValue::Undefined) => f64::INFINITY,
        Some(v) => {
            let n = rt.to_number(v)?;
            if n.is_nan() {
                f64::INFINITY
            } else {
                n.max(0.0)
            }
        }
    };
    if !is_async && !rt.can_block {
        return Err(JsError::type_err(
            "Atomics.wait cannot be called in this context",
        ));
    }
    let data_guard = sb.data.lock().unwrap();
    if byte_idx + ta.kind.element_size() > data_guard.len() {
        return Err(JsError::range("Atomics index out of range"));
    }
    let cur = read_int_at(&data_guard, byte_idx, ta.kind);
    if !is_async {
        if cur != expected {
            return Ok(JsValue::String("not-equal".into()));
        }
        if q == 0.0 {
            return Ok(JsValue::String("timed-out".into()));
        }
        let entry = std::sync::Arc::new(WaiterEntry {
            state: std::sync::Mutex::new(WaiterState::Waiting),
            cv: std::sync::Condvar::new(),
        });
        {
            let mut map = sb.waiters.lock().unwrap();
            map.entry(byte_idx).or_default().push_back(entry.clone());
        }
        drop(data_guard);
        let mut st = entry.state.lock().unwrap();
        if q.is_infinite() {
            while *st == WaiterState::Waiting {
                st = entry.cv.wait(st).unwrap();
            }
        } else {
            let dur = std::time::Duration::from_secs_f64((q / 1000.0).min(86400.0));
            let deadline = std::time::Instant::now() + dur;
            while *st == WaiterState::Waiting {
                let now = std::time::Instant::now();
                if now >= deadline {
                    break;
                }
                let (next_st, timeout_res) = entry.cv.wait_timeout(st, deadline - now).unwrap();
                st = next_st;
                if timeout_res.timed_out() {
                    break;
                }
            }
        }
        if *st == WaiterState::Notified {
            return Ok(JsValue::String("ok".into()));
        }
        *st = WaiterState::TimedOut;
        drop(st);
        let mut map = sb.waiters.lock().unwrap();
        if let Some(queue) = map.get_mut(&byte_idx) {
            queue.retain(|e| !std::sync::Arc::ptr_eq(e, &entry));
        }
        return Ok(JsValue::String("timed-out".into()));
    }
    let res = rt.create_object();
    if cur != expected {
        drop(data_guard);
        let _ = rt.set_property(res, "async", JsValue::Boolean(false));
        let _ = rt.set_property(res, "value", JsValue::String("not-equal".into()));
        return Ok(JsValue::Object(res));
    }
    if q == 0.0 {
        drop(data_guard);
        let _ = rt.set_property(res, "async", JsValue::Boolean(false));
        let _ = rt.set_property(res, "value", JsValue::String("timed-out".into()));
        return Ok(JsValue::Object(res));
    }
    let entry = std::sync::Arc::new(WaiterEntry {
        state: std::sync::Mutex::new(WaiterState::Waiting),
        cv: std::sync::Condvar::new(),
    });
    {
        let mut map = sb.waiters.lock().unwrap();
        map.entry(byte_idx).or_default().push_back(entry.clone());
    }
    drop(data_guard);
    let p = rt.create_promise();
    let deadline = if q.is_finite() {
        Some(
            std::time::Instant::now()
                + std::time::Duration::from_secs_f64((q / 1000.0).min(86400.0)),
        )
    } else {
        None
    };
    rt.async_waiters
        .push((sb, byte_idx, entry, p, deadline));
    let _ = rt.set_property(res, "async", JsValue::Boolean(true));
    let _ = rt.set_property(res, "value", JsValue::Object(p));
    Ok(JsValue::Object(res))
}
