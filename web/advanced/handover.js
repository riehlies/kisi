// Link hand-over for the field solver page: a cross-section handed over in the URL fragment is solved and shown in
// a chosen view without further clicks.
//
//   field_solver.html#params=<the original's link form>&solve=1&view=sfield&mode=odd&exc=V:1:peak&title=<text>
//
// `params=` is the original's own link format (app_solver.js: settings as deflate-raw + base64url, "z." prefix; the
// original reads only that key and applies it on load and on every hashchange). The other keys are read here:
//   solve=1   start the solve once the link's settings are in place
//   view=     geometry | potential | efield | hfield | jfield | sfield | lossfield (the fork's views), after the solve
//   mode=     odd | even (pairs)
//   exc=      kind:value[:peak|rms], kind V | Vd | I | P (the fork's excitation)
//   title=    a line shown above the plot (where the cross-section comes from)
//   editor=hide  collapse the geometry editor under the plot (more room for the field view)
// The fragment never reaches a server and has no length limit of its own; a page that opened this one changes the
// fragment to hand over the next cross-section without a reload (a location change of a window it opened is allowed
// across origins and from file://).
//
// kisi bundles this file into kisi-advanced.html; it uses only the page's DOM and the Plotly menu the fork draws, no
// internals of either. It is written so that it can be added to the fork unchanged as src/fork/handover.js
// (docs/research/advanced-solvers.md, the proposed patch).
const VIEW_LABELS = { geometry: 'Geometry', potential: 'Potential', efield: '|E| Field', hfield: '|H| Field',
                      jfield: 'Current J', sfield: 'Power flow S', lossfield: 'Losses' };
const state = { seq: 0, phase: 'idle', solveMs: null, request: null, error: null };
window.__fieldHandover = state;

function readRequest(hash) {
    if (!/[#&]params=/.test(hash || '')) return null;
    const q = new URLSearchParams(hash.replace(/^#/, ''));
    return { solve: q.get('solve') === '1', view: q.get('view'), mode: q.get('mode'), exc: q.get('exc'),
             title: q.get('title'), editor: q.get('editor') };
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function until(test, ms, seq) {
    const end = performance.now() + ms;
    while (!test()) {
        if (seq !== state.seq) throw new Error('superseded');
        if (performance.now() > end) throw new Error('timeout');
        await sleep(50);
    }
}
function report(extra) {
    Object.assign(state, extra);
    try { if (window.opener) window.opener.postMessage({ type: 'field-handover', ...state, request: undefined }, '*'); }
    catch { /* opener gone */ }
}
function setValue(id, value) {
    const el = document.getElementById(id);
    if (!el || value == null) return;
    el.value = value;
    el.dispatchEvent(new Event('change', { bubbles: true }));
    el.dispatchEvent(new Event('input', { bubbles: true }));
}
function showTitle(text) {
    let el = document.getElementById('handover-title');
    if (!el) {
        el = document.createElement('div');
        el.id = 'handover-title';
        el.style.cssText = 'padding:4px 10px;font-size:0.9em;border-bottom:1px solid rgba(128,128,128,.35)';
        const host = document.getElementById('tab-geometry');
        if (!host) return;
        host.prepend(el);
    }
    el.textContent = text;
    window.dispatchEvent(new Event('resize'));   // the plot takes the room left
}
// a view of the fork's menu row in the plot (Plotly updatemenus, the menu named "view"): the click event the fork's
// menu handler listens to, as Plotly emits it for a click on the menu
function viewMenu() {
    const gd = document.getElementById('sim_canvas');
    const menus = (gd && gd._fullLayout && gd._fullLayout.updatemenus) || [];
    return [gd, menus.find((m) => m.name === 'view')];
}
function shownView() {
    const [, menu] = viewMenu();
    return menu && menu.buttons[menu.active] ? menu.buttons[menu.active].label : null;
}
function selectView(label) {
    const [gd, menu] = viewMenu();
    const active = menu ? menu.buttons.findIndex((b) => b.label === label) : -1;
    if (active < 0 || typeof gd.emit !== 'function') return false;
    gd.emit('plotly_buttonclicked', { menu: { ...menu, active }, button: menu.buttons[active], active });
    return true;
}

// the next draw fits the view to the new cross-section: both the original's geometry plot and the fork's field
// views keep the plot's current axis ranges unless there are none (a hand-over into an open page would otherwise
// show the new cross-section in the previous one's window)
function forgetZoom() {
    const gd = document.getElementById('sim_canvas');
    if (gd && gd.layout && gd.layout.xaxis && gd.layout.yaxis) { delete gd.layout.xaxis.range; delete gd.layout.yaxis.range; }
}

async function apply(req) {
    const seq = ++state.seq;
    report({ phase: 'loading', request: req, solveMs: null, error: null, t0: performance.now() });
    try {
        // the original applies the link asynchronously and then removes it from the address bar
        await until(() => !/[#&]params=/.test(location.hash), 20000, seq);
        await sleep(0);
        if (req.title) showTitle(req.title);
        if (req.editor === 'hide') {
            const b = document.getElementById('btn-custom-collapse');
            if (b && /hide/i.test(b.textContent)) b.click();
        }
        if (req.mode === 'odd' || req.mode === 'even') setValue('plot-mode', req.mode);
        if (req.exc) {
            const [kind, value, form] = req.exc.split(':');
            setValue('fork-exc-kind', kind);
            if (value) setValue('fork-exc-value', value);
            if (form) setValue('fork-exc-rms', form);
        }
        if (!req.solve) { report({ phase: 'ready' }); return; }
        const btn = document.getElementById('btn_solve');
        await until(() => btn && !btn.classList.contains('stop-mode'), 600000, seq);
        const t = performance.now();
        report({ phase: 'solving' });
        btn.click();
        await until(() => btn.classList.contains('stop-mode') || performance.now() - t > 1000, 5000, seq);
        await until(() => !btn.classList.contains('stop-mode'), 3600000, seq);
        report({ phase: 'solved', solveMs: Math.round(performance.now() - t) });
        forgetZoom();
        if (req.view && VIEW_LABELS[req.view]) {
            // the fork falls back to Geometry until the solve's fields are in place: ask again until it shows
            const label = VIEW_LABELS[req.view];
            let asked = 0;
            await until(() => shownView() === label || (performance.now() - asked > 400 && selectView(label) && (asked = performance.now()) && false), 20000, seq);
        }
        report({ phase: 'shown', shownMs: Math.round(performance.now() - t) });
    } catch (e) {
        if (String(e && e.message) !== 'superseded') report({ phase: 'error', error: String(e && e.message || e) });
    }
}

const first = readRequest(location.hash);   // read before the original consumes the fragment
if (first) apply(first);
window.addEventListener('hashchange', () => {
    const req = readRequest(location.hash);
    if (req) apply(req);
});
