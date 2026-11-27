use crate::builtins::{set_getter, set_global, set_method};
use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::cell::RefCell;
use std::rc::Rc;

pub fn init(rt: &mut JsRuntime) {
    init_sync_stack(rt);
    init_async_stack(rt);
}

fn require_stack(rt: &JsRuntime, this: &JsValue, expect_async: bool) -> Result<ObjectId, JsError> {
    let JsValue::Object(oid) = this else {
        return Err(JsError::type_err(if expect_async {
            "Method called on incompatible receiver (expected AsyncDisposableStack)"
        } else {
            "Method called on incompatible receiver (expected DisposableStack)"
        }));
    };
    match &rt.heap.get_obj(*oid).kind {
        ObjectKind::DisposableStack(ds) if ds.is_async == expect_async => Ok(*oid),
        _ => Err(JsError::type_err(if expect_async {
            "Method called on incompatible receiver (expected AsyncDisposableStack)"
        } else {
            "Method called on incompatible receiver (expected DisposableStack)"
        })),
    }
}

fn is_disposed(rt: &JsRuntime, oid: ObjectId) -> bool {
    match &rt.heap.get_obj(oid).kind {
        ObjectKind::DisposableStack(ds) => ds.disposed,
        _ => true,
    }
}

fn push_resource(rt: &mut JsRuntime, oid: ObjectId, res: DisposableResource) {
    if let ObjectKind::DisposableStack(ref mut ds) = rt.heap.get_obj_mut(oid).kind {
        ds.stack.push(res);
    }
}

fn create_disposable_resource(
    rt: &mut JsRuntime,
    val: &JsValue,
    is_async: bool,
) -> Result<Option<DisposableResource>, JsError> {
    if val.is_nullish() {
        if is_async {
            return Ok(Some(DisposableResource {
                value: JsValue::Undefined,
                hint: DisposeHint::Async,
                method: None,
                adopt_arg: None,
            }));
        }
        return Ok(None);
    }
    let JsValue::Object(oid) = val else {
        return Err(JsError::type_err("Disposable resource must be an object"));
    };
    if is_async {
        let async_key = PropertyKey::Symbol(rt.symbols.async_dispose);
        let m = rt.get_property_key(*oid, &async_key)?;
        if m.is_nullish() {
            let sync_key = PropertyKey::Symbol(rt.symbols.dispose);
            let sm = rt.get_property_key(*oid, &sync_key)?;
            if sm.is_nullish() || !rt.is_callable(&sm) {
                return Err(JsError::type_err("Object is not async disposable"));
            }
            Ok(Some(DisposableResource {
                value: val.clone(),
                hint: DisposeHint::SyncWrappedAsync,
                method: Some(sm),
                adopt_arg: None,
            }))
        } else if !rt.is_callable(&m) {
            Err(JsError::type_err("Symbol.asyncDispose is not callable"))
        } else {
            Ok(Some(DisposableResource {
                value: val.clone(),
                hint: DisposeHint::Async,
                method: Some(m),
                adopt_arg: None,
            }))
        }
    } else {
        let sync_key = PropertyKey::Symbol(rt.symbols.dispose);
        let sm = rt.get_property_key(*oid, &sync_key)?;
        if sm.is_nullish() || !rt.is_callable(&sm) {
            return Err(JsError::type_err("Object is not disposable"));
        }
        Ok(Some(DisposableResource {
            value: val.clone(),
            hint: DisposeHint::Sync,
            method: Some(sm),
            adopt_arg: None,
        }))
    }
}

