(function () {
  const params = new URLSearchParams(location.search);
  const token = params.get('token') || '';
  const steps = document.getElementById('steps');

  document.querySelectorAll('.step').forEach(setupStep);

  function setupStep(card) {
    const shot = card.querySelector('.shot');
    if (!shot) return; // manual (text-only) step — nothing to redact
    const img = shot.querySelector('img');
    const overlay = shot.querySelector('.overlay');
    let boxes = [];            // natural image-pixel [x,y,w,h]
    let start = null, tempEl = null;

    const scale = () => img.naturalWidth / (img.clientWidth || img.naturalWidth);

    function persist() { card.dataset.redact = JSON.stringify(boxes); }
    function render() {
      overlay.querySelectorAll('.rbox:not(.temp)').forEach(e => e.remove());
      const s = scale();
      boxes.forEach((b, i) => {
        const d = document.createElement('div');
        d.className = 'rbox';
        d.style.left = (b[0] / s) + 'px'; d.style.top = (b[1] / s) + 'px';
        d.style.width = (b[2] / s) + 'px'; d.style.height = (b[3] / s) + 'px';
        d.title = '✕';
        d.addEventListener('click', ev => { ev.stopPropagation(); boxes.splice(i, 1); persist(); render(); });
        overlay.appendChild(d);
      });
    }
    function addBox(a, b) {
      const s = scale();
      const x = Math.min(a.x, b.x), y = Math.min(a.y, b.y);
      const w = Math.abs(a.x - b.x), h = Math.abs(a.y - b.y);
      if (w < 4 || h < 4) return;
      boxes.push([Math.round(x * s), Math.round(y * s), Math.round(w * s), Math.round(h * s)]);
      persist(); render();
    }

    shot.addEventListener('mousedown', e => {
      if (e.button !== 0 || e.target.classList.contains('rbox')) return;
      const r = img.getBoundingClientRect();
      start = { x: e.clientX - r.left, y: e.clientY - r.top };
      tempEl = document.createElement('div');
      tempEl.className = 'rbox temp';
      overlay.appendChild(tempEl);
      e.preventDefault();
    });
    window.addEventListener('mousemove', e => {
      if (!start) return;
      const r = img.getBoundingClientRect();
      const cx = e.clientX - r.left, cy = e.clientY - r.top;
      tempEl.style.left = Math.min(start.x, cx) + 'px';
      tempEl.style.top = Math.min(start.y, cy) + 'px';
      tempEl.style.width = Math.abs(cx - start.x) + 'px';
      tempEl.style.height = Math.abs(cy - start.y) + 'px';
    });
    window.addEventListener('mouseup', e => {
      if (!start) return;
      const r = img.getBoundingClientRect();
      addBox(start, { x: e.clientX - r.left, y: e.clientY - r.top });
      start = null;
      if (tempEl) { tempEl.remove(); tempEl = null; }
    });

    const elemBtn = card.querySelector('.elembtn');
    if (elemBtn) elemBtn.addEventListener('click', () => {
      const b = JSON.parse(elemBtn.dataset.box);
      boxes.push(b); persist(); render();
    });

    window.addEventListener('resize', render);
    img.addEventListener('load', render);
  }

  function renumber() {
    let n = 0;
    steps.querySelectorAll('.step').forEach(c => {
      c.querySelector('.num').textContent = c.classList.contains('deleted') ? '—' : (++n);
    });
  }

  steps.addEventListener('click', e => {
    const card = e.target.closest('.step');
    if (!card) return;
    if (e.target.classList.contains('up') && card.previousElementSibling)
      card.parentNode.insertBefore(card, card.previousElementSibling);
    else if (e.target.classList.contains('down') && card.nextElementSibling)
      card.parentNode.insertBefore(card.nextElementSibling, card);
    else if (e.target.classList.contains('del'))
      card.classList.toggle('deleted');
    renumber();
  });

  // Insert a manual (text, optional image) step at the end.
  const addBtn = document.getElementById('addstep');
  addBtn.addEventListener('click', () => {
    const card = document.createElement('section');
    card.className = 'step manual';
    card.dataset.manual = '1';
    card.innerHTML =
      '<div class="bar"><span class="num">+</span>' +
      '<button type="button" class="mv up" title="▲">▲</button>' +
      '<button type="button" class="mv down" title="▼">▼</button>' +
      '<button type="button" class="del">' + addBtn.dataset.del + '</button></div>' +
      '<input class="desc" placeholder="' + addBtn.dataset.text + '">' +
      '<input class="mfile" type="file" accept="image/*">';
    steps.appendChild(card);
    renumber();
    card.querySelector('.desc').focus();
  });

  function readAsDataURL(file) {
    return new Promise((resolve, reject) => {
      const r = new FileReader();
      r.onload = () => resolve(r.result);
      r.onerror = reject;
      r.readAsDataURL(file);
    });
  }

  async function gather() {
    const entries = [];
    for (const card of steps.querySelectorAll('.step')) {
      if (card.classList.contains('deleted')) continue;
      if (card.dataset.manual) {
        const text = card.querySelector('.desc').value.trim();
        if (!text) continue; // skip empty manual steps
        const entry = { text };
        const file = card.querySelector('.mfile').files[0];
        if (file) entry.image = await readAsDataURL(file);
        entries.push(entry);
      } else {
        const ref = +card.dataset.ref;
        const auto = card.dataset.auto;
        const desc = card.querySelector('.desc').value;
        const redact = JSON.parse(card.dataset.redact || '[]');
        const entry = { ref, description: desc === auto ? null : desc };
        if (redact.length) entry.redact = redact;
        entries.push(entry);
      }
    }
    return entries;
  }

  const applyBtn = document.getElementById('apply');
  const status = document.getElementById('status');
  applyBtn.addEventListener('click', async () => {
    const entries = await gather();
    status.textContent = '…';
    try {
      const res = await fetch('/apply?token=' + encodeURIComponent(token), {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ steps: entries })
      });
      const j = await res.json();
      if (j.ok) {
        status.textContent = applyBtn.dataset.applied.replace('{n}', j.steps);
        setTimeout(() => location.reload(), 700);
      } else {
        status.textContent = applyBtn.dataset.error + ': ' + (j.error || '');
      }
    } catch (err) {
      status.textContent = applyBtn.dataset.error + ': ' + err;
    }
  });

  document.getElementById('done').addEventListener('click', async () => {
    try { await fetch('/done?token=' + encodeURIComponent(token)); } catch (e) {}
    document.body.innerHTML = '<p style="padding:2rem">stepshot editor closed. You can close this tab.</p>';
  });
})();
