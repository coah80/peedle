use peedle_js::{JsError, JsRuntime, JsValue, SharedBufferBacking};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

struct AgentHub {
    reports: Mutex<VecDeque<String>>,
    broadcasts: Mutex<Vec<(Arc<SharedBufferBacking>, Option<i32>)>>,
    broadcast_cv: Condvar,
    shutdown: AtomicBool,
    start_instant: Instant,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl AgentHub {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            reports: Mutex::new(VecDeque::new()),
            broadcasts: Mutex::new(Vec::new()),
            broadcast_cv: Condvar::new(),
            shutdown: AtomicBool::new(false),
            start_instant: Instant::now(),
            threads: Mutex::new(Vec::new()),
        })
    }
}

struct AgentGuard(Arc<AgentHub>);

impl Drop for AgentGuard {
    fn drop(&mut self) {
        self.0.shutdown.store(true, Ordering::SeqCst);
        self.0.broadcast_cv.notify_all();
        if let Ok(list) = self.0.broadcasts.lock() {
            for (backing, _) in list.iter() {
                if let Ok(mut wmap) = backing.waiters.lock() {
                    for q in wmap.values_mut() {
                        for entry in q.drain(..) {
                            let mut st = entry.state.lock().unwrap();
                            *st = peedle_js::heap::WaiterState::Notified;
                            entry.cv.notify_all();
                        }
                    }
                }
            }
        }
        let handles = {
            let mut guard = self.0.threads.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        for h in handles {
            let _ = h.join();
        }
    }
}

fn install_set_timeout(
    rt: &mut JsRuntime,
    timers: Rc<RefCell<Vec<(Instant, JsValue)>>>,
) {
    let timers_gc = timers.clone();
    rt.add_gc_root_tracer(move |roots| {
        for (_, cb) in timers_gc.borrow().iter() {
            if let JsValue::Object(oid) = cb {
                roots.push(*oid);
            }
        }
    });
    let set_timeout_fn = rt.create_native_function(
        "setTimeout",
        2,
        Rc::new(move |rt, _, args| {
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            let ms = match args.get(1) {
                None | Some(JsValue::Undefined) => 0.0,
                Some(v) => rt.to_number(v).unwrap_or(0.0),
            };
            let clamped = if ms.is_nan() || ms < 0.0 {
                0u64
            } else {
                ms.min(60_000.0) as u64
            };
            timers
                .borrow_mut()
                .push((Instant::now() + Duration::from_millis(clamped), cb));
            Ok(JsValue::Number(1.0))
        }),
    );
    let g = rt.global_object();
    let _ = rt.set_property(g, "setTimeout", JsValue::Object(set_timeout_fn));
}

fn pump_event_loop(
    rt: &mut JsRuntime,
    timers: &Rc<RefCell<Vec<(Instant, JsValue)>>>,
    print_log: Option<&Rc<RefCell<Vec<String>>>>,
    hub: &Arc<AgentHub>,
    max_wait: Duration,
) {
    let deadline = Instant::now() + max_wait;
    loop {
        if let Some(log) = print_log {
            let lines = log.borrow();
            if lines
                .iter()
                .any(|l| l.contains("Test262:AsyncTestComplete") || l.contains("Test262:AsyncTestFailure"))
            {
                break;
            }
        }
        if hub.shutdown.load(Ordering::Relaxed) {
            break;
        }
        let mut progressed = rt.tick_async_waiters();
        let now = Instant::now();
        let mut due = Vec::new();
        {
            let mut list = timers.borrow_mut();
            let mut i = 0;
            while i < list.len() {
                if list[i].0 <= now {
                    due.push(list.remove(i).1);
                } else {
                    i += 1;
                }
            }
        }
        if !due.is_empty() {
            progressed = true;
            for cb in due {
                let _ = rt.call_function(cb, JsValue::Undefined, &[]);
                rt.drain_microtasks();
            }
        }
        if progressed {
            rt.drain_microtasks();
            continue;
        }
        let has_timers = !timers.borrow().is_empty();
        let has_waiters = rt.has_pending_async_waiters();
        if !has_timers && !has_waiters {
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        let mut next_wake = Instant::now() + Duration::from_millis(2);
        if let Some(t) = timers.borrow().iter().map(|(t, _)| *t).min() {
            if t < next_wake {
                next_wake = t;
            }
        }
        if let Some(t) = rt.next_async_waiter_deadline() {
            if t < next_wake {
                next_wake = t;
            }
        }
        let now2 = Instant::now();
        if next_wake > now2 {
            std::thread::sleep((next_wake - now2).min(Duration::from_millis(2)));
        }
    }
}

fn make_agent_object(rt: &mut JsRuntime, hub: Arc<AgentHub>) -> peedle_js::ObjectId {
    let agent = rt.create_object();
    let broadcast_idx = Rc::new(RefCell::new(0usize));

    let hub_start = hub.clone();
    let start_fn = rt.create_native_function(
        "start",
        1,
        Rc::new(move |rt, _, args| {
            let script = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            let hub_w = hub_start.clone();
            let handle = std::thread::spawn(move || {
                let mut wrt = JsRuntime::new();
                wrt.max_steps = Some(100_000_000);
                wrt.max_call_depth = 128;
                let wtimers = Rc::new(RefCell::new(Vec::<(Instant, JsValue)>::new()));
                install_set_timeout(&mut wrt, wtimers.clone());
                let h = make_host_262(&mut wrt, 0, hub_w.clone());
                let wg = wrt.global_object();
                let _ = wrt.set_property(wg, "$262", JsValue::Object(h));
                let _ = wrt.eval(&script);
                pump_event_loop(&mut wrt, &wtimers, None, &hub_w, Duration::from_secs(15));
            });
            hub_start.threads.lock().unwrap().push(handle);
            Ok(JsValue::Undefined)
        }),
    );

    let hub_bcast = hub.clone();
    let broadcast_fn = rt.create_native_function(
        "broadcast",
        1,
        Rc::new(move |rt, _, args| {
            let Some(JsValue::Object(oid)) = args.first() else {
                return Err(JsError::type_err("broadcast requires a SharedArrayBuffer"));
            };
            let Some(backing) = rt.shared_buffer_backing(*oid) else {
                return Err(JsError::type_err("broadcast requires a SharedArrayBuffer"));
            };
            let extra = match args.get(1) {
                None | Some(JsValue::Undefined) => None,
                Some(v) => Some(rt.to_number(v)? as i32),
            };
            {
                let mut list = hub_bcast.broadcasts.lock().unwrap();
                list.push((backing, extra));
            }
            hub_bcast.broadcast_cv.notify_all();
            Ok(JsValue::Undefined)
        }),
    );

    let hub_recv = hub.clone();
    let bidx_recv = broadcast_idx.clone();
    let receive_broadcast_fn = rt.create_native_function(
        "receiveBroadcast",
        1,
        Rc::new(move |rt, _, args| {
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            let idx = *bidx_recv.borrow();
            let item = {
                let mut guard = hub_recv.broadcasts.lock().unwrap();
                loop {
                    if idx < guard.len() {
                        break Some(guard[idx].clone());
                    }
                    if hub_recv.shutdown.load(Ordering::SeqCst) {
                        break None;
                    }
                    let (g, _) = hub_recv
                        .broadcast_cv
                        .wait_timeout(guard, Duration::from_millis(50))
                        .unwrap();
                    guard = g;
                }
            };
            if let Some((backing, extra)) = item {
                *bidx_recv.borrow_mut() = idx + 1;
                let sab = rt.create_shared_array_buffer_from_backing(backing);
                let mut call_args = vec![JsValue::Object(sab)];
                if let Some(id) = extra {
                    call_args.push(JsValue::Number(id as f64));
                }
                let _ = rt.call_function(cb, JsValue::Undefined, &call_args)?;
                rt.drain_microtasks();
            }
            Ok(JsValue::Undefined)
        }),
    );

    let hub_rep = hub.clone();
    let report_fn = rt.create_native_function(
        "report",
        1,
        Rc::new(move |rt, _, args| {
            let s = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            hub_rep.reports.lock().unwrap().push_back(s);
            Ok(JsValue::Undefined)
        }),
    );

    let hub_get = hub.clone();
    let get_report_fn = rt.create_native_function(
        "getReport",
        0,
        Rc::new(move |_, _, _| {
            let item = hub_get.reports.lock().unwrap().pop_front();
            Ok(match item {
                Some(s) => JsValue::String(s),
                None => JsValue::Null,
            })
        }),
    );

    let sleep_fn = rt.create_native_function(
        "sleep",
        1,
        Rc::new(|rt, _, args| {
            let ms = match args.first() {
                None | Some(JsValue::Undefined) => 0.0,
                Some(v) => rt.to_number(v)?,
            };
            if ms > 0.0 && ms.is_finite() {
                std::thread::sleep(Duration::from_millis(ms.min(60_000.0) as u64));
            }
            Ok(JsValue::Undefined)
        }),
    );

    let hub_mono = hub.clone();
    let mono_fn = rt.create_native_function(
        "monotonicNow",
        0,
        Rc::new(move |_, _, _| {
            Ok(JsValue::Number(
                hub_mono.start_instant.elapsed().as_secs_f64() * 1000.0,
            ))
        }),
    );

    let leaving_fn = rt.create_native_function("leaving", 0, Rc::new(|_, _, _| Ok(JsValue::Undefined)));

    let _ = rt.set_property(agent, "start", JsValue::Object(start_fn));
    let _ = rt.set_property(agent, "broadcast", JsValue::Object(broadcast_fn));
    let _ = rt.set_property(agent, "receiveBroadcast", JsValue::Object(receive_broadcast_fn));
    let _ = rt.set_property(agent, "report", JsValue::Object(report_fn));
    let _ = rt.set_property(agent, "getReport", JsValue::Object(get_report_fn));
    let _ = rt.set_property(agent, "sleep", JsValue::Object(sleep_fn));
    let _ = rt.set_property(agent, "monotonicNow", JsValue::Object(mono_fn));
    let _ = rt.set_property(agent, "leaving", JsValue::Object(leaving_fn));
    agent
}

fn test262_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/test262")
}

#[derive(Default)]
struct TestMeta {
    includes: Vec<String>,
    flags: Vec<String>,
    features: Vec<String>,
    negative_phase: Option<String>,
    negative_type: Option<String>,
}

fn parse_meta(src: &str) -> TestMeta {
    let mut meta = TestMeta::default();
    let Some(start) = src.find("/*---") else {
        return meta;
    };
    let rest = &src[start + 5..];
    let Some(end) = rest.find("---*/") else {
        return meta;
    };
    let yaml = &rest[..end];
    let mut section = "";
    for line in yaml.split(|c| c == '\n' || c == '\r') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !line.starts_with(' ') && !line.starts_with('\t') {
            section = "";
            if let Some(v) = trimmed.strip_prefix("includes:") {
                section = "includes";
                parse_inline_list(v, &mut meta.includes);
            } else if let Some(v) = trimmed.strip_prefix("flags:") {
                section = "flags";
                parse_inline_list(v, &mut meta.flags);
            } else if let Some(v) = trimmed.strip_prefix("features:") {
                section = "features";
                parse_inline_list(v, &mut meta.features);
            } else if trimmed.starts_with("negative:") {
                section = "negative";
            }
        } else {
            match section {
                "includes" => {
                    if let Some(item) = trimmed.strip_prefix('-') {
                        meta.includes.push(item.trim().to_string());
                    }
                }
                "flags" => {
                    if let Some(item) = trimmed.strip_prefix('-') {
                        meta.flags.push(item.trim().to_string());
                    }
                }
                "features" => {
                    if let Some(item) = trimmed.strip_prefix('-') {
                        meta.features.push(item.trim().to_string());
                    }
                }
                "negative" => {
                    if let Some(v) = trimmed.strip_prefix("phase:") {
                        meta.negative_phase = Some(v.trim().to_string());
                    } else if let Some(v) = trimmed.strip_prefix("type:") {
                        meta.negative_type = Some(v.trim().to_string());
                    }
                }
                _ => {}
            }
        }
    }
    meta
}

