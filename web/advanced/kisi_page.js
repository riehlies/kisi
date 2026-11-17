// kisi-advanced.html: what the single-file build changes on the fork's page, at run time (no file of the fork or the
// original is changed):
//   * a line on top: where this page comes from, its licence and sources (the files the About tab links to are
//     embedded and opened from here);
//   * without the full-wave modules in the build: the Full-wave choice disabled, Quasi-static chosen;
//   * the Time domain tab hidden (its FFT module, fft.wasm, is not in this build).
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
    el.style.cssText = 'font:11px/1.4 system-ui,sans-serif;padding:2px 10px;opacity:.8;align-self:center;margin-left:auto;' +
        'max-width:46%;text-align:right';
    const rev = (FORK.commit || '').slice(0, 7);
    el.innerHTML = `kisi · Advanced solvers: field views of <a href="${FORK.repo}" target="_blank" rel="noopener">` +
        `js_2d_fields (fork, ${rev})</a> by Henrik Forstén and David Riehl, GPL-3.0 – ` +
        `<a href="LICENSE.txt" target="_blank">licence</a>, <a href="THIRD_PARTY_NOTICES.txt" target="_blank">third-party notices</a>` +
        (hasFW ? '' : ' · <b>quasi-static only in this build</b> (potential, |E|)');
    (document.querySelector('.tab-bar') || document.body).append(el);
}
function restrictBackends() {
    if (hasFW) return;
    const sel = document.getElementById('mesh_backend');
    if (!sel) return;
    for (const o of sel.options) if (o.value !== 'rectilinear' && !o.disabled) { o.disabled = true; o.textContent += ' (not in this build)'; }
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
