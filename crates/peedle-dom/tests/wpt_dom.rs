use peedle_dom::BrowserJsEnv;
use peedle_html::parse_html;
use peedle_net::HttpClient;

fn run_env(html: &str, js: &str) -> String {
    let doc = parse_html(html);
    let mut env = BrowserJsEnv::new(
        doc,
        "https://wpt.test/index.html".to_string(),
        800.0,
        600.0,
        HttpClient::new(),
    );
    let val = env.eval(js).unwrap_or_else(|e| panic!("JS error: {}", e));
    env.rt.to_string(&val).unwrap_or_default()
}

#[test]
fn wpt_event_target_phases_and_propagation() {
    let res = run_env(
        "<div id='outer'><div id='inner'><button id='btn'>Go</button></div></div>",
        r#"
            const log = [];
            const outer = document.getElementById('outer');
            const inner = document.getElementById('inner');
            const btn = document.getElementById('btn');
            outer.addEventListener('ping', e => log.push('outer-cap:' + e.eventPhase), true);
            inner.addEventListener('ping', e => log.push('inner-cap:' + e.eventPhase), true);
            btn.addEventListener('ping', e => { log.push('btn:' + e.eventPhase + ':' + e.detail.n); e.preventDefault(); });
            inner.addEventListener('ping', e => { log.push('inner-bub:' + e.eventPhase); e.stopPropagation(); });
            outer.addEventListener('ping', e => log.push('outer-bub'));
            const ev = new CustomEvent('ping', { bubbles: true, cancelable: true, detail: { n: 42 } });
            const notCanceled = btn.dispatchEvent(ev);
            log.push('default:' + notCanceled + ':' + ev.defaultPrevented);
            log.join('|');
        "#,
    );
    assert_eq!(
        res,
        "outer-cap:1|inner-cap:1|btn:2:42|inner-bub:3|default:false:true"
    );
}

#[test]
fn wpt_mutation_observer_and_shadow_dom() {
    let res = run_env(
        "<div id='host'></div><ul id='list'></ul>",
        r#"
            const host = document.getElementById('host');
            const shadow = host.attachShadow({ mode: 'open' });
            const badge = document.createElement('span');
            badge.id = 'inner-badge';
            badge.textContent = 'ShadowOK';
            shadow.appendChild(badge);
            const foundOutside = document.getElementById('inner-badge');
            const foundInside = shadow.querySelector('#inner-badge').textContent;

            const records = [];
            const list = document.getElementById('list');
            const mo = new MutationObserver((listRecs) => {
                for (const r of listRecs) records.push(r.type);
            });
            mo.observe(list, { childList: true, attributes: true, subtree: true });
            const li = document.createElement('li');
            li.textContent = 'item';
            list.appendChild(li);
            list.setAttribute('data-k', 'v');
            const taken = mo.takeRecords().map(r => r.type);
            mo.disconnect();
            String(foundOutside === null) + ':' + foundInside + ':' + taken.join(',');
        "#,
    );
    assert_eq!(res, "true:ShadowOK:childList,attributes");
}

#[test]
fn wpt_range_and_tree_walker_and_node_iterator() {
    let res = run_env(
        "<div id='box'><span>Alpha</span><b>Beta</b><i>Gamma</i></div>",
        r#"
            const box = document.getElementById('box');
            const r = document.createRange();
            r.setStart(box.firstChild.firstChild, 2);
            r.setEnd(box.lastChild.firstChild, 3);
            const str = r.toString();
            const cloned = r.cloneContents();
            const tw = document.createTreeWalker(box, 1, null);
            const tags = [];
            while (tw.nextNode()) tags.push(tw.currentNode.tagName);

            const ni = document.createNodeIterator(box, 4, null);
            const texts = [];
            let cur;
            while ((cur = ni.nextNode())) texts.push(cur.nodeValue);
            str + '|' + cloned.childNodes.length + '|' + tags.join(',') + '|' + texts.join(',');
        "#,
    );
    assert_eq!(res, "phaBetaGam|3|SPAN,B,I|Alpha,Beta,Gamma");
}

#[test]
fn wpt_url_formdata_headers_abort_and_webcrypto_fips180() {
    let doc = parse_html("<div></div>");
    let mut env = BrowserJsEnv::new(
        doc,
        "https://wpt.test/dir/page.html".to_string(),
        800.0,
        600.0,
        HttpClient::new(),
    );
    let _ = env
        .eval(
            r#"
            const u = new URL('../api/items?b=2&a=1#frag', 'https://wpt.test/dir/page.html');
            u.searchParams.sort();
            u.searchParams.append('c', 'hello world');

            const fd = new FormData();
            fd.append('k1', 'v1');
            fd.append('k1', 'v2');

            const h = new Headers({ 'X-Custom': 'one' });
            h.append('x-custom', 'two');

            const ac = new AbortController();
            let aborted = false;
            ac.signal.addEventListener('abort', () => { aborted = true; });
            ac.abort('done');

            const enc = new TextEncoder().encode('abc');
            const dec = new TextDecoder().decode(enc);
            const uuid = crypto.randomUUID();
            const rand = new Uint8Array(8);
            crypto.getRandomValues(rand);

            window.__out = '';
            Promise.all([
                crypto.subtle.digest('SHA-1', enc),
                crypto.subtle.digest('SHA-256', enc),
            ]).then(([s1, s256]) => {
                const hex = buf => Array.from(new Uint8Array(buf)).map(b => b.toString(16).padStart(2, '0')).join('');
                window.__out = [
                    u.pathname,
                    u.searchParams.toString(),
                    fd.getAll('k1').join('+'),
                    h.get('X-Custom'),
                    String(aborted) + ':' + String(ac.signal.aborted),
                    dec,
                    String(uuid.length === 36),
                    hex(s1),
                    hex(s256),
                ].join('|');
            });
            "#,
        )
        .unwrap();
    env.advance_time_ms(50);
    let out = env.eval("window.__out").unwrap();
    let s = env.rt.to_string(&out).unwrap_or_default();
    assert_eq!(
        s,
        "/api/items|a=1&b=2&c=hello+world|v1+v2|one, two|true:true|abc|true|a9993e364706816aba3e25717850c26c9cd0d89d|ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
