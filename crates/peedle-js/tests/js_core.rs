use peedle_js::{JsError, JsRuntime, JsValue, PropertyDescriptor, PropertyKey};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

fn fixture_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn eval_str(src: &str) -> String {
    let mut rt = JsRuntime::new();
    let v = rt.eval(src).unwrap_or_else(|e| panic!("JS error: {}", e));
    rt.to_string(&v).unwrap()
}

fn eval_num(src: &str) -> f64 {
    let mut rt = JsRuntime::new();
    let v = rt.eval(src).unwrap_or_else(|e| panic!("JS error: {}", e));
    rt.to_number(&v).unwrap()
}

fn eval_bool(src: &str) -> bool {
    let mut rt = JsRuntime::new();
    let v = rt.eval(src).unwrap_or_else(|e| panic!("JS error: {}", e));
    rt.to_boolean(&v)
}

#[test]
fn test_arithmetic_operators_and_literals() {
    assert_eq!(eval_num("1 + 2 * 3 - 4 / 2 + 2 ** 3"), 13.0);
    assert_eq!(eval_num("0xff + 0o10 + 0b101 + 1_000"), 1268.0);
    assert_eq!(eval_num("-8 >>> 1"), 2147483644.0);
    assert_eq!(eval_str("String(100n + 23n * 2n)"), "146");
    assert!(eval_bool("null ?? 'ok' === 'ok'"));
    assert_eq!(eval_str("let a = null; a ??= 'set'; a"), "set");
    assert_eq!(eval_num("let b = 0; b ||= 42; b &&= 99; b"), 99.0);
}

#[test]
fn test_closures_hoisting_tdz_and_for_let() {
    assert_eq!(
        eval_str(
            "const fns = [];
             for (let i = 0; i < 4; i++) { fns.push(() => i); }
             fns.map(f => f()).join(',')"
        ),
        "0,1,2,3"
    );
    assert_eq!(eval_num("var x = add(20, 22); function add(a, b) { return a + b; } x"), 42.0);
    let mut rt = JsRuntime::new();
    assert!(rt.eval("{ x; let x = 1; }").is_err());
}

#[test]
fn test_destructuring_spread_rest_and_defaults() {
    assert_eq!(
        eval_str(
            "const { a, b: [x, , y = 10, ...rest], ...more } = { a: 1, b: [2, 3, undefined, 4, 5], c: 6 };
             JSON.stringify({ a, x, y, rest, more })"
        ),
        r#"{"a":1,"x":2,"y":10,"rest":[4,5],"more":{"c":6}}"#
    );
    assert_eq!(
        eval_num("const sum = (a = 1, ...nums) => nums.reduce((s, n) => s + n, a); sum(undefined, 2, 3, 4)"),
        10.0
    );
}

