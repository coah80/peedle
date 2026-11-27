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
            let on_ful = args.first().cloned();
            let on_rej = args.get(1).cloned();
            let next = rt.promise_then(pid, on_ful, on_rej);
            Ok(JsValue::Object(next))
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
            let on_fin = args.first().cloned().unwrap_or(JsValue::Undefined);
            let oid = rt.to_object(&this)?;
            let then_fn = rt.get_property(oid, "then")?;
            if !rt.is_callable(&on_fin) {
                return rt.call_function(then_fn, this, &[on_fin.clone(), on_fin]);
            }
            let f1 = on_fin.clone();
            let on_ful = rt.create_native_function(
                "",
                1,
                Rc::new(move |rt, _, a| {
                    let val = a.first().cloned().unwrap_or(JsValue::Undefined);
                    let res = rt.call_function(f1.clone(), JsValue::Undefined, &[])?;
                    let p = rt.promise_resolve_value(res);
                    let ret_val = rt.create_native_function(
                        "",
                        0,
                        Rc::new(move |_, _, _| Ok(val.clone())),
                    );
                    Ok(JsValue::Object(rt.promise_then(
                        p,
                        Some(JsValue::Object(ret_val)),
                        None,
                    )))
                }),
            );
            let f2 = on_fin;
            let on_rej = rt.create_native_function(
                "",
                1,
                Rc::new(move |rt, _, a| {
                    let reason = a.first().cloned().unwrap_or(JsValue::Undefined);
                    let res = rt.call_function(f2.clone(), JsValue::Undefined, &[])?;
                    let p = rt.promise_resolve_value(res);
                    let thrower = rt.create_native_function(
                        "",
                        0,
                        Rc::new(move |_, _, _| Err(JsError::thrown(reason.clone()))),
                    );
                    Ok(JsValue::Object(rt.promise_then(
                        p,
                        Some(JsValue::Object(thrower)),
                        None,
                    )))
                }),
            );
            rt.call_function(then_fn, this, &[JsValue::Object(on_ful), JsValue::Object(on_rej)])
        }),
    );

    let ctor = rt.create_native_constructor(
        "Promise",
        1,
        proto,
        Rc::new(|rt, this, args| {
            if !this.is_object() {
                return Err(JsError::type_err("Promise constructor cannot be invoked without 'new'"));
            }
            let executor = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&executor) {
                return Err(JsError::type_err("Promise resolver is not a function"));
            }
            let promise = rt.create_promise();
            let (resolve_fn, reject_fn) = make_resolving_functions(rt, promise);
            if let Err(e) = rt.call_function(
                executor,
                JsValue::Undefined,
                &[JsValue::Object(resolve_fn), JsValue::Object(reject_fn)],
            ) {
                let ev = rt.error_to_js_value(e);
                rt.reject_promise(promise, ev);
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
                    let next_p = rt.call_function(resolve_fn.clone(), this.clone(), &[item])?;
                    let next_oid = rt.to_object(&next_p)?;
                    let then_fn = rt.get_property(next_oid, "then")?;
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
                    rt.call_function(then_fn, next_p, &[JsValue::Object(on_ok), JsValue::Object(on_err)])?;
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
                    let next_p = rt.call_function(resolve_fn.clone(), this.clone(), &[item])?;
                    let next_oid = rt.to_object(&next_p)?;
                    let then_fn = rt.get_property(next_oid, "then")?;
                    rt.call_function(then_fn, next_p, &[cap.resolve.clone(), cap.reject.clone()])?;
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
                    let next_p = rt.call_function(resolve_fn.clone(), this.clone(), &[item])?;
                    let next_oid = rt.to_object(&next_p)?;
                    let then_fn = rt.get_property(next_oid, "then")?;
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
                    rt.call_function(then_fn, next_p, &[cap.resolve.clone(), JsValue::Object(on_err)])?;
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