fn init_sync_stack(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.disposable_stack_proto;
    let sym_dispose = rt.symbols.dispose;
    let sym_tag = rt.symbols.to_string_tag;

    let ctor = rt.create_spec_constructor(
        "DisposableStack",
        0,
        proto,
        Rc::new(move |rt, _, _| {
            let nt = rt.native_new_target.clone().unwrap_or(JsValue::Undefined);
            if nt.is_undefined() {
                return Err(JsError::type_err(
                    "Constructor DisposableStack requires 'new'",
                ));
            }
            let obj_proto = rt.get_prototype_from_new_target(rt.intrinsics.disposable_stack_proto)?;
            let obj = rt.heap.alloc_object(JsObject::new(
                Some(obj_proto),
                ObjectKind::DisposableStack(DisposableStackData {
                    disposed: false,
                    stack: Vec::new(),
                    is_async: false,
                }),
            ));
            Ok(JsValue::Object(obj))
        }),
    );

    set_getter(
        rt,
        proto,
        "disposed",
        Rc::new(|rt, this, _| {
            let oid = require_stack(rt, &this, false)?;
            Ok(JsValue::Boolean(is_disposed(rt, oid)))
        }),
    );

    set_method(
        rt,
        proto,
        "use",
        1,
        Rc::new(|rt, this, args| {
            let oid = require_stack(rt, &this, false)?;
            if is_disposed(rt, oid) {
                return Err(JsError::reference("DisposableStack has already been disposed"));
            }
            let val = args.first().cloned().unwrap_or(JsValue::Undefined);
            if let Some(res) = create_disposable_resource(rt, &val, false)? {
                push_resource(rt, oid, res);
            }
            Ok(val)
        }),
    );

    set_method(
        rt,
        proto,
        "adopt",
        2,
        Rc::new(|rt, this, args| {
            let oid = require_stack(rt, &this, false)?;
            if is_disposed(rt, oid) {
                return Err(JsError::reference("DisposableStack has already been disposed"));
            }
            let val = args.first().cloned().unwrap_or(JsValue::Undefined);
            let on_dispose = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&on_dispose) {
                return Err(JsError::type_err("onDispose is not a function"));
            }
            push_resource(
                rt,
                oid,
                DisposableResource {
                    value: JsValue::Undefined,
                    hint: DisposeHint::Sync,
                    method: Some(on_dispose),
                    adopt_arg: Some(val.clone()),
                },
            );
            Ok(val)
        }),
    );

    set_method(
        rt,
        proto,
        "defer",
        1,
        Rc::new(|rt, this, args| {
            let oid = require_stack(rt, &this, false)?;
            if is_disposed(rt, oid) {
                return Err(JsError::reference("DisposableStack has already been disposed"));
            }
            let on_dispose = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&on_dispose) {
                return Err(JsError::type_err("onDispose is not a function"));
            }
            push_resource(
                rt,
                oid,
                DisposableResource {
                    value: JsValue::Undefined,
                    hint: DisposeHint::Sync,
                    method: Some(on_dispose),
                    adopt_arg: None,
                },
            );
            Ok(JsValue::Undefined)
        }),
    );

    set_method(
        rt,
        proto,
        "move",
        0,
        Rc::new(|rt, this, _| {
            let oid = require_stack(rt, &this, false)?;
            if is_disposed(rt, oid) {
                return Err(JsError::reference("DisposableStack has already been disposed"));
            }
            let moved_stack = match &mut rt.heap.get_obj_mut(oid).kind {
                ObjectKind::DisposableStack(ref mut ds) => {
                    ds.disposed = true;
                    std::mem::take(&mut ds.stack)
                }
                _ => Vec::new(),
            };
            let new_obj = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.disposable_stack_proto),
                ObjectKind::DisposableStack(DisposableStackData {
                    disposed: false,
                    stack: moved_stack,
                    is_async: false,
                }),
            ));
            Ok(JsValue::Object(new_obj))
        }),
    );

    let dispose_fn = rt.create_native_function(
        "dispose",
        0,
        Rc::new(|rt, this, _| {
            let oid = require_stack(rt, &this, false)?;
            if is_disposed(rt, oid) {
                return Ok(JsValue::Undefined);
            }
            let mut stack = match &mut rt.heap.get_obj_mut(oid).kind {
                ObjectKind::DisposableStack(ref mut ds) => {
                    ds.disposed = true;
                    std::mem::take(&mut ds.stack)
                }
                _ => Vec::new(),
            };
            let mut current_err: Option<JsValue> = None;
            while let Some(res) = stack.pop() {
                if let Some(method) = res.method {
                    let r = if let Some(arg) = res.adopt_arg {
                        rt.call_function(method, JsValue::Undefined, &[arg])
                    } else {
                        rt.call_function(method, res.value, &[])
                    };
                    if let Err(e) = r {
                        let ev = rt.error_to_js_value(e);
                        current_err = Some(match current_err {
                            None => ev,
                            Some(suppressed) => rt.create_suppressed_error(ev, suppressed),
                        });
                    }
                }
            }
            if let Some(err) = current_err {
                Err(JsError::Thrown(err))
            } else {
                Ok(JsValue::Undefined)
            }
        }),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::from("dispose"),
        PropEntry::data(JsValue::Object(dispose_fn), true, false, true),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(sym_dispose),
        PropEntry::data(JsValue::Object(dispose_fn), true, false, true),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(sym_tag),
        PropEntry::data(JsValue::String("DisposableStack".into()), false, false, true),
    );

    set_global(rt, "DisposableStack", JsValue::Object(ctor));
}

