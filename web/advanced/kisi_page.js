// kisi-advanced.html: what the single-file build changes on the fork's page, at run time (no file of the fork or the
// original is changed):
//   * in the tab bar: the switch to the 3D view of a via (web/advanced/fdtd/, window.kisiAdvMode) and a line with
//     where this page comes from, its licence and sources (the files the About tab links to are embedded and opened
//     from here) and, in the quasi-static build, what it lacks;
//   * quasi-static build (no full-wave modules, tools/advanced_bundle.py without --fw; local work and CI checks): the
//     Full-wave choice disabled, Quasi-static chosen;
//   * the Time domain tab hidden (its FFT module, fft.wasm, is not in kisi's builds).
const adv = window.__kisiAdv;
const FORK = window.__kisiAdvInfo || {};
const hasFW = adv && adv.has('wasm_solver/gmsh.wasm') && adv.has('wasm_solver/eigen_solver.wasm');

async function blobURL(name, type) {
    return URL.createObjectURL(new Blob([await adv.getFile(name)], { type }));
}
async function linkNotices() {
    for (const [file, name] of [['LICENSE.txt', 'kisi/LICENSE.txt'], ['THIRD_PARTY_NOTICES.txt', 'kisi/THIRD_PARTY_NOTICES.txt']]) {
        const url = await blobURL(name, 'text/plain;charset=utf-8');
        for (const a of document.querySelectorAll(`a[href="${file}"]`)) a.href = url;
    }
}
function banner() {
    const el = document.createElement('div');
    el.id = 'kisi-adv-banner';
    el.style.cssText = 'font:11px/1.4 system-ui,sans-serif;padding:2px 10px;opacity:.85;align-self:center;margin-left:auto;' +
        'max-width:52%;text-align:right';
    const rev = (FORK.commit || '').slice(0, 7);
    el.innerHTML = `kisi field views${hasFW ? '' : ' (quasi-static build)'}: <a href="${FORK.repo}" target="_blank" rel="noopener">` +
        `js_2d_fields (fork ${rev}, ${(FORK.patches || []).length} patches of kisi)</a> by Henrik Forstén and David Riehl, GPL-3.0 – ` +
        `<a href="LICENSE.txt" target="_blank">licence</a>, <a href="THIRD_PARTY_NOTICES.txt" target="_blank">third-party notices</a>` +
        (hasFW ? '' : '<br><b>Quasi-static build: potential and |E| only.</b> ' +
            'The release\'s kisi-advanced.html adds |H|, current density, power flow and losses (full-wave).');
    const bar = document.querySelector('.tab-bar') || document.body;
    const sw = Object.assign(document.createElement('button'), { className: 'kisi3d-switch', id: 'kisi3d-switch',
        textContent: '3D via ▸', title: 'The 3D field view of a via (kisi-fdtd in this browser)' });
    sw.onclick = () => window.kisiAdvMode && window.kisiAdvMode('3d');
    bar.append(sw, el);
}
function restrictBackends() {
    if (hasFW) return;
    const sel = document.getElementById('mesh_backend');
    if (!sel) return;
    for (const o of sel.options) if (o.value !== 'rectilinear' && !o.disabled) { o.disabled = true; o.textContent += ' (not in the quasi-static build)'; }
    if (sel.value !== 'rectilinear') { sel.value = 'rectilinear'; sel.dispatchEvent(new Event('change', { bubbles: true })); }
}
function hideTimeDomain() {
    if (adv && adv.has('wasm_solver/fft.wasm')) return;
    for (const b of document.querySelectorAll('.tab-button[data-tab="eye"]')) b.style.display = 'none';
}

banner();
hideTimeDomain();
linkNotices();
// the original builds its controls in init (DOMContentLoaded); a link applied later may choose Full-wave again
restrictBackends();
setTimeout(restrictBackends, 0);
window.addEventListener('hashchange', () => setTimeout(restrictBackends, 200));
document.addEventListener('click', (e) => { if (e.target && e.target.id === 'btn_solve') restrictBackends(); }, true);
