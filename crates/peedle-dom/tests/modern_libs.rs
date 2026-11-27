use std::path::Path;

use peedle_dom::BrowserJsEnv;
use peedle_html::parse_html;
use peedle_net::HttpClient;

fn new_env(html: &str) -> BrowserJsEnv {
    let doc = parse_html(html);
    BrowserJsEnv::new(
        doc,
        "https://example.com/".to_string(),
        1024.0,
        768.0,
        HttpClient::new(),
    )
}

#[test]
fn dom_events_style_canvas_storage_crypto() {
    let mut env = new_env(
        r##"<!DOCTYPE html><html><body>
            <div id="parent"><button id="btn" class="a b">Click</button></div>
            <canvas id="cv" width="40" height="40"></canvas>
        </body></html>"##,
    );
    let res = env
        .eval(
            r##"
        const log = [];
        const parent = document.getElementById('parent');
        const btn = document.getElementById('btn');
        parent.addEventListener('click', () => log.push('parent-capture'), true);
        btn.addEventListener('click', () => log.push('btn'));
        parent.addEventListener('click', () => log.push('parent-bubble'));
        btn.click();

        btn.classList.add('active');
        btn.classList.remove('a');
        btn.style.backgroundColor = '#ff0000';
        const styleVal = btn.style.backgroundColor;

        const cv = document.getElementById('cv');
        const ctx = cv.getContext('2d');
        ctx.fillStyle = '#00ff00';
        ctx.fillRect(0, 0, 20, 20);
        const px = ctx.getImageData(5, 5, 1, 1).data;

        localStorage.setItem('k', 'v123');
        const u = new URL('/path?x=42', 'https://peedle.test/');
        JSON.stringify({
            log,
            hasActive: btn.classList.contains('active'),
            hasA: btn.classList.contains('a'),
            styleVal,
            px: [px[0], px[1], px[2], px[3]],
            ls: localStorage.getItem('k'),
            urlX: u.searchParams.get('x'),
            uuidLen: crypto.randomUUID().length
        });
    "##,
        )
        .unwrap();

    let s = res.as_str().unwrap();
    assert!(s.contains(r#"["parent-capture","btn","parent-bubble"]"#), "got {}", s);
    assert!(s.contains(r#""hasActive":true"#), "got {}", s);
    assert!(s.contains(r#""hasA":false"#), "got {}", s);
    assert!(s.contains(r#""px":[0,255,0,255]"#), "got {}", s);
    assert!(s.contains(r#""ls":"v123""#), "got {}", s);
    assert!(s.contains(r#""urlX":"42""#), "got {}", s);
    assert!(s.contains(r#""uuidLen":36"#), "got {}", s);
}

#[test]
fn real_preact_renders_and_updates_dom() {
    let lib_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/libs/preact.umd.js");
    if !lib_path.exists() {
        return;
    }
    let preact_src = std::fs::read_to_string(&lib_path).unwrap();
    let mut env = new_env(r#"<!DOCTYPE html><html><body><div id="root"></div></body></html>"#);
    env.eval(&preact_src).unwrap();
    env.eval(
        r#"
        const { h, render } = preact;
        const root = document.getElementById('root');
        render(h('div', { id: 'card', class: 'hero' }, [
            h('h1', null, 'Hello from Preact in Peedle!'),
            h('span', { id: 'count' }, 'Count: 1')
        ]), root);
    "#,
    )
    .unwrap();
    let html = env.dom.borrow().doc.inner_html(env.dom.borrow().doc.body().unwrap());
    eprintln!("Preact rendered HTML: {}", html);
    assert!(html.contains("Hello from Preact in Peedle!"), "got {}", html);
    assert!(html.contains("Count: 1"), "got {}", html);
}

#[test]
fn real_jquery_queries_and_mutates_dom() {
    let lib_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/libs/jquery.min.js");
    if !lib_path.exists() {
        return;
    }
    let jq_src = std::fs::read_to_string(&lib_path).unwrap();
    let mut env = new_env(
        r#"<!DOCTYPE html><html><body><ul id="list"><li class="item">One</li><li class="item">Two</li></ul></body></html>"#,
    );
    env.eval(&jq_src).unwrap();
    let out = env
        .eval(
            r#"
        $('#list').append('<li class="item">Three</li>');
        $('.item').addClass('done');
        $('.item').length + ':' + $('#list li:last-child').text();
    "#,
        )
        .unwrap();
    eprintln!("jQuery result: {:?}", out);
    assert_eq!(out.as_str(), Some("3:Three"));
}

#[test]
fn real_react18_mounts_and_clicks() {
    let r_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/libs/react.production.min.js");
    let rd_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/libs/react-dom.production.min.js");
    if !r_path.exists() || !rd_path.exists() {
        return;
    }
    let react_src = std::fs::read_to_string(&r_path).unwrap();
    let react_dom_src = std::fs::read_to_string(&rd_path).unwrap();
    let mut env = new_env(r#"<!DOCTYPE html><html><body><div id="app"></div></body></html>"#);
    env.eval(&react_src).unwrap();
    env.eval(&react_dom_src).unwrap();
    env.eval(
        r#"
        function Counter() {
            const [n, setN] = React.useState(0);
            return React.createElement('div', { id: 'counter' },
                React.createElement('span', { id: 'val' }, 'Value: ' + n),
                React.createElement('button', { id: 'inc', onClick: () => setN(c => c + 1) }, '+1')
            );
        }
        const root = ReactDOM.createRoot(document.getElementById('app'));
        root.render(React.createElement(Counter));
    "#,
    )
    .unwrap();
    let before = env.dom.borrow().doc.inner_html(env.dom.borrow().doc.body().unwrap());
    eprintln!("React 18 initial HTML: {}", before);
    assert!(before.contains("Value: 0"), "React initial render failed: {}", before);

    env.eval("document.getElementById('inc').click();").unwrap();
    let after = env.dom.borrow().doc.inner_html(env.dom.borrow().doc.body().unwrap());
    eprintln!("React 18 after click HTML: {}", after);
    assert!(after.contains("Value: 1"), "React click state update failed: {}", after);
}

#[test]
fn real_vue3_mounts_and_reacts() {
    let v_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/libs/vue.global.prod.js");
    if !v_path.exists() {
        return;
    }
    let vue_src = std::fs::read_to_string(&v_path).unwrap();
    let mut env = new_env(r#"<!DOCTYPE html><html><body><div id="vapp"></div></body></html>"#);
    env.eval(&vue_src).unwrap();
    env.eval(
        r#"
        const { createApp, ref, h } = Vue;
        const app = createApp({
            setup() {
                const count = ref(10);
                return () => h('div', { id: 'vbox' }, [
                    h('p', { id: 'vnum' }, 'Vue count: ' + count.value),
                    h('button', { id: 'vbtn', onClick: () => { count.value += 5; } }, 'Add 5')
                ]);
            }
        });
        app.mount('#vapp');
    "#,
    )
    .unwrap();
    let before = env.dom.borrow().doc.inner_html(env.dom.borrow().doc.body().unwrap());
    eprintln!("Vue 3 initial HTML: {}", before);
    assert!(before.contains("Vue count: 10"), "Vue initial render failed: {}", before);

    env.eval("document.getElementById('vbtn').click();").unwrap();
    let after = env.dom.borrow().doc.inner_html(env.dom.borrow().doc.body().unwrap());
    eprintln!("Vue 3 after click HTML: {}", after);
    assert!(after.contains("Vue count: 15"), "Vue reactivity update failed: {}", after);
}

#[test]
fn real_react18_todo_app_workflow() {
    let r_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/libs/react.production.min.js");
    let rd_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/libs/react-dom.production.min.js");
    if !r_path.exists() || !rd_path.exists() {
        return;
    }
    let react_src = std::fs::read_to_string(&r_path).unwrap();
    let react_dom_src = std::fs::read_to_string(&rd_path).unwrap();
    let mut env = new_env(r#"<!DOCTYPE html><html><body><div id="todo-root"></div></body></html>"#);
    env.eval(&react_src).unwrap();
    env.eval(&react_dom_src).unwrap();
    env.eval(
        r#"
        const { useState, useMemo, createElement: e } = React;
        function TodoApp() {
            const [items, setItems] = useState([
                { id: 1, text: 'Build HTML5 parser', done: true },
                { id: 2, text: 'Run React 18', done: false }
            ]);
            const [filter, setFilter] = useState('all');
            const visible = useMemo(() => {
                if (filter === 'active') return items.filter(i => !i.done);
                if (filter === 'done') return items.filter(i => i.done);
                return items;
            }, [items, filter]);
            const add = () => setItems(prev => [...prev, { id: prev.length + 1, text: 'Ship Peedle', done: false }]);
            const toggle = id => setItems(prev => prev.map(i => i.id === id ? { ...i, done: !i.done } : i));
            const remaining = items.filter(i => !i.done).length;
            return e('section', { className: 'todoapp' },
                e('button', { id: 'add-btn', onClick: add }, 'Add'),
                e('button', { id: 'filter-active', onClick: () => setFilter('active') }, 'Active'),
                e('span', { id: 'remaining' }, remaining + ' left'),
                e('ul', { id: 'todos' },
                    visible.map(item => e('li', {
                        key: item.id,
                        id: 'todo-' + item.id,
                        className: item.done ? 'done' : 'open',
                        onClick: () => toggle(item.id)
                    }, item.text))
                )
            );
        }
        ReactDOM.createRoot(document.getElementById('todo-root')).render(e(TodoApp));
    "#,
    )
    .unwrap();

    let s1 = env.eval("document.getElementById('remaining').textContent").unwrap();
    assert_eq!(s1.as_str(), Some("1 left"));

    env.eval("document.getElementById('add-btn').click();").unwrap();
    let s2 = env.eval("document.getElementById('remaining').textContent + ':' + document.querySelectorAll('#todos li').length").unwrap();
    assert_eq!(s2.as_str(), Some("2 left:3"));

    env.eval("document.getElementById('todo-2').click();").unwrap();
    env.eval("document.getElementById('filter-active').click();").unwrap();
    let s3 = env.eval("document.getElementById('remaining').textContent + ':' + document.querySelectorAll('#todos li').length + ':' + document.querySelector('#todos li').textContent").unwrap();
    assert_eq!(s3.as_str(), Some("1 left:1:Ship Peedle"));
}

#[test]
fn real_marked_renders_markdown_into_dom() {
    let m_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/libs/marked.min.js");
    if !m_path.exists() {
        return;
    }
    let marked_src = std::fs::read_to_string(&m_path).unwrap();
    let mut env = new_env(r#"<!DOCTYPE html><html><body><article id="out"></article></body></html>"#);
    env.eval(&marked_src).unwrap();
    env.eval(
        r#"
        const html = marked.parse('# Peedle\n\n- **Fast**\n- *Independent*\n');
        document.getElementById('out').innerHTML = html;
    "#,
    )
    .unwrap();
    let check = env
        .eval("document.querySelector('#out h1').textContent + '|' + document.querySelectorAll('#out li').length + '|' + document.querySelector('#out strong').textContent")
        .unwrap();
    assert_eq!(check.as_str(), Some("Peedle|2|Fast"));
}

#[test]
fn real_vue3_computed_todo_workflow() {
    let v_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/libs/vue.global.prod.js");
    if !v_path.exists() {
        return;
    }
    let vue_src = std::fs::read_to_string(&v_path).unwrap();
    let mut env = new_env(r#"<!DOCTYPE html><html><body><div id="vtodo"></div></body></html>"#);
    env.eval(&vue_src).unwrap();
    env.eval(
        r#"
        const { createApp, ref, computed, h } = Vue;
        const app = createApp({
            setup() {
                const tasks = ref([
                    { id: 1, title: 'Parse HTML5', done: true },
                    { id: 2, title: 'Pass Acid2', done: false }
                ]);
                const pending = computed(() => tasks.value.filter(t => !t.done).length);
                const addTask = () => tasks.value.push({ id: 3, title: 'Pass Acid3', done: false });
                const finishSecond = () => { tasks.value[1].done = true; };
                return () => h('div', { id: 'vt' }, [
                    h('span', { id: 'vpending' }, pending.value + ' pending'),
                    h('button', { id: 'vadd', onClick: addTask }, 'Add'),
                    h('button', { id: 'vdone', onClick: finishSecond }, 'Done'),
                    h('ul', { id: 'vlist' }, tasks.value.map(t => h('li', { key: t.id }, t.title)))
                ]);
            }
        });
        app.mount('#vtodo');
    "#,
    )
    .unwrap();
    assert_eq!(
        env.eval("document.getElementById('vpending').textContent").unwrap().as_str(),
        Some("1 pending")
    );
    env.eval("document.getElementById('vadd').click();").unwrap();
    assert_eq!(
        env.eval("document.getElementById('vpending').textContent + ':' + document.querySelectorAll('#vlist li').length").unwrap().as_str(),
        Some("2 pending:3")
    );
    env.eval("document.getElementById('vdone').click();").unwrap();
    assert_eq!(
        env.eval("document.getElementById('vpending').textContent").unwrap().as_str(),
        Some("1 pending")
    );
}

#[test]
fn real_d3_generates_svg_bar_chart() {
    let d3_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/libs/d3.min.js");
    if !d3_path.exists() {
        return;
    }
    let d3_src = std::fs::read_to_string(&d3_path).unwrap();
    let mut env = new_env(r#"<!DOCTYPE html><html><body><div id="chart"></div></body></html>"#);
    env.eval(&d3_src).unwrap();
    let out = env
        .eval(
            r#"
        const data = [10, 25, 40, 15, 30];
        const x = d3.scaleBand().domain(d3.range(data.length)).range([0, 200]).padding(0.1);
        const y = d3.scaleLinear().domain([0, d3.max(data)]).range([100, 0]);
        const svg = d3.select('#chart').append('svg').attr('width', 200).attr('height', 100);
        svg.selectAll('rect')
            .data(data)
            .join('rect')
            .attr('class', 'bar')
            .attr('x', (d, i) => x(i))
            .attr('y', d => y(d))
            .attr('width', x.bandwidth())
            .attr('height', d => 100 - y(d));
        const rects = document.querySelectorAll('#chart svg rect.bar');
        rects.length + ':' + rects[2].getAttribute('height') + ':' + d3.sum(data);
    "#,
        )
        .unwrap();
    assert_eq!(out.as_str(), Some("5:100:120"));
}

#[test]
fn real_alpine_reactive_counter() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/libs/alpine.min.js");
    if !path.exists() {
        return;
    }
    let alpine_src = std::fs::read_to_string(&path).unwrap();
    let mut env = new_env(
        r#"<!DOCTYPE html><html><body>
            <div id="box" x-data="{ count: 1, label: 'Peedle' }">
                <span id="out" x-text="label + ':' + count"></span>
                <button id="inc" @click="count++">Up</button>
            </div>
        </body></html>"#,
    );
    env.eval(&alpine_src).unwrap();
    env.eval("if (window.Alpine && !window.Alpine._started) { Alpine.start(); }").unwrap();
    let before = env
        .eval("document.getElementById('out').textContent")
        .unwrap();
    assert_eq!(before.as_str(), Some("Peedle:1"));
    env.eval("document.getElementById('inc').click();").unwrap();
    let after = env
        .eval("document.getElementById('out').textContent")
        .unwrap();
    assert_eq!(after.as_str(), Some("Peedle:2"));
}

#[test]
fn real_htmx_boots_and_handles_events() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/libs/htmx.min.js");
    if !path.exists() {
        return;
    }
    let htmx_src = std::fs::read_to_string(&path).unwrap();
    let mut env = new_env(
        r#"<!DOCTYPE html><html><head></head><body>
            <div id="panel"><button id="btn" class="action">Go</button><div id="target">old</div></div>
        </body></html>"#,
    );
    env.eval(&htmx_src).unwrap();
    env.dom.borrow_mut().ready_state = "complete".to_string();
    env.dispatch_dom_event(None, "DOMContentLoaded", true, false).unwrap();
    let out = env
        .eval(
            r#"
        let fired = 0;
        const btn = htmx.find('#btn');
        htmx.on(btn, 'custom-ping', () => { fired += 1; });
        htmx.addClass(btn, 'active');
        htmx.trigger(btn, 'custom-ping');
        htmx.swap('#target', '<span>swapped</span>', { swapStyle: 'innerHTML' });
        htmx.version.split('.')[0] + ':' + fired + ':' + btn.classList.contains('active') + ':' + htmx.find('#target span').textContent;
    "#,
        )
        .unwrap();
    assert_eq!(out.as_str(), Some("2:1:true:swapped"));
}

#[test]
fn real_redux_store_subscribe_and_dom_sync() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/libs/redux.min.js");
    if !path.exists() {
        return;
    }
    let redux_src = std::fs::read_to_string(&path).unwrap();
    let mut env = new_env(r#"<!DOCTYPE html><html><body><div id="count">0</div><button id="add">+</button></body></html>"#);
    env.eval(&redux_src).unwrap();
    let out = env
        .eval(
            r#"
        function counter(state = { value: 0, log: [] }, action) {
            if (action.type === 'INC') {
                return { value: state.value + action.by, log: state.log.concat(action.by) };
            }
            return state;
        }
        const store = Redux.createStore(counter);
        const el = document.getElementById('count');
        store.subscribe(() => {
            el.textContent = String(store.getState().value);
        });
        document.getElementById('add').addEventListener('click', () => {
            store.dispatch({ type: 'INC', by: 5 });
        });
        store.dispatch({ type: 'INC', by: 2 });
        document.getElementById('add').click();
        el.textContent + ':' + store.getState().log.join(',');
    "#,
        )
        .unwrap();
    assert_eq!(out.as_str(), Some("7:2,5"));
}

#[test]
fn real_prismjs_highlights_javascript_into_dom() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/libs/prism.min.js");
    if !path.exists() {
        return;
    }
    let prism_src = std::fs::read_to_string(&path).unwrap();
    let mut env = new_env(
        r#"<!DOCTYPE html><html><body><pre><code id="snippet" class="language-javascript">const answer = 42; return answer + "!";</code></pre></body></html>"#,
    );
    env.eval(&prism_src).unwrap();
    let out = env
        .eval(
            r#"
        const code = document.getElementById('snippet');
        Prism.highlightElement(code);
        const kw = Array.from(code.querySelectorAll('.token.keyword')).map(n => n.textContent).join(',');
        const num = code.querySelector('.token.number').textContent;
        const str = code.querySelector('.token.string').textContent;
        kw + '|' + num + '|' + str;
    "#,
        )
        .unwrap();
    assert_eq!(out.as_str(), Some("const,return|42|\"!\""));
}

#[test]
fn real_chartjs_renders_bar_chart_to_canvas2d() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/libs/chart.umd.min.js");
    if !path.exists() {
        return;
    }
    let chart_src = std::fs::read_to_string(&path).unwrap();
    let mut env = new_env(
        r#"<!DOCTYPE html><html><body><div style="width:400px;height:240px"><canvas id="cv" width="400" height="240"></canvas></div></body></html>"#,
    );
    env.eval(&chart_src).unwrap();
    let out = env
        .eval(
            r#"
        const canvas = document.getElementById('cv');
        const chart = new Chart(canvas, {
            type: 'bar',
            data: {
                labels: ['URL', 'HTML', 'CSS', 'JS'],
                datasets: [{
                    label: 'Conformance',
                    data: [100, 100, 99, 99],
                    backgroundColor: ['#38bdf8', '#818cf8', '#34d399', '#fbbf24']
                }]
            },
            options: {
                responsive: false,
                animation: false
            }
        });
        chart.update('none');
        const meta = chart.getDatasetMeta(0);
        chart.config.type + ':' + meta.data.length + ':' + (chart.width > 0);
    "#,
        )
        .unwrap();
    assert_eq!(out.as_str(), Some("bar:4:true"));
    let st = env.dom.borrow();
    let cv_id = st.doc.get_element_by_id("cv").unwrap();
    let cv = st.canvases.get(&cv_id).expect("chartjs must draw onto Canvas2D");
    let non_transparent = cv.fb.data.chunks_exact(4).filter(|p| p[3] > 0).count();
    assert!(
        non_transparent > 500,
        "expected Chart.js to rasterize bars onto Canvas2D, got {} painted pixels",
        non_transparent
    );
}