fn parse_inline_list(raw: &str, out: &mut Vec<String>) {
    let t = raw.trim();
    if let Some(inner) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        for part in inner.split(',') {
            let p = part.trim().trim_matches('"').trim_matches('\'');
            if !p.is_empty() {
                out.push(p.to_string());
            }
        }
    }
}

fn collect_js_files_opt(dir: &Path, limit: usize, recursive: bool, out: &mut Vec<PathBuf>) {
    if out.len() >= limit {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if out.len() >= limit {
            break;
        }
        if p.is_dir() {
            if recursive {
                collect_js_files_opt(&p, limit, true, out);
            }
        } else if p.extension().and_then(|e| e.to_str()) == Some("js") {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.contains("_FIXTURE") {
                out.push(p);
            }
        }
    }
}

fn format_js_error(rt: &mut JsRuntime, err: &JsError) -> String {
    match err {
        JsError::Native { kind, message, .. } => format!("{}: {}", kind.name(), message),
        JsError::Thrown(JsValue::Object(oid)) => {
            let name = rt
                .get_property(*oid, "name")
                .ok()
                .and_then(|v| rt.to_string(&v).ok())
                .unwrap_or_else(|| "Object".to_string());
            let msg = rt
                .get_property(*oid, "message")
                .ok()
                .and_then(|v| rt.to_string(&v).ok())
                .unwrap_or_default();
            if msg.is_empty() {
                name
            } else {
                format!("{}: {}", name, msg)
            }
        }
        JsError::Thrown(v) => format!("Uncaught {:?}", v),
    }
}

