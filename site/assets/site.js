// Small progressive enhancements. Every page works without this script.

const storage = {
  get(key) { try { return localStorage.getItem(key); } catch { return null; } },
  set(key, value) { try { localStorage.setItem(key, value); } catch { /* private mode */ } },
};

// Theme: follow the system until the reader picks one.
const themeButton = document.querySelector('[data-theme-toggle]');
const savedTheme = storage.get('theme');
if (savedTheme === 'light' || savedTheme === 'dark') document.documentElement.dataset.theme = savedTheme;
if (themeButton) {
  themeButton.addEventListener('click', () => {
    const current = document.documentElement.dataset.theme
      || (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
    const next = current === 'dark' ? 'light' : 'dark';
    document.documentElement.dataset.theme = next;
    storage.set('theme', next);
  });
}

// Sidebar: open on wide screens, collapsed behind a summary on phones.
const navigation = document.querySelector('.navigation');
const narrow = matchMedia('(max-width: 820px)');
if (navigation) {
  navigation.open = !narrow.matches;
  narrow.addEventListener('change', (event) => { navigation.open = !event.matches; });
}

// Copy buttons on code blocks.
for (const button of document.querySelectorAll('.copy')) {
  if (!navigator.clipboard) continue;
  button.hidden = false;
  button.addEventListener('click', async () => {
    const code = button.closest('.code').querySelector('pre code').textContent;
    try {
      await navigator.clipboard.writeText(code);
      button.textContent = 'Copied';
    } catch {
      button.textContent = 'Select to copy';
    }
    setTimeout(() => { button.textContent = 'Copy'; }, 1600);
  });
}

// Highlight the section in view in the page outline.
const tocLinks = [...document.querySelectorAll('.toc a')];
if (tocLinks.length && 'IntersectionObserver' in window) {
  const byId = new Map(tocLinks.map((a) => [decodeURIComponent(a.hash.slice(1)), a]));
  const observer = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      tocLinks.forEach((a) => a.classList.remove('active'));
      byId.get(entry.target.id)?.classList.add('active');
    }
  }, { rootMargin: '-70px 0px -70% 0px' });
  byId.forEach((_, id) => { const el = document.getElementById(id); if (el) observer.observe(el); });
}

// Configuration reference filter.
const optFilter = document.getElementById('opt-filter');
const optNew = document.getElementById('opt-new');
if (optFilter && optNew) {
  const apply = () => {
    const query = optFilter.value.trim().toLowerCase();
    for (const section of document.querySelectorAll('[data-section]')) {
      let visible = 0;
      for (const option of section.querySelectorAll('.conf-option')) {
        const show = (!query || option.dataset.name.includes(query) || option.textContent.toLowerCase().includes(query))
          && (!optNew.checked || option.dataset.new === '1');
        option.hidden = !show;
        if (show) visible += 1;
      }
      section.hidden = visible === 0;
    }
  };
  optFilter.addEventListener('input', apply);
  optNew.addEventListener('change', apply);
}

// Search: a small index of every page section, loaded on first use.
const searchInput = document.getElementById('search');
const results = document.getElementById('search-results');
if (searchInput && results) {
  let index = null;
  let selected = -1;
  let renderSeq = 0;
  const load = async () => {
    if (index) return index;
    try {
      const response = await fetch(searchInput.dataset.index);
      index = (await response.json()).map((entry) => ({
        ...entry,
        hay: `${entry.t} ${entry.s} ${entry.x}`.toLowerCase(),
      }));
    } catch {
      index = [];
    }
    return index;
  };
  const escapeHtml = (text) => text.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  const snippet = (text, term) => {
    const at = text.toLowerCase().indexOf(term);
    const start = Math.max(0, at - 40);
    return (start > 0 ? '…' : '') + text.slice(start, start + 140);
  };
  // hidden results are emptied, so no key can follow a link from an old query
  const hide = () => { results.hidden = true; results.innerHTML = ''; selected = -1; };
  const render = async () => {
    const seq = ++renderSeq;
    const query = searchInput.value.trim().toLowerCase();
    if (query.length < 2) { hide(); return; }
    const terms = query.split(/\s+/);
    const entries = await load();
    // a newer keystroke started its own render while the index loaded
    if (seq !== renderSeq) return;
    const scored = [];
    for (const entry of entries) {
      if (!terms.every((term) => entry.hay.includes(term))) continue;
      let score = 0;
      for (const term of terms) {
        if (entry.t.toLowerCase().includes(term)) score += 8;
        if (entry.s.toLowerCase().includes(term)) score += 5;
        score += Math.min(3, entry.x.toLowerCase().split(term).length - 1);
      }
      scored.push([score, entry]);
    }
    scored.sort((a, b) => b[0] - a[0]);
    selected = -1;
    results.innerHTML = scored.length
      ? scored.slice(0, 8).map(([, e]) => `<li><a href="${searchInput.dataset.root}${e.u}"><span class="r-path">${escapeHtml(e.t)}${e.s ? ' :: ' + escapeHtml(e.s) : ''}</span><span class="r-text">${escapeHtml(snippet(e.x, terms[0]))}</span></a></li>`).join('')
      : '<li class="r-empty">No matches. Try a shorter word or an option name.</li>';
    results.hidden = false;
  };
  const move = (step) => {
    const links = [...results.querySelectorAll('a')];
    if (!links.length) return;
    selected = (selected + step + links.length) % links.length;
    links.forEach((a, i) => a.setAttribute('aria-selected', String(i === selected)));
    links[selected].scrollIntoView({ block: 'nearest' });
  };
  searchInput.addEventListener('input', render);
  searchInput.addEventListener('focus', () => { load(); if (searchInput.value.trim().length > 1) render(); });
  searchInput.addEventListener('keydown', (event) => {
    if (results.hidden && event.key !== 'Escape') return;
    if (event.key === 'ArrowDown') { event.preventDefault(); move(1); }
    else if (event.key === 'ArrowUp') { event.preventDefault(); move(-1); }
    else if (event.key === 'Enter') {
      const link = results.querySelectorAll('a')[Math.max(0, selected)];
      if (link) { event.preventDefault(); window.location.href = link.href; }
    } else if (event.key === 'Escape') { renderSeq += 1; hide(); searchInput.blur(); }
  });
  document.addEventListener('click', (event) => {
    if (!event.target.closest('.search')) { renderSeq += 1; hide(); }
  });
  document.addEventListener('keydown', (event) => {
    if (event.key === '/' && !/input|textarea|select/i.test(document.activeElement.tagName)) {
      event.preventDefault();
      searchInput.focus();
    }
  });
}