fn combine_suppressed(rt: &mut JsRuntime, current_err: &Rc<RefCell<Option<JsValue>>>, ev: JsValue) {
    let prev = current_err.borrow_mut().take();
    let next = match prev {
        None => ev,
        Some(suppressed) => rt.create_suppressed_error(ev, suppressed),
    };
    *current_err.borrow_mut() = Some(next);
}

fn drive_async_dispose(
    rt: &mut JsRuntime,
    stack: Rc<RefCell<Vec<DisposableResource>>>,
    current_err: Rc<RefCell<Option<JsValue>>>,
    flags: Rc<RefCell<(bool, bool)>>,
    out_p: ObjectId,
) {
    loop {
        let item = stack.borrow_mut().pop();
        let Some(res) = item else {
            let (needs_await, has_awaited) = *flags.borrow();
            if needs_await && !has_awaited {
                flags.borrow_mut().1 = true;
                let p = rt.promise_resolve_value(JsValue::Undefined);
                let stack_c = stack.clone();
                let err_c = current_err.clone();
                let flags_c = flags.clone();
                let on_done = rt.create_native_function(
                    "",
                    1,
                    Rc::new(move |rt, _, _| {
                        drive_async_dispose(
                            rt,
                            stack_c.clone(),
                            err_c.clone(),
                            flags_c.clone(),
                            out_p,
                        );
                        Ok(JsValue::Undefined)
                    }),
                );
                rt.promise_then(p, Some(JsValue::Object(on_done)), None);
                return;
            }
            let final_err = current_err.borrow_mut().take();
            if let Some(e) = final_err {
                rt.reject_promise(out_p, e);
            } else {
                rt.resolve_promise(out_p, JsValue::Undefined);
            }
            return;
        };

        let Some(method) = res.method else {
            flags.borrow_mut().0 = true;
            continue;
        };

        let call_res = if let Some(arg) = res.adopt_arg {
            rt.call_function(method, JsValue::Undefined, &[arg])
        } else {
            rt.call_function(method, res.value, &[])
        };

        match call_res {
            Err(e) => {
                let ev = rt.error_to_js_value(e);
                combine_suppressed(rt, &current_err, ev);
            }
            Ok(val) => {
                let to_await = match res.hint {
                    DisposeHint::SyncWrappedAsync => JsValue::Undefined,
                    _ => val,
                };
                flags.borrow_mut().1 = true;
                let p = rt.promise_resolve_value(to_await);
                let stack_ok = stack.clone();
                let err_ok = current_err.clone();
                let flags_ok = flags.clone();
                let on_ok = rt.create_native_function(
                    "",
                    1,
                    Rc::new(move |rt, _, _| {
                        drive_async_dispose(
                            rt,
                            stack_ok.clone(),
                            err_ok.clone(),
                            flags_ok.clone(),
                            out_p,
                        );
                        Ok(JsValue::Undefined)
                    }),
                );
                let stack_err = stack.clone();
                let err_err = current_err.clone();
                let flags_err = flags.clone();
                let on_err = rt.create_native_function(
                    "",
                    1,
                    Rc::new(move |rt, _, args| {
                        let rej = args.first().cloned().unwrap_or(JsValue::Undefined);
                        combine_suppressed(rt, &err_err, rej);
                        drive_async_dispose(
                            rt,
                            stack_err.clone(),
                            err_err.clone(),
                            flags_err.clone(),
                            out_p,
                        );
                        Ok(JsValue::Undefined)
                    }),
                );
                rt.promise_then(
                    p,
                    Some(JsValue::Object(on_ok)),
                    Some(JsValue::Object(on_err)),
                );
                return;
            }
        }
    }
}

