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
    assert!(pct >= 80.0, "WPT URL pass rate too low: {:.1}%", pct);
}