fn make_host_262(rt: &mut JsRuntime, realm_id: usize, hub: Arc<AgentHub>) -> peedle_js::ObjectId {
    let host_262 = rt.create_object();
    let realm_global = rt.realms[realm_id].global_object;
    let _ = rt.set_property(host_262, "global", JsValue::Object(realm_global));
    let detach_fn = rt.create_native_function(
        "detachArrayBuffer",
        1,
        Rc::new(|rt, _, args| {
            if let Some(JsValue::Object(oid)) = args.first() {
                if let peedle_js::ObjectKind::ArrayBuffer(ref mut ab) =
                    rt.heap.get_obj_mut(*oid).kind
                {
                    ab.bytes.clear();
                    ab.detached = true;
                }
            }
            Ok(JsValue::Undefined)
        }),
    );
    let eval_script_fn = rt.create_native_function(
        "evalScript",
        1,
        Rc::new(move |rt, _, args| {
            let src = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            rt.eval_in_realm(realm_id, &src)
        }),
    );
    let hub_realm = hub.clone();
    let create_realm_fn = rt.create_native_function(
        "createRealm",
        0,
        Rc::new(move |rt, _, _| {
            let new_id = rt.create_realm();
            let prev = rt.switch_realm(new_id);
            let h = make_host_262(rt, new_id, hub_realm.clone());
            let rg = rt.global_object();
            let _ = rt.set_property(rg, "$262", JsValue::Object(h));
            rt.switch_realm(prev);
            Ok(JsValue::Object(h))
        }),
    );
    let agent = make_agent_object(rt, hub);
    let ams = rt.abstract_module_source_ctor();
    let _ = rt.set_property(host_262, "agent", JsValue::Object(agent));
    let _ = rt.set_property(host_262, "AbstractModuleSource", JsValue::Object(ams));
    let _ = rt.set_property(host_262, "detachArrayBuffer", JsValue::Object(detach_fn));
    let _ = rt.set_property(host_262, "evalScript", JsValue::Object(eval_script_fn));
    let _ = rt.set_property(host_262, "createRealm", JsValue::Object(create_realm_fn));
    host_262
}