fn init_async_stack(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.async_disposable_stack_proto;
    let sym_async_dispose = rt.symbols.async_dispose;
    let sym_tag = rt.symbols.to_string_tag;

    let ctor = rt.create_spec_constructor(
        "AsyncDisposableStack",
        0,
        proto,
        Rc::new(move |rt, _, _| {
            let nt = rt.native_new_target.clone().unwrap_or(JsValue::Undefined);
            if nt.is_undefined() {
                return Err(JsError::type_err(
                    "Constructor AsyncDisposableStack requires 'new'",
                ));
            }
            let obj_proto =
                rt.get_prototype_from_new_target(rt.intrinsics.async_disposable_stack_proto)?;
            let obj = rt.heap.alloc_object(JsObject::new(
                Some(obj_proto),
                ObjectKind::DisposableStack(DisposableStackData {
                    disposed: false,
                    stack: Vec::new(),
                    is_async: true,
                }),
            ));
            Ok(JsValue::Object(obj))
        }),
    );

    set_getter(
        rt,
        proto,
        "disposed",
        Rc::new(|rt, this, _| {
            let oid = require_stack(rt, &this, true)?;
            Ok(JsValue::Boolean(is_disposed(rt, oid)))
        }),
    );

    set_method(
        rt,
        proto,
        "use",
        1,
        Rc::new(|rt, this, args| {
            let oid = require_stack(rt, &this, true)?;
            if is_disposed(rt, oid) {
                return Err(JsError::reference(
                    "AsyncDisposableStack has already been disposed",
                ));
            }
            let val = args.first().cloned().unwrap_or(JsValue::Undefined);
            if let Some(res) = create_disposable_resource(rt, &val, true)? {
                push_resource(rt, oid, res);
            }
            Ok(val)
        }),
    );

    set_method(
        rt,
        proto,
        "adopt",
        2,
        Rc::new(|rt, this, args| {
            let oid = require_stack(rt, &this, true)?;
            if is_disposed(rt, oid) {
                return Err(JsError::reference(
                    "AsyncDisposableStack has already been disposed",
                ));
            }
            let val = args.first().cloned().unwrap_or(JsValue::Undefined);
            let on_dispose = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&on_dispose) {
                return Err(JsError::type_err("onDisposeAsync is not a function"));
            }
            push_resource(
                rt,
                oid,
                DisposableResource {
                    value: JsValue::Undefined,
                    hint: DisposeHint::Async,
                    method: Some(on_dispose),
                    adopt_arg: Some(val.clone()),
                },
            );
            Ok(val)
        }),
    );

    set_method(
        rt,
        proto,
        "defer",
        1,
        Rc::new(|rt, this, args| {
            let oid = require_stack(rt, &this, true)?;
            if is_disposed(rt, oid) {
                return Err(JsError::reference(
                    "AsyncDisposableStack has already been disposed",
                ));
            }
            let on_dispose = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&on_dispose) {
                return Err(JsError::type_err("onDisposeAsync is not a function"));
            }
            push_resource(
                rt,
                oid,
                DisposableResource {
                    value: JsValue::Undefined,
                    hint: DisposeHint::Async,
                    method: Some(on_dispose),
                    adopt_arg: None,
                },
            );
            Ok(JsValue::Undefined)
        }),
    );

    set_method(
        rt,
        proto,
        "move",
        0,
        Rc::new(|rt, this, _| {
            let oid = require_stack(rt, &this, true)?;
            if is_disposed(rt, oid) {
                return Err(JsError::reference(
                    "AsyncDisposableStack has already been disposed",
                ));
            }
            let moved_stack = match &mut rt.heap.get_obj_mut(oid).kind {
                ObjectKind::DisposableStack(ref mut ds) => {
                    ds.disposed = true;
                    std::mem::take(&mut ds.stack)
                }
                _ => Vec::new(),
            };
            let new_obj = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.async_disposable_stack_proto),
                ObjectKind::DisposableStack(DisposableStackData {
                    disposed: false,
                    stack: moved_stack,
                    is_async: true,
                }),
            ));
            Ok(JsValue::Object(new_obj))
        }),
    );

    let dispose_async_fn = rt.create_native_function(
        "disposeAsync",
        0,
        Rc::new(|rt, this, _| {
            let out_p = rt.create_promise();
            let oid = match require_stack(rt, &this, true) {
                Ok(o) => o,
                Err(e) => {
                    let ev = rt.error_to_js_value(e);
                    rt.reject_promise(out_p, ev);
                    return Ok(JsValue::Object(out_p));
                }
            };
            if is_disposed(rt, oid) {
                rt.resolve_promise(out_p, JsValue::Undefined);
                return Ok(JsValue::Object(out_p));
            }
            let stack = match &mut rt.heap.get_obj_mut(oid).kind {
                ObjectKind::DisposableStack(ref mut ds) => {
                    ds.disposed = true;
                    std::mem::take(&mut ds.stack)
                }
                _ => Vec::new(),
            };
            drive_async_dispose(
                rt,
                Rc::new(RefCell::new(stack)),
                Rc::new(RefCell::new(None)),
                Rc::new(RefCell::new((false, false))),
                out_p,
            );
            Ok(JsValue::Object(out_p))
        }),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::from("disposeAsync"),
        PropEntry::data(JsValue::Object(dispose_async_fn), true, false, true),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(sym_async_dispose),
        PropEntry::data(JsValue::Object(dispose_async_fn), true, false, true),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(sym_tag),
        PropEntry::data(
            JsValue::String("AsyncDisposableStack".into()),
            false,
            false,
            true,
        ),
    );

    set_global(rt, "AsyncDisposableStack", JsValue::Object(ctor));
}