#[test]
fn test_classes_inheritance_private_fields_and_static_blocks() {
    let code = r#"
        class Animal {
            #secret = 10;
            static count = 0;
            static { Animal.count += 1; }
            constructor(name) { this.name = name; }
            speak() { return this.name + ':' + this.#secret; }
            get secret() { return this.#secret; }
            set secret(v) { this.#secret = v; }
        }
        class Dog extends Animal {
            constructor(name, breed) {
                super(name);
                this.breed = breed;
            }
            speak() { return super.speak() + '/' + this.breed; }
        }
        const d = new Dog('Rex', 'Lab');
        d.secret = 42;
        d.speak() + ':' + Animal.count + ':' + (d instanceof Animal) + ':' + (d instanceof Dog);
    "#;
    assert_eq!(eval_str(code), "Rex:42/Lab:1:true:true");
}

#[test]
fn test_optional_chaining_and_templates() {
    assert_eq!(
        eval_str(
            "const o = { a: { b: (x) => x * 2 } };
             [o?.a?.b?.(5), o?.x?.y?.(5), o?.a?.['b'](3)].map(String).join(',')"
        ),
        "10,undefined,6"
    );
    assert_eq!(
        eval_str(
            "function tag(strs, ...vals) { return strs.raw.join('|') + ':' + vals.join(','); }
             tag`hello\\n${1 + 2}world${4}`"
        ),
        r"hello\n|world|:3,4"
    );
}

#[test]
fn test_try_catch_finally_and_labeled_breaks() {
    assert_eq!(
        eval_str(
            "const log = [];
             function f() {
                 try { log.push('try'); return 'from-try'; }
                 finally { log.push('finally'); }
             }
             const r = f();
             log.join(',') + '=' + r"
        ),
        "try,finally=from-try"
    );
    assert_eq!(
        eval_num(
            "let sum = 0;
             outer: for (let i = 0; i < 5; i++) {
                 for (let j = 0; j < 5; j++) {
                     if (j === 2) continue outer;
                     if (i === 3) break outer;
                     sum += 1;
                 }
             }
             sum"
        ),
        6.0
    );
    assert_eq!(eval_str("try { throw new TypeError('boom'); } catch { 'caught' }"), "caught");
}

#[test]
fn test_generators_and_yield_delegate() {
    let code = r#"
        function* inner() { yield 'a'; yield 'b'; return 'done'; }
        function* outer() {
            const r = yield* inner();
            yield r;
            const x = yield 10;
            yield x * 2;
        }
        const g = outer();
        const out = [];
        out.push(g.next().value);
        out.push(g.next().value);
        out.push(g.next().value);
        out.push(g.next().value);
        out.push(g.next(21).value);
        out.push(g.next().done);
        out.join(',');
    "#;
    assert_eq!(eval_str(code), "a,b,done,10,42,true");
}

#[test]
fn test_promises_and_async_await() {
    let code = r#"
        let result = '';
        async function compute(x) {
            const a = await Promise.resolve(x + 1);
            const b = await Promise.all([Promise.resolve(a * 2), Promise.resolve(a * 3)]);
            return b[0] + b[1];
        }
        compute(4).then(v => { result = 'val=' + v; });
    "#;
    let mut rt = JsRuntime::new();
    rt.eval(code).unwrap();
    let res = rt.eval("result").unwrap();
    assert_eq!(rt.to_string(&res).unwrap(), "val=25");

    let code2 = r#"
        let caught = '';
        async function fail() {
            try { await Promise.reject('err42'); }
            catch (e) { caught = 'got:' + e; }
        }
        fail();
    "#;
    rt.eval(code2).unwrap();
    let res2 = rt.eval("caught").unwrap();
    assert_eq!(rt.to_string(&res2).unwrap(), "got:err42");
}

#[test]
fn test_proxy_and_reflect_traps() {
    let code = r#"
        const log = [];
        const target = { a: 1, b: 2 };
        const p = new Proxy(target, {
            get(t, k, r) { log.push('get:' + String(k)); return Reflect.get(t, k, r); },
            set(t, k, v, r) { log.push('set:' + String(k)); return Reflect.set(t, k, v, r); },
            has(t, k) { log.push('has:' + String(k)); return Reflect.has(t, k); },
            deleteProperty(t, k) { log.push('del:' + String(k)); return Reflect.deleteProperty(t, k); },
            ownKeys(t) { log.push('keys'); return Reflect.ownKeys(t); }
        });
        const v = p.a;
        p.c = 3;
        const h = 'b' in p;
        delete p.b;
        const ks = Object.keys(p);
        v + ':' + h + ':' + ks.join(',') + '|' + log.join(',');
    "#;
    assert_eq!(
        eval_str(code),
        "1:true:a,c|get:a,set:c,has:b,del:b,keys"
    );
}

#[test]
fn test_regexp_features() {
    assert_eq!(
        eval_str(
            "const m = /(?<year>\\d{4})-(?<month>\\d{2})/.exec('date: 2026-11');
             m.groups.year + '/' + m.groups.month"
        ),
        "2026/11"
    );
    assert_eq!(
        eval_str("'foo1 foo2 bar3'.match(/(?<=foo)\\d/g).join(',')"),
        "1,2"
    );
    assert_eq!(
        eval_str("'aabbcc'.replace(/(.)\\1/g, '[$1]')"),
        "[a][b][c]"
    );
    assert_eq!(
        eval_str("'hello world'.replace(/(\\w+) (\\w+)/, '$2 $1')"),
        "world hello"
    );
}

#[test]
fn test_collections_typed_arrays_json_and_builtins() {
    assert_eq!(
        eval_str(
            "const m = new Map([['a', 1], ['b', 2]]);
             m.set('c', 3);
             const s = new Set([1, 2, 2, 3]);
             m.size + ':' + m.get('b') + ':' + [...s].join(',')"
        ),
        "3:2:1,2,3"
    );
    assert_eq!(
        eval_str(
            "const buf = new ArrayBuffer(8);
             const dv = new DataView(buf);
             dv.setInt32(0, 0x12345678, false);
             const u8 = new Uint8Array(buf);
             Array.from(u8.slice(0, 4)).map(x => x.toString(16)).join(',')"
        ),
        "12,34,56,78"
    );
    assert_eq!(
        eval_str(
            "const obj = JSON.parse('{\"a\":1,\"b\":[2,3]}', (k, v) => typeof v === 'number' ? v * 10 : v);
             JSON.stringify(obj)"
        ),
        r#"{"a":10,"b":[20,30]}"#
    );
    assert_eq!(
        eval_str("atob(btoa('Hello Peedle!'))"),
        "Hello Peedle!"
    );
    assert_eq!(
        eval_str("new TextDecoder().decode(new TextEncoder().encode('Peedle 🚀'))"),
        "Peedle 🚀"
    );
    assert_eq!(
        eval_str("const a = { x: [1, 2] }; const b = structuredClone(a); b.x.push(3); a.x.length + ':' + b.x.length"),
        "2:3"
    );
}

#[test]
fn test_es_modules() {
    let mut rt = JsRuntime::new();
    rt.set_module_loader(|spec| match spec {
        "./math.js" => Ok("export const pi = 3; export function dbl(x) { return x * 2; } export default 10;".into()),
        _ => Err(JsError::error("not found")),
    });
    let ns = rt
        .eval_module(
            "main.js",
            "import base, { pi, dbl } from './math.js'; export const answer = base + dbl(pi);",
        )
        .unwrap();
    let oid = ns.as_object().unwrap();
    let ans = rt.get_property(oid, "answer").unwrap();
    assert_eq!(rt.to_number(&ans).unwrap(), 16.0);
}

#[test]
fn test_rust_embedding_api_and_gc() {
    struct DomNode {
        tag: String,
    }
    let mut rt = JsRuntime::new();
    let node = rt.create_object();
    rt.set_host_data(node, DomNode { tag: "DIV".into() });
    let getter = rt.create_native_function(
        "getTag",
        0,
        Rc::new(|rt, this, _| {
            let oid = this.as_object().unwrap();
            let tag = rt.get_host_data::<DomNode>(oid).unwrap().tag.clone();
            Ok(JsValue::String(tag))
        }),
    );
    rt.define_property(
        node,
        PropertyKey::from("getTag"),
        PropertyDescriptor::data(JsValue::Object(getter), true, true, true),
    )
    .unwrap();
    let g = rt.global_object();
    rt.set_property(g, "myNode", JsValue::Object(node)).unwrap();
    let res = rt.eval("console.log(myNode.getTag()); myNode.getTag()").unwrap();
    assert_eq!(rt.to_string(&res).unwrap(), "DIV");
    assert_eq!(rt.console_output(), &["DIV".to_string()]);
    rt.eval("for (let i = 0; i < 500; i++) { ({ a: i, b: [1, 2, 3] }); }").unwrap();
    rt.collect_garbage();
    assert_eq!(rt.get_host_data::<DomNode>(node).unwrap().tag, "DIV");
}

#[test]
fn test_lodash_real_library() {
    let path = fixture_path("fixtures/libs/lodash.min.js");
    let src = std::fs::read_to_string(&path).expect("lodash.min.js missing");
    let mut rt = JsRuntime::new();
    let t0 = Instant::now();
    rt.eval(&src).expect("failed to load lodash.min.js");
    let load_ms = t0.elapsed().as_millis();
    eprintln!("lodash.min.js loaded in {} ms", load_ms);

    let chunk = rt.eval("JSON.stringify(_.chunk([1, 2, 3, 4, 5], 2))").unwrap();
    assert_eq!(rt.to_string(&chunk).unwrap(), "[[1,2],[3,4],[5]]");

    let group = rt
        .eval("JSON.stringify(_.groupBy(['one', 'two', 'three'], 'length'))")
        .unwrap();
    assert_eq!(rt.to_string(&group).unwrap(), r#"{"3":["one","two"],"5":["three"]}"#);

    let merged = rt
        .eval("JSON.stringify(_.merge({ a: { b: 1, c: 2 } }, { a: { c: 3, d: 4 } }))")
        .unwrap();
    assert_eq!(rt.to_string(&merged).unwrap(), r#"{"a":{"b":1,"c":3,"d":4}}"#);

    let cloned = rt
        .eval("const o = { a: [{ b: 2 }] }; const c = _.cloneDeep(o); c.a[0].b = 99; o.a[0].b + ':' + c.a[0].b")
        .unwrap();
    assert_eq!(rt.to_string(&cloned).unwrap(), "2:99");
}

#[test]
fn test_marked_real_library() {
    let path = fixture_path("fixtures/libs/marked.min.js");
    let src = std::fs::read_to_string(&path).expect("marked.min.js missing");
    let mut rt = JsRuntime::new();
    let t0 = Instant::now();
    rt.eval(&src).expect("failed to load marked.min.js");
    let load_ms = t0.elapsed().as_millis();
    eprintln!("marked.min.js loaded in {} ms", load_ms);

    let html = rt
        .eval(r##"marked.parse("# Hello **Peedle**") "##)
        .expect("marked.parse failed");
    let html_str = rt.to_string(&html).unwrap();
    eprintln!("marked output: {:?}", html_str);
    assert!(html_str.contains("<h1>Hello <strong>Peedle</strong></h1>"));
}
