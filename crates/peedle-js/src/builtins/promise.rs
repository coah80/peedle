use crate::builtins::{set_global, set_method};
use crate::error::{ErrorKind, JsError};
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct PromiseCapability {
    promise: JsValue,
    resolve: JsValue,
    reject: JsValue,
}

fn new_promise_capability(rt: &mut JsRuntime, ctor: JsValue) -> Result<PromiseCapability, JsError> {
    if !rt.is_constructor(&ctor) {
        return Err(JsError::type_err("Promise constructor is not a constructor"));
    }
    let res_slot: Rc<RefCell<Option<JsValue>>> = Rc::new(RefCell::new(None));
    let rej_slot: Rc<RefCell<Option<JsValue>>> = Rc::new(RefCell::new(None));
    let r1 = res_slot.clone();
    let r2 = rej_slot.clone();
    let executor = rt.create_native_function(
        "",
        2,
        Rc::new(move |_, _, args| {
            if r1.borrow().as_ref().is_some_and(|v| !v.is_undefined())
                || r2.borrow().as_ref().is_some_and(|v| !v.is_undefined())
            {
                return Err(JsError::type_err("Promise capability already initialized"));
            }
            *r1.borrow_mut() = Some(args.first().cloned().unwrap_or(JsValue::Undefined));
            *r2.borrow_mut() = Some(args.get(1).cloned().unwrap_or(JsValue::Undefined));
            Ok(JsValue::Undefined)
        }),
    );
    let promise = rt.construct(ctor, &[JsValue::Object(executor)])?;
    let resolve = res_slot.borrow().clone().unwrap_or(JsValue::Undefined);
    let reject = rej_slot.borrow().clone().unwrap_or(JsValue::Undefined);
    if !rt.is_callable(&resolve) || !rt.is_callable(&reject) {
        return Err(JsError::type_err("Promise resolve or reject function is not callable"));
    }
    Ok(PromiseCapability {
        promise,
        resolve,
        reject,
    })
}

fn get_promise_resolve(rt: &mut JsRuntime, ctor: ObjectId) -> Result<JsValue, JsError> {
    let f = rt.get_property(ctor, "resolve")?;
    if !rt.is_callable(&f) {
        return Err(JsError::type_err("Promise resolve is not callable"));
    }
    Ok(f)
}

fn promise_resolve_ctor(
    rt: &mut JsRuntime,
    ctor: JsValue,
    val: JsValue,
) -> Result<JsValue, JsError> {
    if let JsValue::Object(vid) = val {
        if matches!(rt.heap.get_obj(vid).kind, ObjectKind::Promise(_)) {
            let vctor = rt.get_property(vid, "constructor")?;
            if vctor == ctor {
                return Ok(val);
            }
        }
    }
    let cap = new_promise_capability(rt, ctor)?;
    rt.call_function(cap.resolve, JsValue::Undefined, &[val])?;
    Ok(cap.promise)
}

