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

#[test]
fn wpt_intl_number_datetime_collator_plural() {
    let res = run_env(
        "<div></div>",
        r#"
            const nf = new Intl.NumberFormat('en-US', { minimumFractionDigits: 1, maximumFractionDigits: 2 });
            const pct = new Intl.NumberFormat('en-US', { style: 'percent' });
            const dtf = new Intl.DateTimeFormat('en-US');
            const col = new Intl.Collator('en-US');
            const pr = new Intl.PluralRules('en-US');
            [
                nf.format(12345.6),
                pct.format(0.75),
                dtf.format(0),
                col.compare('a', 'b'),
                pr.select(1),
                pr.select(2),
            ].join('|');
        "#,
    );
    assert_eq!(res, "12,345.6|75%|1970-01-01|-1|one|other");
}

#[test]
fn wpt_domparser_xmlserializer_and_template_contents() {
    let res = run_env(
        "<div id='host'></div>",
        r#"
            const dp = new DOMParser();
            const parsed = dp.parseFromString('<section><template id="t"><p class="hi">Hello</p></template></section>', 'text/html');
            const tpl = parsed.querySelector('#t');
            const cloned = tpl.content.cloneNode(true);
            document.getElementById('host').appendChild(cloned);
            const xs = new XMLSerializer();
            const ser = xs.serializeToString(document.querySelector('#host p'));
            document.querySelector('#host p.hi').textContent + '|' + ser.includes('Hello');
        "#,
    );
    assert_eq!(res, "Hello|true");
}

#[test]
fn wpt_canvas2d_paths_transforms_and_text_alignment() {
    let doc = parse_html("<canvas id='c' width='120' height='80'></canvas>");
    let mut env = BrowserJsEnv::new(
        doc,
        "https://wpt.test/canvas.html".to_string(),
        800.0,
        600.0,
        HttpClient::new(),
    );
    let out = env
        .eval(
            r#"
            const cv = document.getElementById('c');
            const ctx = cv.getContext('2d');
            ctx.fillStyle = '#38bdf8';
            ctx.beginPath();
            ctx.roundRect(10, 10, 60, 40, 8);
            ctx.fill();
            ctx.textAlign = 'center';
            ctx.textBaseline = 'middle';
            ctx.font = '14px sans-serif';
            ctx.fillStyle = '#ffffff';
            ctx.fillText('OK', 40, 30);
            ctx.textAlign + '|' + ctx.textBaseline + '|' + (ctx.measureText('OK').width > 0);
        "#,
        )
        .unwrap();
    assert_eq!(out.as_str(), Some("center|middle|true"));
    let st = env.dom.borrow();
    let cid = st.doc.get_element_by_id("c").unwrap();
    let fb = &st.canvases.get(&cid).unwrap().fb;
    let painted = fb.data.chunks_exact(4).filter(|p| p[3] > 0).count();
    assert!(painted > 1500, "expected painted rounded rect + text, got {}", painted);
}

#[test]
fn wpt_xpath_and_event_once_and_dataset_classlist() {
    let res = run_env(
        "<div id='el' class='a b' data-user-id='99'><span>One</span></div>",
        r#"
            const el = document.getElementById('el');
            el.classList.toggle('b');
            el.classList.add('c');
            el.dataset.roleName = 'admin';
            let hits = 0;
            el.addEventListener('tap', () => { hits += 1; }, { once: true });
            el.dispatchEvent(new Event('tap'));
            el.dispatchEvent(new Event('tap'));
            const ev = new XPathEvaluator();
            const expr = ev.createExpression('//span', null);
            const r = expr.evaluate(document, 4, null);
            [
                el.className,
                el.dataset.userId,
                el.getAttribute('data-role-name'),
                String(hits),
                r.iterateNext() === null,
            ].join('|');
        "#,
    );
    assert_eq!(res, "a c|99|admin|1|true");
}
