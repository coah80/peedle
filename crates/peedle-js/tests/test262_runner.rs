use peedle_js::{JsError, JsRuntime, JsValue};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn test262_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/test262")
}

#[derive(Default)]
struct TestMeta {
    includes: Vec<String>,
    flags: Vec<String>,
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
    for line in yaml.lines() {
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

fn collect_js_files(dir: &Path, limit: usize, out: &mut Vec<PathBuf>) {
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
            collect_js_files(&p, limit, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("js") {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.contains("_FIXTURE") {
                out.push(p);
            }
        }
    }
}

fn run_single_test(
    src: &str,
    meta: &TestMeta,
    harness_cache: &HashMap<String, String>,
) -> Result<(), String> {
    let mut rt = JsRuntime::new();
    rt.max_steps = Some(500_000);
    rt.max_call_depth = 128;

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

    let host_262 = rt.create_object();
    let detach_fn = rt.create_native_function(
        "detachArrayBuffer",
        1,
        Rc::new(|rt, _, args| {
            if let Some(JsValue::Object(oid)) = args.first() {
                if let peedle_js::ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(*oid).kind {
                    ab.bytes.clear();
                    ab.detached = true;
                }
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.set_property(host_262, "detachArrayBuffer", JsValue::Object(detach_fn));
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
                    return Err(format!("harness {} failed: {}", inc, e));
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
        rt.eval_module("test.mjs", &test_src)
    } else {
        rt.eval(&test_src)
    };

    if let Some(ref expected_type) = meta.negative_type {
        return match result {
            Ok(_) => Err(format!("Expected {} error but succeeded", expected_type)),
            Err(e) => {
                if error_matches_type(&mut rt, &e, expected_type) {
                    Ok(())
                } else {
                    Err(format!("Expected {} but got {}", expected_type, e))
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
        Err(e) => Err(format!("{}", e)),
    }
}

fn error_matches_type(rt: &mut JsRuntime, err: &JsError, expected: &str) -> bool {
    match err {
        JsError::Native { kind, .. } => {
            kind.name() == expected || expected == "Error"
        }
        JsError::Thrown(JsValue::Object(oid)) => {
            if let Ok(name_v) = rt.get_property(*oid, "name") {
                if let Ok(name) = rt.to_string(&name_v) {
                    if name == expected {
                        return true;
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

    let suites: [(&str, usize); 16] = [
        ("language/types", 200),
        ("language/expressions", 300),
        ("language/statements", 300),
        ("built-ins/Object", 250),
        ("built-ins/Array", 250),
        ("built-ins/String", 250),
        ("built-ins/Number", 150),
        ("built-ins/Math", 150),
        ("built-ins/JSON", 150),
        ("built-ins/Map", 150),
        ("built-ins/Set", 150),
        ("built-ins/Promise", 150),
        ("built-ins/Proxy", 150),
        ("built-ins/Reflect", 150),
        ("built-ins/RegExp", 150),
        ("built-ins/TypedArray", 150),
    ];

    let mut total_pass = 0usize;
    let mut total_fail = 0usize;
    let mut sample_failures: Vec<String> = Vec::new();

    for (suite, limit) in suites {
        let dir = root.join("test").join(suite);
        let mut files = Vec::new();
        collect_js_files(&dir, limit, &mut files);
        let mut pass = 0usize;
        let mut fail = 0usize;
        let mut suite_samples = 0usize;

        for f in files {
            let Ok(src) = std::fs::read_to_string(&f) else {
                continue;
            };
            let meta = parse_meta(&src);
            if meta.flags.iter().any(|fl| fl == "module") {
                continue;
            }
            match run_single_test(&src, &meta, &harness_cache) {
                Ok(()) => pass += 1,
                Err(msg) => {
                    fail += 1;
                    if suite_samples < 3 {
                        suite_samples += 1;
                        let rel = f.strip_prefix(&root).unwrap_or(&f);
                        sample_failures.push(format!("{} -> {}", rel.display(), msg));
                    }
                }
            }
        }
        total_pass += pass;
        total_fail += fail;
        let total = pass + fail;
        let pct = if total > 0 {
            (pass as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        eprintln!(
            "test262 {:24} {:4}/{:4} ({:5.1}%)",
            suite, pass, total, pct
        );
    }

    let grand = total_pass + total_fail;
    let grand_pct = if grand > 0 {
        (total_pass as f64 / grand as f64) * 100.0
    } else {
        0.0
    };
    eprintln!(
        "TEST262 TOTAL: {}/{} ({:.1}%)",
        total_pass, grand, grand_pct
    );
    for s in sample_failures.iter().take(30) {
        eprintln!("  FAIL: {}", s);
    }
    assert!(grand >= 2000, "Expected at least 2000 Test262 tests run, got {}", grand);
    assert!(
        grand_pct >= 60.0,
        "Test262 pass rate too low: {:.1}% ({}/{})",
        grand_pct,
        total_pass,
        grand
    );
}