pub fn init(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.promise_proto;
    let tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(tag),
        PropEntry::data(JsValue::String("Promise".into()), false, false, true),
    );

    set_method(
        rt,
        proto,
        "then",
        2,
        Rc::new(|rt, this, args| {
            let JsValue::Object(pid) = this else {
                return Err(JsError::type_err("Promise.prototype.then called on non-Promise"));
            };
            if !matches!(rt.heap.get_obj(pid).kind, ObjectKind::Promise(_)) {
                return Err(JsError::type_err("Promise.prototype.then called on non-Promise"));
            }
            let ctor = rt.species_constructor(pid, rt.intrinsics.promise_ctor)?;
            let on_ful = args.first().cloned();
            let on_rej = args.get(1).cloned();
            if ctor == JsValue::Object(rt.intrinsics.promise_ctor) {
                let next = rt.promise_then(pid, on_ful, on_rej);
                return Ok(JsValue::Object(next));
            }
            let cap = new_promise_capability(rt, ctor)?;
            rt.promise_then_with_capability(pid, on_ful, on_rej, cap.resolve, cap.reject);
            Ok(cap.promise)
        }),
    );

    set_method(
        rt,
        proto,
        "catch",
        1,
        Rc::new(|rt, this, args| {
            let on_rej = args.first().cloned().unwrap_or(JsValue::Undefined);
            let oid = rt.to_object(&this)?;
            let then_fn = rt.get_property(oid, "then")?;
            rt.call_function(then_fn, this, &[JsValue::Undefined, on_rej])
        }),
    );

    set_method(
        rt,
        proto,
        "finally",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Promise.prototype.finally called on non-object"));
            };
            let on_fin = args.first().cloned().unwrap_or(JsValue::Undefined);
            let ctor = rt.species_constructor(oid, rt.intrinsics.promise_ctor)?;
            let (then_fin, catch_fin) = if !rt.is_callable(&on_fin) {
                (on_fin.clone(), on_fin)
            } else {
                let f1 = on_fin.clone();
                let c1 = ctor.clone();
                let on_ful = rt.create_native_function(
                    "",
                    1,
                    Rc::new(move |rt, _, a| {
                        let val = a.first().cloned().unwrap_or(JsValue::Undefined);
                        let res = rt.call_function(f1.clone(), JsValue::Undefined, &[])?;
                        let p = promise_resolve_ctor(rt, c1.clone(), res)?;
                        let p_oid = rt.to_object(&p)?;
                        let p_then = rt.get_property(p_oid, "then")?;
                        let ret_val = rt.create_native_function(
                            "",
                            0,
                            Rc::new(move |_, _, _| Ok(val.clone())),
                        );
                        rt.call_function(p_then, p, &[JsValue::Object(ret_val)])
                    }),
                );
                let f2 = on_fin;
                let c2 = ctor;
                let on_rej = rt.create_native_function(
                    "",
                    1,
                    Rc::new(move |rt, _, a| {
                        let reason = a.first().cloned().unwrap_or(JsValue::Undefined);
                        let res = rt.call_function(f2.clone(), JsValue::Undefined, &[])?;
                        let p = promise_resolve_ctor(rt, c2.clone(), res)?;
                        let p_oid = rt.to_object(&p)?;
                        let p_then = rt.get_property(p_oid, "then")?;
                        let thrower = rt.create_native_function(
                            "",
                            0,
                            Rc::new(move |_, _, _| Err(JsError::thrown(reason.clone()))),
                        );
                        rt.call_function(p_then, p, &[JsValue::Object(thrower)])
                    }),
                );
                (JsValue::Object(on_ful), JsValue::Object(on_rej))
            };
            let then_fn = rt.get_property(oid, "then")?;
            rt.call_function(then_fn, this, &[then_fin, catch_fin])
        }),
    );

    let ctor = rt.create_spec_constructor(
        "Promise",
        1,
        proto,
        Rc::new(|rt, _, args| {
            rt.require_new_target("Promise")?;
            let executor = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&executor) {
                return Err(JsError::type_err("Promise resolver is not a function"));
            }
            let obj_proto = rt.get_prototype_from_new_target(rt.intrinsics.promise_proto)?;
            let promise = rt.heap.alloc_object(JsObject::new(
                Some(obj_proto),
                ObjectKind::Promise(PromiseData {
                    state: PromiseState::Pending,
                    reactions: Vec::new(),
                    handled: false,
                }),
            ));
            let (resolve_fn, reject_fn) = make_resolving_functions(rt, promise);
            if let Err(e) = rt.call_function(
                executor,
                JsValue::Undefined,
                &[JsValue::Object(resolve_fn), JsValue::Object(reject_fn)],
            ) {
                let ev = rt.error_to_js_value(e);
                let _ = rt.call_function(JsValue::Object(reject_fn), JsValue::Undefined, &[ev]);
            }
            Ok(JsValue::Object(promise))
        }),
    );
    rt.intrinsics.promise_ctor = ctor;

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

    set_method(
        rt,
        ctor,
        "resolve",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(cid) = this else {
                return Err(JsError::type_err("Promise.resolve called on non-object"));
            };
            let v = args.first().cloned().unwrap_or(JsValue::Undefined);
            if let JsValue::Object(vid) = v {
                if matches!(rt.heap.get_obj(vid).kind, ObjectKind::Promise(_)) {
                    let vctor = rt.get_property(vid, "constructor")?;
                    if vctor == JsValue::Object(cid) {
                        return Ok(v);
                    }
                }
            }
            let cap = new_promise_capability(rt, this)?;
            rt.call_function(cap.resolve, JsValue::Undefined, &[v])?;
            Ok(cap.promise)
        }),
    );

    set_method(
        rt,
        ctor,
        "reject",
        1,
        Rc::new(|rt, this, args| {
            if !this.is_object() {
                return Err(JsError::type_err("Promise.reject called on non-object"));
            }
            let reason = args.first().cloned().unwrap_or(JsValue::Undefined);
            let cap = new_promise_capability(rt, this)?;
            rt.call_function(cap.reject, JsValue::Undefined, &[reason])?;
            Ok(cap.promise)
        }),
    );

    set_method(
        rt,
        ctor,
        "withResolvers",
        0,
        Rc::new(|rt, this, _| {
            let cap = new_promise_capability(rt, this)?;
            let obj = rt.create_object();
            let _ = rt.set_property(obj, "promise", cap.promise);
            let _ = rt.set_property(obj, "resolve", cap.resolve);
            let _ = rt.set_property(obj, "reject", cap.reject);
            Ok(JsValue::Object(obj))
        }),
    );

    set_method(
        rt,
        ctor,
        "try",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(cid) = this else {
                return Err(JsError::type_err("Promise.try called on non-object"));
            };
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            let rest = if args.len() > 1 { &args[1..] } else { &[] };
            match rt.call_function(cb, JsValue::Undefined, rest) {
                Ok(v) => {
                    if let JsValue::Object(vid) = v {
                        if matches!(rt.heap.get_obj(vid).kind, ObjectKind::Promise(_)) {
                            let vctor = rt.get_property(vid, "constructor")?;
                            if vctor == JsValue::Object(cid) {
                                return Ok(v);
                            }
                        }
                    }
                    let cap = new_promise_capability(rt, this)?;
                    rt.call_function(cap.resolve, JsValue::Undefined, &[v])?;
                    Ok(cap.promise)
                }
                Err(e) => {
                    let cap = new_promise_capability(rt, this)?;
                    let ev = rt.error_to_js_value(e);
                    rt.call_function(cap.reject, JsValue::Undefined, &[ev])?;
                    Ok(cap.promise)
                }
            }
        }),
    );

    set_method(
        rt,
        ctor,
        "all",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(cid) = this else {
                return Err(JsError::type_err("Promise.all called on non-object"));
            };
            let cap = new_promise_capability(rt, this.clone())?;
            let res = (|| -> Result<(), JsError> {
                let resolve_fn = get_promise_resolve(rt, cid)?;
                let iter = rt.get_iterator(args.first().unwrap_or(&JsValue::Undefined), false)?;
                let results = Rc::new(RefCell::new(Vec::<JsValue>::new()));
                let remaining = Rc::new(Cell::new(1usize));
                let mut index = 0usize;
                while let Some(item) = rt.iterator_step(&iter)? {
                    results.borrow_mut().push(JsValue::Undefined);
                    let next_p = match rt.call_function(resolve_fn.clone(), this.clone(), &[item]) {
                        Ok(p) => p,
                        Err(e) => {
                            rt.iterator_close(&iter);
                            return Err(e);
                        }
                    };
                    let next_oid = match rt.to_object(&next_p) {
                        Ok(o) => o,
                        Err(e) => {
                            rt.iterator_close(&iter);
                            return Err(e);
                        }
                    };
                    let then_fn = match rt.get_property(next_oid, "then") {
                        Ok(f) => f,
                        Err(e) => {
                            rt.iterator_close(&iter);
                            return Err(e);
                        }
                    };
                    let called = Rc::new(Cell::new(false));
                    let res_ref = results.clone();
                    let rem_ref = remaining.clone();
                    let cap_resolve = cap.resolve.clone();
                    let cur_idx = index;
                    let on_ok = rt.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, a| {
                            if called.replace(true) {
                                return Ok(JsValue::Undefined);
                            }
                            let v = a.first().cloned().unwrap_or(JsValue::Undefined);
                            res_ref.borrow_mut()[cur_idx] = v;
                            let left = rem_ref.get() - 1;
                            rem_ref.set(left);
                            if left == 0 {
                                let vals = res_ref.borrow().clone();
                                let arr = rt.create_array(vals);
                                rt.call_function(
                                    cap_resolve.clone(),
                                    JsValue::Undefined,
                                    &[JsValue::Object(arr)],
                                )?;
                            }
                            Ok(JsValue::Undefined)
                        }),
                    );
                    remaining.set(remaining.get() + 1);
                    if let Err(e) = rt.call_function(
                        then_fn,
                        next_p,
                        &[JsValue::Object(on_ok), cap.reject.clone()],
                    ) {
                        rt.iterator_close(&iter);
                        return Err(e);
                    }
                    index += 1;
                }
                let left = remaining.get() - 1;
                remaining.set(left);
                if left == 0 {
                    let vals = results.borrow().clone();
                    let arr = rt.create_array(vals);
                    rt.call_function(cap.resolve.clone(), JsValue::Undefined, &[JsValue::Object(arr)])?;
                }
                Ok(())
            })();
            if let Err(e) = res {
                let ev = rt.error_to_js_value(e);
                let _ = rt.call_function(cap.reject, JsValue::Undefined, &[ev]);
            }
            Ok(cap.promise)
        }),
    );

    set_method(
        rt,
        ctor,
        "allSettled",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(cid) = this else {
                return Err(JsError::type_err("Promise.allSettled called on non-object"));
            };
            let cap = new_promise_capability(rt, this.clone())?;
            let res = (|| -> Result<(), JsError> {
                let resolve_fn = get_promise_resolve(rt, cid)?;
                let iter = rt.get_iterator(args.first().unwrap_or(&JsValue::Undefined), false)?;
                let results = Rc::new(RefCell::new(Vec::<JsValue>::new()));
                let remaining = Rc::new(Cell::new(1usize));
                let mut index = 0usize;
                while let Some(item) = rt.iterator_step(&iter)? {
                    results.borrow_mut().push(JsValue::Undefined);
                    let next_p = match rt.call_function(resolve_fn.clone(), this.clone(), &[item]) {
                        Ok(p) => p,
                        Err(e) => {
                            rt.iterator_close(&iter);
                            return Err(e);
                        }
                    };
                    let next_oid = match rt.to_object(&next_p) {
                        Ok(o) => o,
                        Err(e) => {
                            rt.iterator_close(&iter);
                            return Err(e);
                        }
                    };
                    let then_fn = match rt.get_property(next_oid, "then") {
                        Ok(f) => f,
                        Err(e) => {
                            rt.iterator_close(&iter);
                            return Err(e);
                        }
                    };
                    let called = Rc::new(Cell::new(false));
                    let c1 = called.clone();
                    let res1 = results.clone();
                    let rem1 = remaining.clone();
                    let cr1 = cap.resolve.clone();
                    let cur_idx = index;
                    let on_ok = rt.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, a| {
                            if c1.replace(true) {
                                return Ok(JsValue::Undefined);
                            }
                            let v = a.first().cloned().unwrap_or(JsValue::Undefined);
                            let entry = rt.create_object();
                            let _ = rt.set_property(entry, "status", JsValue::String("fulfilled".into()));
                            let _ = rt.set_property(entry, "value", v);
                            res1.borrow_mut()[cur_idx] = JsValue::Object(entry);
                            let left = rem1.get() - 1;
                            rem1.set(left);
                            if left == 0 {
                                let arr = rt.create_array(res1.borrow().clone());
                                rt.call_function(cr1.clone(), JsValue::Undefined, &[JsValue::Object(arr)])?;
                            }
                            Ok(JsValue::Undefined)
                        }),
                    );
                    let res2 = results.clone();
                    let rem2 = remaining.clone();
                    let cr2 = cap.resolve.clone();
                    let on_err = rt.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, a| {
                            if called.replace(true) {
                                return Ok(JsValue::Undefined);
                            }
                            let r = a.first().cloned().unwrap_or(JsValue::Undefined);
                            let entry = rt.create_object();
                            let _ = rt.set_property(entry, "status", JsValue::String("rejected".into()));
                            let _ = rt.set_property(entry, "reason", r);
                            res2.borrow_mut()[cur_idx] = JsValue::Object(entry);
                            let left = rem2.get() - 1;
                            rem2.set(left);
                            if left == 0 {
                                let arr = rt.create_array(res2.borrow().clone());
                                rt.call_function(cr2.clone(), JsValue::Undefined, &[JsValue::Object(arr)])?;
                            }
                            Ok(JsValue::Undefined)
                        }),
                    );
                    remaining.set(remaining.get() + 1);
                    if let Err(e) = rt.call_function(
                        then_fn,
                        next_p,
                        &[JsValue::Object(on_ok), JsValue::Object(on_err)],
                    ) {
                        rt.iterator_close(&iter);
                        return Err(e);
                    }
                    index += 1;
                }
                let left = remaining.get() - 1;
                remaining.set(left);
                if left == 0 {
                    let arr = rt.create_array(results.borrow().clone());
                    rt.call_function(cap.resolve.clone(), JsValue::Undefined, &[JsValue::Object(arr)])?;
                }
                Ok(())
            })();
            if let Err(e) = res {
                let ev = rt.error_to_js_value(e);
                let _ = rt.call_function(cap.reject, JsValue::Undefined, &[ev]);
            }
            Ok(cap.promise)
        }),
    );

    set_method(
        rt,
        ctor,
        "race",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(cid) = this else {
                return Err(JsError::type_err("Promise.race called on non-object"));
            };
            let cap = new_promise_capability(rt, this.clone())?;
            let res = (|| -> Result<(), JsError> {
                let resolve_fn = get_promise_resolve(rt, cid)?;
                let iter = rt.get_iterator(args.first().unwrap_or(&JsValue::Undefined), false)?;
                while let Some(item) = rt.iterator_step(&iter)? {
                    let next_p = match rt.call_function(resolve_fn.clone(), this.clone(), &[item]) {
                        Ok(p) => p,
                        Err(e) => {
                            rt.iterator_close(&iter);
                            return Err(e);
                        }
                    };
                    let next_oid = match rt.to_object(&next_p) {
                        Ok(o) => o,
                        Err(e) => {
                            rt.iterator_close(&iter);
                            return Err(e);
                        }
                    };
                    let then_fn = match rt.get_property(next_oid, "then") {
                        Ok(f) => f,
                        Err(e) => {
                            rt.iterator_close(&iter);
                            return Err(e);
                        }
                    };
                    if let Err(e) = rt.call_function(
                        then_fn,
                        next_p,
                        &[cap.resolve.clone(), cap.reject.clone()],
                    ) {
                        rt.iterator_close(&iter);
                        return Err(e);
                    }
                }
                Ok(())
            })();
            if let Err(e) = res {
                let ev = rt.error_to_js_value(e);
                let _ = rt.call_function(cap.reject, JsValue::Undefined, &[ev]);
            }
            Ok(cap.promise)
        }),
    );

    set_method(
        rt,
        ctor,
        "any",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(cid) = this else {
                return Err(JsError::type_err("Promise.any called on non-object"));
            };
            let cap = new_promise_capability(rt, this.clone())?;
            let res = (|| -> Result<(), JsError> {
                let resolve_fn = get_promise_resolve(rt, cid)?;
                let iter = rt.get_iterator(args.first().unwrap_or(&JsValue::Undefined), false)?;
                let errors = Rc::new(RefCell::new(Vec::<JsValue>::new()));
                let remaining = Rc::new(Cell::new(1usize));
                let mut index = 0usize;
                while let Some(item) = rt.iterator_step(&iter)? {
                    errors.borrow_mut().push(JsValue::Undefined);
                    let next_p = match rt.call_function(resolve_fn.clone(), this.clone(), &[item]) {
                        Ok(v) => v,
                        Err(e) => {
                            rt.iterator_close(&iter);
                            return Err(e);
                        }
                    };
                    let next_oid = match rt.to_object(&next_p) {
                        Ok(o) => o,
                        Err(e) => {
                            rt.iterator_close(&iter);
                            return Err(e);
                        }
                    };
                    let then_fn = match rt.get_property(next_oid, "then") {
                        Ok(f) => f,
                        Err(e) => {
                            rt.iterator_close(&iter);
                            return Err(e);
                        }
                    };
                    let called = Rc::new(Cell::new(false));
                    let errs_ref = errors.clone();
                    let rem_ref = remaining.clone();
                    let cap_rej = cap.reject.clone();
                    let cur_idx = index;
                    let on_err = rt.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, a| {
                            if called.replace(true) {
                                return Ok(JsValue::Undefined);
                            }
                            let r = a.first().cloned().unwrap_or(JsValue::Undefined);
                            errs_ref.borrow_mut()[cur_idx] = r;
                            let left = rem_ref.get() - 1;
                            rem_ref.set(left);
                            if left == 0 {
                                let err = rt.create_error_object(
                                    ErrorKind::AggregateError,
                                    "All promises were rejected",
                                );
                                let arr = rt.create_array(errs_ref.borrow().clone());
                                let _ = rt.set_property(err, "errors", JsValue::Object(arr));
                                rt.call_function(
                                    cap_rej.clone(),
                                    JsValue::Undefined,
                                    &[JsValue::Object(err)],
                                )?;
                            }
                            Ok(JsValue::Undefined)
                        }),
                    );
                    remaining.set(remaining.get() + 1);
                    if let Err(e) = rt.call_function(
                        then_fn,
                        next_p,
                        &[cap.resolve.clone(), JsValue::Object(on_err)],
                    ) {
                        rt.iterator_close(&iter);
                        return Err(e);
                    }
                    index += 1;
                }
                let left = remaining.get() - 1;
                remaining.set(left);
                if left == 0 {
                    let err = rt.create_error_object(
                        ErrorKind::AggregateError,
                        "All promises were rejected",
                    );
                    let arr = rt.create_array(errors.borrow().clone());
                    let _ = rt.set_property(err, "errors", JsValue::Object(arr));
                    rt.call_function(cap.reject.clone(), JsValue::Undefined, &[JsValue::Object(err)])?;
                }
                Ok(())
            })();
            if let Err(e) = res {
                let ev = rt.error_to_js_value(e);
                let _ = rt.call_function(cap.reject, JsValue::Undefined, &[ev]);
            }
            Ok(cap.promise)
        }),
    );

    set_method(
        rt,
        ctor,
        "allSettledKeyed",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(cid) = this else {
                return Err(JsError::type_err(
                    "Promise.allSettledKeyed called on non-object",
                ));
            };
            let cap = new_promise_capability(rt, this.clone())?;
            let res = (|| -> Result<(), JsError> {
                let resolve_fn = get_promise_resolve(rt, cid)?;
                let input = args.first().cloned().unwrap_or(JsValue::Undefined);
                let JsValue::Object(pid) = input else {
                    return Err(JsError::type_err(
                        "Promise.allSettledKeyed argument must be an object",
                    ));
                };
                let all_keys = rt.own_keys(pid)?;
                let mut pairs: Vec<(PropertyKey, JsValue)> = Vec::new();
                for k in all_keys {
                    if let Some(desc) = rt.get_own_property_descriptor(pid, &k)? {
                        if desc.enumerable.unwrap_or(false) {
                            let v = rt.get_property_key(pid, &k)?;
                            pairs.push((k, v));
                        }
                    }
                }
                let keys: Rc<Vec<PropertyKey>> =
                    Rc::new(pairs.iter().map(|(k, _)| k.clone()).collect());
                let results = Rc::new(RefCell::new(vec![JsValue::Undefined; pairs.len()]));
                let remaining = Rc::new(Cell::new(1usize));
                for (cur_idx, (_, item)) in pairs.into_iter().enumerate() {
                    let next_p = rt.call_function(resolve_fn.clone(), this.clone(), &[item])?;
                    let next_oid = rt.to_object(&next_p)?;
                    let then_fn = rt.get_property(next_oid, "then")?;
                    let called = Rc::new(Cell::new(false));
                    let called2 = called.clone();
                    let res1 = results.clone();
                    let res2 = results.clone();
                    let keys1 = keys.clone();
                    let keys2 = keys.clone();
                    let rem1 = remaining.clone();
                    let rem2 = remaining.clone();
                    let cr1 = cap.resolve.clone();
                    let cr2 = cap.resolve.clone();
                    let on_ok = rt.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, a| {
                            if called.replace(true) {
                                return Ok(JsValue::Undefined);
                            }
                            let v = a.first().cloned().unwrap_or(JsValue::Undefined);
                            let entry = rt.create_object();
                            let _ = rt.set_property(
                                entry,
                                "status",
                                JsValue::String("fulfilled".into()),
                            );
                            let _ = rt.set_property(entry, "value", v);
                            res1.borrow_mut()[cur_idx] = JsValue::Object(entry);
                            let left = rem1.get() - 1;
                            rem1.set(left);
                            if left == 0 {
                                let out = rt.create_object_with_proto(None);
                                let vals = res1.borrow();
                                for (k, val) in keys1.iter().zip(vals.iter()) {
                                    let _ = rt.define_property(
                                        out,
                                        k.clone(),
                                        PropertyDescriptor::data(val.clone(), true, true, true),
                                    );
                                }
                                rt.call_function(
                                    cr1.clone(),
                                    JsValue::Undefined,
                                    &[JsValue::Object(out)],
                                )?;
                            }
                            Ok(JsValue::Undefined)
                        }),
                    );
                    let on_err = rt.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, a| {
                            if called2.replace(true) {
                                return Ok(JsValue::Undefined);
                            }
                            let r = a.first().cloned().unwrap_or(JsValue::Undefined);
                            let entry = rt.create_object();
                            let _ = rt.set_property(
                                entry,
                                "status",
                                JsValue::String("rejected".into()),
                            );
                            let _ = rt.set_property(entry, "reason", r);
                            res2.borrow_mut()[cur_idx] = JsValue::Object(entry);
                            let left = rem2.get() - 1;
                            rem2.set(left);
                            if left == 0 {
                                let out = rt.create_object_with_proto(None);
                                let vals = res2.borrow();
                                for (k, val) in keys2.iter().zip(vals.iter()) {
                                    let _ = rt.define_property(
                                        out,
                                        k.clone(),
                                        PropertyDescriptor::data(val.clone(), true, true, true),
                                    );
                                }
                                rt.call_function(
                                    cr2.clone(),
                                    JsValue::Undefined,
                                    &[JsValue::Object(out)],
                                )?;
                            }
                            Ok(JsValue::Undefined)
                        }),
                    );
                    remaining.set(remaining.get() + 1);
                    rt.call_function(
                        then_fn,
                        next_p,
                        &[JsValue::Object(on_ok), JsValue::Object(on_err)],
                    )?;
                }
                let left = remaining.get() - 1;
                remaining.set(left);
                if left == 0 {
                    let out = rt.create_object_with_proto(None);
                    let vals = results.borrow();
                    for (k, val) in keys.iter().zip(vals.iter()) {
                        let _ = rt.define_property(
                            out,
                            k.clone(),
                            PropertyDescriptor::data(val.clone(), true, true, true),
                        );
                    }
                    rt.call_function(
                        cap.resolve.clone(),
                        JsValue::Undefined,
                        &[JsValue::Object(out)],
                    )?;
                }
                Ok(())
            })();
            if let Err(e) = res {
                let ev = rt.error_to_js_value(e);
                let _ = rt.call_function(cap.reject, JsValue::Undefined, &[ev]);
            }
            Ok(cap.promise)
        }),
    );

    set_method(
        rt,
        ctor,
        "allKeyed",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(cid) = this else {
                return Err(JsError::type_err("Promise.allKeyed called on non-object"));
            };
            let cap = new_promise_capability(rt, this.clone())?;
            let res = (|| -> Result<(), JsError> {
                let resolve_fn = get_promise_resolve(rt, cid)?;
                let promises_arg = args.first().cloned().unwrap_or(JsValue::Undefined);
                let JsValue::Object(pid) = promises_arg else {
                    return Err(JsError::type_err("Promise.allKeyed requires an object"));
                };
                let all_keys = rt.own_keys(pid)?;
                let keys_list = Rc::new(RefCell::new(Vec::<PropertyKey>::new()));
                let values_list = Rc::new(RefCell::new(Vec::<JsValue>::new()));
                let remaining = Rc::new(Cell::new(1usize));
                let mut index = 0usize;
                for key in all_keys {
                    let Some(desc) = rt.get_own_property_descriptor(pid, &key)? else {
                        continue;
                    };
                    if !desc.enumerable.unwrap_or(false) {
                        continue;
                    }
                    let item = rt.get_property_key(pid, &key)?;
                    keys_list.borrow_mut().push(key);
                    values_list.borrow_mut().push(JsValue::Undefined);
                    let next_p = rt.call_function(resolve_fn.clone(), this.clone(), &[item])?;
                    let next_oid = rt.to_object(&next_p)?;
                    let then_fn = rt.get_property(next_oid, "then")?;
                    let called = Rc::new(Cell::new(false));
                    let k_ref = keys_list.clone();
                    let v_ref = values_list.clone();
                    let rem_ref = remaining.clone();
                    let cap_resolve = cap.resolve.clone();
                    let cur_idx = index;
                    let on_ok = rt.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, a| {
                            if called.replace(true) {
                                return Ok(JsValue::Undefined);
                            }
                            let v = a.first().cloned().unwrap_or(JsValue::Undefined);
                            v_ref.borrow_mut()[cur_idx] = v;
                            let left = rem_ref.get() - 1;
                            rem_ref.set(left);
                            if left == 0 {
                                let out = rt.create_object_with_proto(None);
                                let ks = k_ref.borrow();
                                let vs = v_ref.borrow();
                                for (k, val) in ks.iter().zip(vs.iter()) {
                                    let _ = rt.define_property(
                                        out,
                                        k.clone(),
                                        PropertyDescriptor::data(val.clone(), true, true, true),
                                    );
                                }
                                rt.call_function(
                                    cap_resolve.clone(),
                                    JsValue::Undefined,
                                    &[JsValue::Object(out)],
                                )?;
                            }
                            Ok(JsValue::Undefined)
                        }),
                    );
                    remaining.set(remaining.get() + 1);
                    rt.call_function(then_fn, next_p, &[JsValue::Object(on_ok), cap.reject.clone()])?;
                    index += 1;
                }
                let left = remaining.get() - 1;
                remaining.set(left);
                if left == 0 {
                    let out = rt.create_object_with_proto(None);
                    let ks = keys_list.borrow();
                    let vs = values_list.borrow();
                    for (k, val) in ks.iter().zip(vs.iter()) {
                        let _ = rt.define_property(
                            out,
                            k.clone(),
                            PropertyDescriptor::data(val.clone(), true, true, true),
                        );
                    }
                    rt.call_function(cap.resolve.clone(), JsValue::Undefined, &[JsValue::Object(out)])?;
                }
                Ok(())
            })();
            if let Err(e) = res {
                let ev = rt.error_to_js_value(e);
                let _ = rt.call_function(cap.reject, JsValue::Undefined, &[ev]);
            }
            Ok(cap.promise)
        }),
    );

    set_global(rt, "Promise", JsValue::Object(ctor));
}

fn make_resolving_functions(rt: &mut JsRuntime, promise: ObjectId) -> (ObjectId, ObjectId) {
    let settled = Rc::new(Cell::new(false));
    let s1 = settled.clone();
    let resolve_fn = rt.create_native_function(
        "",
        1,
        Rc::new(move |rt, _, args| {
            if !s1.replace(true) {
                let v = args.first().cloned().unwrap_or(JsValue::Undefined);
                rt.resolve_promise(promise, v);
            }
            Ok(JsValue::Undefined)
        }),
    );
    let s2 = settled;
    let reject_fn = rt.create_native_function(
        "",
        1,
        Rc::new(move |rt, _, args| {
            if !s2.replace(true) {
                let r = args.first().cloned().unwrap_or(JsValue::Undefined);
                rt.reject_promise(promise, r);
            }
            Ok(JsValue::Undefined)
        }),
    );
    (resolve_fn, reject_fn)
}