fn run_single_test(
    test_path: &Path,
    src: &str,
    meta: &TestMeta,
    harness_cache: &HashMap<String, String>,
) -> Result<(), String> {
    let mut rt = JsRuntime::new();
    rt.max_steps = Some(100_000_000);
    rt.max_call_depth = 128;
    rt.can_block = !meta.flags.iter().any(|f| f == "CanBlockIsFalse");

    let base_dir = test_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let base_dir_bytes = base_dir.clone();
    rt.set_module_loader(move |specifier: &str| {
        let candidate = if Path::new(specifier).is_absolute() {
            PathBuf::from(specifier)
        } else {
            base_dir.join(specifier)
        };
        std::fs::read_to_string(&candidate)
            .map_err(|_| JsError::error(format!("Cannot load module '{}'", specifier)))
    });
    rt.set_module_bytes_loader(move |specifier: &str| {
        let candidate = if Path::new(specifier).is_absolute() {
            PathBuf::from(specifier)
        } else {
            base_dir_bytes.join(specifier)
        };
        std::fs::read(&candidate)
            .map_err(|_| JsError::error(format!("Cannot load module '{}'", specifier)))
    });

    let is_async = meta.flags.iter().any(|f| f == "async");
    let is_module = meta.flags.iter().any(|f| f == "module");
    let is_raw = meta.flags.iter().any(|f| f == "raw");
    let only_strict = meta.flags.iter().any(|f| f == "onlyStrict");

    let print_log = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let log_clone = print_log.clone();
    let print_fn = rt.create_native_function(
        "print",
        1,
        Rc::new(move |rt, _, args| {
            let s = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            log_clone.borrow_mut().push(s);
            Ok(JsValue::Undefined)
        }),
    );
    let g = rt.global_object();
    let _ = rt.set_property(g, "print", JsValue::Object(print_fn));

    let hub = AgentHub::new();
    let _agent_guard = AgentGuard(hub.clone());
    let timers = Rc::new(RefCell::new(Vec::<(Instant, JsValue)>::new()));
    if meta.includes.iter().any(|inc| inc == "atomicsHelper.js") {
        install_set_timeout(&mut rt, timers.clone());
    }

    let host_262 = make_host_262(&mut rt, 0, hub.clone());
    let _ = rt.set_property(g, "$262", JsValue::Object(host_262));

    if !is_raw {
        if let Some(sta) = harness_cache.get("sta.js") {
            let _ = rt.eval(sta);
        }
        if let Some(assert_js) = harness_cache.get("assert.js") {
            let _ = rt.eval(assert_js);
        }
        if is_async {
            if let Some(dh) = harness_cache.get("doneprintHandle.js") {
                let _ = rt.eval(dh);
            }
        }
        for inc in &meta.includes {
            if let Some(code) = harness_cache.get(inc) {
                if let Err(e) = rt.eval(code) {
                    return Err(format!("harness {} failed: {}", inc, format_js_error(&mut rt, &e)));
                }
            }
        }
    }

    let test_src = if only_strict && !is_raw && !is_module {
        format!("\"use strict\";\n{}", src)
    } else {
        src.to_string()
    };

    let result = if is_module {
        let mod_name = test_path
            .to_str()
            .unwrap_or("test.mjs");
        rt.eval_module(mod_name, &test_src)
    } else {
        rt.eval(&test_src)
    };

    if result.is_ok() && (is_async || rt.has_pending_async_waiters() || !timers.borrow().is_empty()) {
        pump_event_loop(
            &mut rt,
            &timers,
            if is_async { Some(&print_log) } else { None },
            &hub,
            Duration::from_secs(15),
        );
    }

    if let Some(ref expected_type) = meta.negative_type {
        return match result {
            Ok(_) => Err(format!("Expected {} error but succeeded", expected_type)),
            Err(e) => {
                if error_matches_type(&mut rt, &e, expected_type) {
                    Ok(())
                } else {
                    Err(format!(
                        "Expected {} but got {}",
                        expected_type,
                        format_js_error(&mut rt, &e)
                    ))
                }
            }
        };
    }

    match result {
        Ok(_) => {
            if is_async {
                let lines = print_log.borrow();
                if lines.iter().any(|l| l.contains("Test262:AsyncTestComplete")) {
                    Ok(())
                } else if let Some(fail) = lines.iter().find(|l| l.contains("Test262:AsyncTestFailure")) {
                    Err(fail.clone())
                } else {
                    Err("Async test did not complete".into())
                }
            } else {
                Ok(())
            }
        }
        Err(e) => Err(format_js_error(&mut rt, &e)),
    }
}

