use std::path::Path;

use peedle_url::{Url, UrlSearchParams};
use serde_json::Value;

#[test]
fn basic_url_parsing_and_joining() {
    let base = Url::parse("https://Example.COM:443/a/b/c?x=1&y=2#frag").unwrap();
    assert_eq!(base.href(), "https://example.com/a/b/c?x=1&y=2#frag");
    assert_eq!(base.origin(), "https://example.com");
    assert_eq!(base.pathname(), "/a/b/c");
    assert_eq!(base.search(), "?x=1&y=2");
    assert_eq!(base.hash(), "#frag");

    let rel = base.join("../d/./e?z=3").unwrap();
    assert_eq!(rel.href(), "https://example.com/a/d/e?z=3");

    let mut params = UrlSearchParams::parse("?b=2&a=1&b=3");
    assert_eq!(params.get("a"), Some("1"));
    assert_eq!(params.get_all("b"), vec!["2", "3"]);
    params.sort();
    assert_eq!(params.serialize(), "a=1&b=2&b=3");
}

#[test]
fn wpt_urltestdata_conformance() {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/wpt/urltestdata.json");
    if !fixture_path.exists() {
        return;
    }
    let raw = std::fs::read_to_string(&fixture_path).unwrap();
    let entries: Vec<Value> = serde_json::from_str(&raw).unwrap();

    let mut total = 0usize;
    let mut passed = 0usize;
    let mut failures = Vec::new();

    for entry in entries {
        if entry.is_string() {
            continue;
        }
        let Some(input) = entry.get("input").and_then(|v| v.as_str()) else {
            continue;
        };
        let base_str = entry.get("base").and_then(|v| v.as_str());
        let expected_failure = entry
            .get("failure")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let base_url = match base_str {
            Some(b) => match Url::parse(b) {
                Ok(u) => Some(u),
                Err(_) => {
                    if expected_failure {
                        total += 1;
                        passed += 1;
                        continue;
                    }
                    None
                }
            },
            None => None,
        };

        total += 1;
        let result = Url::parse_with_base(input, base_url.as_ref());

        if expected_failure {
            if result.is_err() {
                passed += 1;
            } else if failures.len() < 15 {
                failures.push(format!(
                    "expected fail: input={:?} base={:?} got={}",
                    input,
                    base_str,
                    result.unwrap().href()
                ));
            }
            continue;
        }

        match result {
            Ok(url) => {
                let exp_href = entry.get("href").and_then(|v| v.as_str()).unwrap_or("");
                let exp_origin = entry.get("origin").and_then(|v| v.as_str());
                let href_ok = url.href() == exp_href;
                let origin_ok = exp_origin.map_or(true, |o| url.origin() == o);
                if href_ok && origin_ok {
                    passed += 1;
                } else if failures.len() < 15 {
                    failures.push(format!(
                        "mismatch: input={:?} base={:?} got={} exp={}",
                        input,
                        base_str,
                        url.href(),
                        exp_href
                    ));
                }
            }
            Err(e) => {
                if failures.len() < 15 {
                    failures.push(format!(
                        "unexpected err: input={:?} base={:?} err={:?}",
                        input, base_str, e
                    ));
                }
            }
        }
    }

    let pct = (passed as f64 / total.max(1) as f64) * 100.0;
    eprintln!("WPT urltestdata: {}/{} ({:.1}%)", passed, total, pct);
    for f in &failures {
        eprintln!("  {}", f);
    }
    assert_eq!(passed, total, "WPT URL pass rate regressed: {}/{}", passed, total);
}

#[test]
fn wpt_url_setters_conformance() {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/wpt/setters_tests.json");
    if !fixture_path.exists() {
        return;
    }
    let raw = std::fs::read_to_string(&fixture_path).unwrap();
    let root: Value = serde_json::from_str(&raw).unwrap();
    let obj = root.as_object().unwrap();

    let mut total = 0usize;
    let mut passed = 0usize;

    for (prop, cases_val) in obj {
        if prop == "comment" {
            continue;
        }
        let Some(cases) = cases_val.as_array() else {
            continue;
        };
        for tc in cases {
            let Some(href) = tc.get("href").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(new_val) = tc.get("new_value").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(expected) = tc.get("expected").and_then(|v| v.as_object()) else {
                continue;
            };
            let Ok(mut url) = Url::parse(href) else {
                continue;
            };
            total += 1;
            match prop.as_str() {
                "protocol" => url.set_protocol(new_val),
                "username" => url.set_username(new_val),
                "password" => url.set_password(new_val),
                "host" => url.set_host(new_val),
                "hostname" => url.set_hostname(new_val),
                "port" => url.set_port(new_val),
                "pathname" => url.set_pathname(new_val),
                "search" => url.set_search(new_val),
                "hash" => url.set_hash(new_val),
                "href" => {
                    let _ = url.set_href(new_val);
                }
                _ => continue,
            }
            let mut ok = true;
            for (ek, ev) in expected {
                let Some(exp_str) = ev.as_str() else {
                    continue;
                };
                let got_str = match ek.as_str() {
                    "href" => url.href(),
                    "protocol" => url.protocol(),
                    "username" => url.username.clone(),
                    "password" => url.password.clone(),
                    "host" => url.host_str(),
                    "hostname" => url.hostname(),
                    "port" => url.port_str(),
                    "pathname" => url.pathname(),
                    "search" => url.search(),
                    "hash" => url.hash(),
                    _ => continue,
                };
                if got_str != exp_str {
                    ok = false;
                    break;
                }
            }
            if ok {
                passed += 1;
            }
        }
    }

    let pct = (passed as f64 / total.max(1) as f64) * 100.0;
    eprintln!("WPT url setters: {}/{} ({:.1}%)", passed, total, pct);
    assert!(pct >= 80.0, "WPT URL setters pass rate too low: {:.1}%", pct);
}