fn error_matches_type(rt: &mut JsRuntime, err: &JsError, expected: &str) -> bool {
    match err {
        JsError::Native { kind, .. } => {
            kind.name() == expected || expected == "Error"
        }
        JsError::Thrown(JsValue::Object(oid)) => {
            if let Ok(name_v) = rt.get_property(*oid, "name") {
                if !name_v.is_undefined() {
                    if let Ok(name) = rt.to_string(&name_v) {
                        if name == expected {
                            return true;
                        }
                    }
                }
            }
            if let Ok(JsValue::Object(ctor)) = rt.get_property(*oid, "constructor") {
                if let Ok(name_v) = rt.get_property(ctor, "name") {
                    if let Ok(name) = rt.to_string(&name_v) {
                        if name == expected {
                            return true;
                        }
                    }
                }
            }
            expected == "Error"
        }
        JsError::Thrown(_) => false,
    }
}

#[test]
fn test262_broad_suite() {
    let root = test262_root();
    let harness_dir = root.join("harness");
    let mut harness_cache = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(&harness_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("js") {
                if let (Some(name), Ok(content)) = (
                    p.file_name().and_then(|n| n.to_str()),
                    std::fs::read_to_string(&p),
                ) {
                    harness_cache.insert(name.to_string(), content);
                }
            }
        }
    }

    let suites: &[(&str, usize)] = &[
        ("language/types", 200),
        ("language/literals", 600),
        ("language/identifiers", 300),
        ("language/identifier-resolution", 50),
        ("language/white-space", 100),
        ("language/line-terminators", 100),
        ("language/comments", 100),
        ("language/asi", 150),
        ("language/keywords", 50),
        ("language/reserved-words", 50),
        ("language/future-reserved-words", 100),
        ("language/punctuators", 50),
        ("language/directive-prologue", 100),
        ("language/source-text", 50),
        ("language/statementList", 100),
        ("language/global-code", 100),
        ("language/module-code", 700),
        ("language/import", 200),
        ("language/export", 20),
        ("language/function-code", 300),
        ("language/arguments-object", 350),
        ("language/eval-code", 400),
        ("language/block-scope", 200),
        ("language/computed-property-names", 100),
        ("language/destructuring", 50),
        ("language/rest-parameters", 50),
        ("language/expressions@flat", 20),
        ("language/expressions/addition", 60),
        ("language/expressions/array", 60),
        ("language/expressions/arrow-function", 400),
        ("language/expressions/assignment", 550),
        ("language/expressions/assignmenttargettype", 400),
        ("language/expressions/async-arrow-function", 100),
        ("language/expressions/async-function", 150),
        ("language/expressions/async-generator", 1000),
        ("language/expressions/await", 50),
        ("language/expressions/bitwise-and", 50),
        ("language/expressions/bitwise-not", 50),
        ("language/expressions/bitwise-or", 50),
        ("language/expressions/bitwise-xor", 50),
        ("language/expressions/call", 150),
        ("language/expressions/class", 5000),
        ("language/expressions/coalesce", 50),
        ("language/expressions/comma", 20),
        ("language/expressions/compound-assignment", 500),
        ("language/expressions/concatenation", 20),
        ("language/expressions/conditional", 50),
        ("language/expressions/delete", 100),
        ("language/expressions/division", 60),
        ("language/expressions/does-not-equals", 50),
        ("language/expressions/dynamic-import", 1200),
        ("language/expressions/equals", 60),
        ("language/expressions/exponentiation", 60),
        ("language/expressions/function", 350),
        ("language/expressions/generators", 350),
        ("language/expressions/greater-than", 60),
        ("language/expressions/greater-than-or-equal", 60),
        ("language/expressions/grouping", 20),
        ("language/expressions/import.meta", 50),
        ("language/expressions/in", 50),
        ("language/expressions/instanceof", 60),
        ("language/expressions/left-shift", 60),
        ("language/expressions/less-than", 60),
        ("language/expressions/less-than-or-equal", 60),
        ("language/expressions/logical-and", 30),
        ("language/expressions/logical-assignment", 100),
        ("language/expressions/logical-not", 30),
        ("language/expressions/logical-or", 30),
        ("language/expressions/member-expression", 20),
        ("language/expressions/modulus", 50),
        ("language/expressions/multiplication", 50),
        ("language/expressions/new", 100),
        ("language/expressions/new.target", 30),
        ("language/expressions/object", 1500),
        ("language/expressions/optional-chaining", 50),
        ("language/expressions/postfix-decrement", 50),
        ("language/expressions/postfix-increment", 50),
        ("language/expressions/prefix-decrement", 50),
        ("language/expressions/prefix-increment", 50),
        ("language/expressions/property-accessors", 30),
        ("language/expressions/relational", 20),
        ("language/expressions/right-shift", 50),
        ("language/expressions/strict-does-not-equals", 50),
        ("language/expressions/strict-equals", 50),
        ("language/expressions/subtraction", 50),
        ("language/expressions/super", 150),
        ("language/expressions/tagged-template", 50),
        ("language/expressions/template-literal", 100),
        ("language/expressions/this", 20),
        ("language/expressions/typeof", 30),
        ("language/expressions/unary-minus", 30),
        ("language/expressions/unary-plus", 30),
        ("language/expressions/unsigned-right-shift", 60),
        ("language/expressions/void", 20),
        ("language/expressions/yield", 100),
        ("language/statements/async-function", 100),
        ("language/statements/async-generator", 400),
        ("language/statements/await-using", 150),
        ("language/statements/block", 50),
        ("language/statements/break", 50),
        ("language/statements/class", 5000),
        ("language/statements/const", 200),
        ("language/statements/continue", 50),
        ("language/statements/debugger", 10),
        ("language/statements/do-while", 50),
        ("language/statements/empty", 10),
        ("language/statements/expression", 10),
        ("language/statements/for", 450),
        ("language/statements/for-await-of", 1500),
        ("language/statements/for-in", 150),
        ("language/statements/for-of", 1000),
        ("language/statements/function", 500),
        ("language/statements/generators", 350),
        ("language/statements/if", 100),
        ("language/statements/labeled", 50),
        ("language/statements/let", 200),
        ("language/statements/return", 50),
        ("language/statements/switch", 150),
        ("language/statements/throw", 50),
        ("language/statements/try", 250),
        ("language/statements/using", 100),
        ("language/statements/variable", 250),
        ("language/statements/while", 50),
        ("language/statements/with", 250),
        ("built-ins/global", 50),
        ("built-ins/Infinity", 20),
        ("built-ins/NaN", 20),
        ("built-ins/undefined", 20),
        ("built-ins/isFinite", 30),
        ("built-ins/isNaN", 30),
        ("built-ins/parseFloat", 100),
        ("built-ins/parseInt", 100),
        ("built-ins/decodeURI", 100),
        ("built-ins/decodeURIComponent", 100),
        ("built-ins/encodeURI", 50),
        ("built-ins/encodeURIComponent", 50),
        ("built-ins/eval", 50),
        ("built-ins/Object", 4000),
        ("built-ins/Array", 4000),
        ("built-ins/String", 1500),
        ("built-ins/Number", 400),
        ("built-ins/Math", 400),
        ("built-ins/JSON", 200),
        ("built-ins/Map", 250),
        ("built-ins/Set", 450),
        ("built-ins/Iterator@flat", 50),
        ("built-ins/Iterator/from", 50),
        ("built-ins/Iterator/concat", 100),
        ("built-ins/Iterator/zip", 60),
        ("built-ins/Iterator/zipKeyed", 60),
        ("built-ins/Iterator/prototype@flat", 20),
        ("built-ins/Iterator/prototype/Symbol.dispose", 20),
        ("built-ins/Iterator/prototype/Symbol.iterator", 20),
        ("built-ins/Iterator/prototype/Symbol.toStringTag", 20),
        ("built-ins/Iterator/prototype/chunks", 60),
        ("built-ins/Iterator/prototype/constructor", 20),
        ("built-ins/Iterator/prototype/drop", 100),
        ("built-ins/Iterator/prototype/every", 100),
        ("built-ins/Iterator/prototype/filter", 100),
        ("built-ins/Iterator/prototype/find", 100),
        ("built-ins/Iterator/prototype/flatMap", 100),
        ("built-ins/Iterator/prototype/forEach", 100),
        ("built-ins/Iterator/prototype/includes", 60),
        ("built-ins/Iterator/prototype/join", 30),
        ("built-ins/Iterator/prototype/map", 100),
        ("built-ins/Iterator/prototype/reduce", 100),
        ("built-ins/Iterator/prototype/some", 100),
        ("built-ins/Iterator/prototype/take", 100),
        ("built-ins/Iterator/prototype/toArray", 100),
        ("built-ins/Iterator/prototype/windows", 60),
        ("built-ins/Promise", 800),
        ("built-ins/Proxy", 350),
        ("built-ins/Reflect", 200),
        ("built-ins/RegExp@flat", 600),
        ("built-ins/RegExp/prototype", 600),
        ("built-ins/RegExp/CharacterClassEscapes", 50),
        ("built-ins/RegExp/Symbol.species", 20),
        ("built-ins/RegExp/dotall", 20),
        ("built-ins/RegExp/escape", 50),
        ("built-ins/RegExp/lookBehind", 50),
        ("built-ins/RegExp/match-indices", 50),
        ("built-ins/RegExp/named-groups", 100),
        ("built-ins/RegExp/property-escapes@flat", 200),
        ("built-ins/RegExp/regexp-modifiers", 100),
        ("built-ins/RegExp/unicodeSets", 200),
        ("built-ins/TypedArray", 2000),
        ("built-ins/TypedArrayConstructors", 800),
        ("built-ins/Uint8Array", 100),
        ("built-ins/Function", 550),
        ("built-ins/GeneratorFunction", 50),
        ("built-ins/GeneratorPrototype", 100),
        ("built-ins/AsyncFunction", 50),
        ("built-ins/AsyncGeneratorFunction", 50),
        ("built-ins/AsyncGeneratorPrototype", 100),
        ("built-ins/AsyncIteratorPrototype", 50),
        ("built-ins/AsyncFromSyncIteratorPrototype", 50),
        ("built-ins/ArrayIteratorPrototype", 50),
        ("built-ins/StringIteratorPrototype", 20),
        ("built-ins/MapIteratorPrototype", 20),
        ("built-ins/SetIteratorPrototype", 20),
        ("built-ins/RegExpStringIteratorPrototype", 30),
        ("built-ins/Symbol", 150),
        ("built-ins/Error", 150),
        ("built-ins/NativeErrors", 150),
        ("built-ins/AggregateError", 50),
        ("built-ins/SuppressedError", 50),
        ("built-ins/DisposableStack", 150),
        ("built-ins/AsyncDisposableStack", 150),
        ("built-ins/ThrowTypeError", 30),
        ("built-ins/ArrayBuffer", 300),
        ("built-ins/SharedArrayBuffer", 150),
        ("built-ins/Atomics", 500),
        ("built-ins/DataView", 650),
        ("built-ins/Date", 650),
        ("built-ins/BigInt", 100),
        ("built-ins/Boolean", 100),
        ("built-ins/WeakMap", 150),
        ("built-ins/WeakSet", 100),
        ("built-ins/WeakRef", 50),
        ("built-ins/FinalizationRegistry", 100),
        ("built-ins/AbstractModuleSource", 20),
        ("built-ins/ShadowRealm", 100),
    ];

    let filter = std::env::var("TEST262_FILTER").ok();
    let trace = std::env::var("TEST262_TRACE").is_ok();
    let mut suite_files: Vec<(&str, Vec<PathBuf>)> = Vec::new();
    for &(suite, limit) in suites {
        if let Some(ref flt) = filter {
            if !suite.contains(flt.as_str()) {
                continue;
            }
        }
        let (rel_dir, recursive) = if let Some(base) = suite.strip_suffix("@flat") {
            (base, false)
        } else {
            (suite, true)
        };
        let dir = root.join("test").join(rel_dir);
        let mut files = Vec::new();
        collect_js_files_opt(&dir, limit, recursive, &mut files);
        suite_files.push((suite, files));
    }

    let workers = std::thread::available_parallelism()
        .map(|n| n.get().clamp(4, 16))
        .unwrap_or(4);
    let mut flat_jobs: Vec<(usize, PathBuf)> = Vec::new();
    for (s_idx, (_, files)) in suite_files.iter().enumerate() {
        for f in files {
            flat_jobs.push((s_idx, f.clone()));
        }
    }
    let next_job = std::sync::atomic::AtomicUsize::new(0);
    let suite_count = suite_files.len();

    let worker_results: Vec<Vec<(usize, usize, Vec<(String, String, String)>)>> =
        std::thread::scope(|s| {
            let mut handles = Vec::new();
            for _ in 0..workers {
                let jobs = &flat_jobs;
                let counter = &next_job;
                let hc = &harness_cache;
                let r = &root;
                handles.push(s.spawn(move || {
                    let mut per_suite: Vec<(usize, usize, Vec<(String, String, String)>)> =
                        (0..suite_count).map(|_| (0, 0, Vec::new())).collect();
                    loop {
                        let idx = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if idx >= jobs.len() {
                            break;
                        }
                        let (s_idx, ref f) = jobs[idx];
                        let Ok(src) = std::fs::read_to_string(f) else {
                            continue;
                        };
                        let meta = parse_meta(&src);
                        if trace {
                            eprintln!("RUN {}", f.display());
                        }
                        match run_single_test(f, &src, &meta, hc) {
                            Ok(()) => per_suite[s_idx].0 += 1,
                            Err(msg) => {
                                per_suite[s_idx].1 += 1;
                                let rel = f.strip_prefix(r).unwrap_or(f);
                                per_suite[s_idx].2.push((
                                    rel.display().to_string(),
                                    meta.features.join(","),
                                    msg,
                                ));
                            }
                        }
                    }
                    per_suite
                }));
            }
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

    let mut total_pass = 0usize;
    let mut total_fail = 0usize;
    let mut sample_failures: Vec<String> = Vec::new();
    let mut dump_lines: Vec<String> = Vec::new();

    for (s_idx, (suite, _)) in suite_files.iter().enumerate() {
        let mut suite_pass = 0usize;
        let mut suite_fail = 0usize;
        let mut suite_failures: Vec<(String, String, String)> = Vec::new();
        for wr in &worker_results {
            suite_pass += wr[s_idx].0;
            suite_fail += wr[s_idx].1;
            suite_failures.extend(wr[s_idx].2.iter().cloned());
        }
        suite_failures.sort_by(|a, b| a.0.cmp(&b.0));
        total_pass += suite_pass;
        total_fail += suite_fail;
        let total = suite_pass + suite_fail;
        let pct = if total > 0 {
            (suite_pass as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        eprintln!(
            "test262 {:24} {:4}/{:4} ({:5.1}%)",
            suite, suite_pass, total, pct
        );
        for (i, (rel, feats, msg)) in suite_failures.iter().enumerate() {
            if i < 3 {
                sample_failures.push(format!("{} -> {}", rel, msg));
            }
            dump_lines.push(format!(
                "{} | {} | [{}] | {}",
                suite,
                rel,
                feats,
                msg.replace('\n', " ")
            ));
        }
    }

    if let Ok(dump_path) = std::env::var("TEST262_DUMP") {
        let _ = std::fs::write(&dump_path, dump_lines.join("\n"));
    }

    let grand = total_pass + total_fail;
    let grand_pct = if grand > 0 {
        (total_pass as f64 / grand as f64) * 100.0
    } else {
        0.0
    };
    eprintln!(
        "TEST262 TOTAL: {}/{} ({:.2}%)",
        total_pass, grand, grand_pct
    );
    for s in sample_failures.iter().take(30) {
        eprintln!("  FAIL: {}", s);
    }
    if filter.is_none() {
        assert!(
            grand >= 42_600,
            "Expected at least 42600 Test262 tests run, got {}",
            grand
        );
        assert!(
            total_pass >= 42_500,
            "Expected at least 42500 Test262 tests passing, got {}/{}",
            total_pass,
            grand
        );
        assert!(
            grand_pct >= 99.7,
            "Test262 pass rate too low: {:.2}% ({}/{})",
            grand_pct,
            total_pass,
            grand
        );
    }
}
