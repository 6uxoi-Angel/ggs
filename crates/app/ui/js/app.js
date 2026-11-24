/* ═══ Integra 3 — UI logic (vanilla JS, no build step) ═══ */
'use strict';

const T = {
  invoke: (cmd, args) => window.__TAURI__.core.invoke(cmd, args),
  listen: (event, handler) => window.__TAURI__.event.listen(event, handler),
  dialog: () => window.__TAURI__.dialog,
};

const BUILTIN_THEMES = ['dark_accent', 'dark', 'light'];
const THEME_COLOR_KEYS = ['accent', 'background', 'surface', 'tabbar', 'tab', 'text', 'muted', 'border', 'success', 'warning', 'danger'];
const TABS = [
  { id: 'dashboard', ico: '▦' },
  { id: 'apps', ico: '⇄' },
  { id: 'lists', ico: '☰' },
  { id: 'diagnostics', ico: '◎' },
  { id: 'dns', ico: '⌖' },
  { id: 'tgproxy', ico: '⇆' },
  { id: 'analysis', ico: '◈' },
  { id: 'extension', ico: '⧉' },
  { id: 'logs', ico: '≣' },
  { id: 'settings', ico: '⚙' },
];

const state = {
  bundle: {},
  config: null,
  profiles: [],
  strategies: {},
  page: 'dashboard',
  // lists
  listId: 'list-general',
  listMeta: null,
  listEntries: [],
  listSelected: new Set(),
  listFilter: '',
  // netmon
  netmonRunning: false,
  netmonTimer: null,
  netmonSummary: [],
  netmonEntries: [],
  netmonTableFilter: '',
  // logs
  logs: [],
  logLevel: 'ALL',
  // diagnostics
  diagSites: [],
  diagResults: null,
  diagBusy: false,
  // benchmark
  bench: { running: false, progress: null, result: null },
  // bridge
  bridge: null,
  pairing: null,
  pairingTimer: null,
  // maintenance
  release: null,
  installedVersion: null,
  maintBusy: false,
  maintPercent: 0,
  // dev
  devClicks: 0,
  devClickTimer: null,
  // theme editor (live values while customizing)
  themeEdit: null,
  imageCache: {},
  settingsRefreshTimer: null,
  // dns
  dnsBench: null,
  dnsBusy: false,
  rbxCands: [],
  rbxBusy: false,
  // tg proxy
  tg: null,
  tgLogs: [],
  tgPollTimer: null,
  tgLogTimer: null,
};

/* ── i18n ──
   Resolution order: backend bundle (selected language) → window.__I18N_DATA__
   (UI source of truth, key -> [ru, en]) → ru → en → the key itself. */
function t(key, args) {
  let out = state.bundle[key];
  if (out == null) {
    const pair = window.__I18N_DATA__?.[key];
    if (pair) {
      const lang = state.config?.language === 'en' ? 1 : 0;
      out = pair[lang] ?? pair[0] ?? pair[1];
    }
  }
  if (out == null) out = key;
  if (args) for (const [name, value] of Object.entries(args)) out = out.replaceAll(`{${name}}`, value);
  return out;
}
function esc(s) {
  return String(s ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}
function applyI18n(root = document) {
  root.querySelectorAll('[data-i18n]').forEach((el) => { el.textContent = t(el.dataset.i18n); });
  // keys whose translations contain inline markup (<b>, <code>, <i> …)
  root.querySelectorAll('[data-i18n-html]').forEach((el) => { el.innerHTML = t(el.dataset.i18nHtml); });
  root.querySelectorAll('[data-i18n-placeholder]').forEach((el) => { el.placeholder = t(el.dataset.i18nPlaceholder); });
  root.querySelectorAll('[data-i18n-title]').forEach((el) => { el.title = t(el.dataset.i18nTitle); });
  root.querySelectorAll('[data-i18n-aria-label]').forEach((el) => { el.setAttribute('aria-label', t(el.dataset.i18nAriaLabel)); });
}

/* ── api wrapper ── */
async function api(cmd, args = {}) {
  try {
    return await T.invoke(cmd, args);
  } catch (e) {
    const err = typeof e === 'string' ? { message: e } : (e || {});
    if (err.key === 'zapret.not_admin' || err.key === 'dns.not_admin') offerAdmin();
    else toast(err.message || String(e), 'err');
    throw e;
  }
}
function offerAdmin() {
  openModal({
    title: t('app.admin_required_title'),
    body: `<p>${esc(t('zapret.not_admin'))}</p>`,
    actions: [
      { label: t('app.relaunch_admin'), kind: 'primary', onClick: async () => { closeModal(); try { await api('relaunch_as_admin'); } catch { /* noop */ } } },
      { label: t('common.cancel') },
    ],
  });
}

/* ── toasts / modal ── */
function toast(message, type = 'info', ms = 3600) {
  const box = document.getElementById('toasts');
  const el = document.createElement('div');
  el.className = `toast ${type}`;
  el.textContent = message;
  box.appendChild(el);
  setTimeout(() => { el.classList.add('hide'); setTimeout(() => el.remove(), 350); }, ms);
}

/* Modal accessibility: a stack of openers (nested opens replace the same
   dialog node), Escape closes the top-most open modal, focus is trapped
   inside while open and returns to the opener on close. */
const modalStack = [];
function focusableIn(root) {
  return [...root.querySelectorAll('button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])')]
    .filter((el) => !el.disabled && el.offsetParent !== null);
}
function openModal({ title, body, actions = [{ label: t('common.ok') }] }) {
  modalStack.push({ opener: document.activeElement });
  document.getElementById('modal-title').textContent = title;
  document.getElementById('modal-body').innerHTML = body;
  const box = document.getElementById('modal-actions');
  box.innerHTML = '';
  for (const action of actions) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = `btn ${action.kind || 'ghost'}`;
    btn.textContent = action.label;
    btn.onclick = () => { if (action.onClick) action.onClick(); else closeModal(); };
    box.appendChild(btn);
  }
  document.getElementById('modal-backdrop').hidden = false;
  requestAnimationFrame(() => {
    const first = focusableIn(document.getElementById('modal'))[0];
    if (first) first.focus();
  });
}
function closeModal() {
  const backdrop = document.getElementById('modal-backdrop');
  if (backdrop.hidden) return;
  modalStack.pop();
  backdrop.hidden = true;
  // restore focus to the nearest opener that is still in the document
  while (modalStack.length) {
    const { opener } = modalStack[modalStack.length - 1];
    if (opener && opener.isConnected) { opener.focus(); return; }
    modalStack.pop();
  }
}
document.addEventListener('keydown', (e) => {
  const backdrop = document.getElementById('modal-backdrop');
  if (backdrop.hidden) return;
  if (e.key === 'Escape') {
    e.preventDefault();
    closeModal();
  } else if (e.key === 'Tab') {
    // trap focus inside the modal while it is open
    const modal = document.getElementById('modal');
    const items = focusableIn(modal);
    if (!items.length) return;
    const first = items[0];
    const last = items[items.length - 1];
    if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
    else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
    else if (!modal.contains(document.activeElement)) { e.preventDefault(); first.focus(); }
  }
});
function confirmModal(text, onYes, danger = true) {
  openModal({
    title: t('common.status'),
    body: `<p>${esc(text)}</p>`,
    actions: [
      { label: t('common.yes'), kind: danger ? 'danger' : 'primary', onClick: () => { closeModal(); onYes(); } },
      { label: t('common.no') },
    ],
  });
}

/* ── busy helper ── */
async function withBusy(btn, fn) {
  if (!btn) return fn();
  if (btn.dataset.busy === '1') return undefined;
  btn.dataset.busy = '1';
  const wasDisabled = btn.disabled;
  btn.disabled = true;
  btn.setAttribute('aria-busy', 'true');
  btn.classList.add('busy');
  try {
    return await fn();
  } finally {
    btn.disabled = wasDisabled;
    btn.removeAttribute('aria-busy');
    btn.classList.remove('busy');
    delete btn.dataset.busy;
  }
}

/* ── theme & taskbar ── */
function presetById(id) {
  return (state.config?.theme_presets || []).find((p) => p.id === id);
}
function themeVar(key) {
  return key === 'background' ? '--bg' : key === 'tabbar' ? '--tabbar' : `--${key}`;
}
async function applyTheme() {
  const cfg = state.config;
  if (!cfg) return;
  const theme = cfg.theme || 'dark_accent';
  const isPreset = theme.startsWith('preset:');
  const preset = isPreset ? presetById(theme.slice(7)) : null;
  const base = isPreset ? (preset?.base || 'dark_accent') : (BUILTIN_THEMES.includes(theme) ? theme : 'dark_accent');

  // Theme classes live on <html> so the inline head script can pre-apply them
  // before first paint (no FOUC). dark_accent is the CSS default: no class.
  document.documentElement.classList.remove('theme-dark', 'theme-light');
  if (base === 'dark' || base === 'light') document.documentElement.classList.add(`theme-${base}`);
  try { localStorage.setItem('integra.theme', base); } catch { /* best effort */ }
  applyTaskbar();

  // Custom colors must live on <body> inline style: a class rule on body
  // would otherwise override values inherited from <html>.
  const styleTarget = document.body.style;
  for (const key of THEME_COLOR_KEYS) styleTarget.removeProperty(themeVar(key));
  const liveColors = preset ? { ...(preset.colors || {}), ...(state.themeEdit || {}) } : state.themeEdit;
  if (liveColors) {
    for (const key of THEME_COLOR_KEYS) {
      const value = liveColors[key];
      if (value) styleTarget.setProperty(themeVar(key), value);
    }
  }

  // background images: preset values, then live edits while customizing
  styleTarget.setProperty('--tabbar-image', 'none');
  styleTarget.setProperty('--content-image', 'none');
  const imgSlots = [['tabbar_image', '--tabbar-image'], ['content_image', '--content-image']];
  if (preset) {
    for (const [slot, key] of imgSlots) {
      const path = preset[slot];
      if (path) {
        const url = await imageDataUrl(path);
        if (url) styleTarget.setProperty(key, `url("${url}")`);
      }
    }
  }
  for (const [slot, key] of imgSlots) {
    const path = state.themeEdit?.[slot];
    if (path) {
      const url = await imageDataUrl(path);
      if (url) styleTarget.setProperty(key, `url("${url}")`);
    }
  }
  requestAnimationFrame(moveIndicator);
}
async function imageDataUrl(path) {
  if (state.imageCache[path]) return state.imageCache[path];
  try {
    const raw = await T.invoke('read_image_data_url', { path });
    const url = await optimizeImageDataUrl(raw);
    state.imageCache[path] = url;
    return url;
  } catch { return null; }
}
function loadImage(src) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error('decode failed'));
    img.src = src;
  });
}
/** Downscale huge images so WebView2 reliably paints them as a CSS background. */
async function optimizeImageDataUrl(raw) {
  const MAX_SIDE = 2560;
  const img = await loadImage(raw);
  const maxSide = Math.max(img.naturalWidth, img.naturalHeight);
  if (maxSide <= MAX_SIDE) return raw;
  const k = MAX_SIDE / maxSide;
  const w = Math.max(1, Math.round(img.naturalWidth * k));
  const h = Math.max(1, Math.round(img.naturalHeight * k));
  const c = document.createElement('canvas');
  c.width = w; c.height = h;
  const ctx = c.getContext('2d');
  if (!ctx) return raw;
  ctx.drawImage(img, 0, 0, w, h);
  return c.toDataURL('image/jpeg', 0.88);
}
function applyTaskbar() {
  const cfg = state.config;
  const taskbar = cfg?.taskbar || { position: 'top', style: 'full', show_labels: true };
  document.body.classList.remove('pos-top', 'pos-bottom', 'pos-left', 'pos-right', 'style-full', 'style-compact', 'style-icons', 'labels-on');
  document.body.classList.add(`pos-${taskbar.position}`, `style-${taskbar.style}`);
  if (taskbar.show_labels) document.body.classList.add('labels-on');
  requestAnimationFrame(moveIndicator);
}
function moveIndicator() {
  const btn = document.querySelector('.tab-btn.active');
  const bar = document.getElementById('taskbar');
  const ind = document.getElementById('tab-indicator');
  if (!btn || !bar) { if (ind) ind.style.opacity = 0; return; }
  const b = btn.getBoundingClientRect();
  const r = bar.getBoundingClientRect();
  const vertical = document.body.classList.contains('pos-left') || document.body.classList.contains('pos-right');
  if (vertical) {
    const leftEdge = document.body.classList.contains('pos-left');
    ind.style.left = (leftEdge ? 0 : r.width - 3) + 'px';
    ind.style.width = '3px';
    ind.style.top = (b.top - r.top) + 'px';
    ind.style.height = b.height + 'px';
  } else {
    const bottom = document.body.classList.contains('pos-bottom');
    ind.style.top = bottom ? '0px' : (r.height - 3) + 'px';
    ind.style.height = '3px';
    ind.style.left = (b.left - r.left) + 'px';
    ind.style.width = b.width + 'px';
  }
  ind.style.opacity = 1;
}

/* ── tabs ── */
function buildTabs() {
  const box = document.getElementById('tab-buttons');
  box.innerHTML = '';
  for (const tab of TABS) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'tab-btn';
    btn.id = `tab-${tab.id}`;
    btn.dataset.page = tab.id;
    btn.setAttribute('role', 'tab');
    btn.setAttribute('aria-controls', `page-${tab.id}`);
    btn.innerHTML = `<span class="ico">${tab.ico}</span><span class="tab-label">${esc(t('nav.' + tab.id))}</span>`;
    btn.onclick = () => show(tab.id);
    box.appendChild(btn);
  }
  // pair each panel with its tab (aria-controls ↔ panel id)
  document.querySelectorAll('#content .page').forEach((panel) => {
    panel.setAttribute('role', 'tabpanel');
    panel.setAttribute('aria-labelledby', `tab-${panel.id.slice(5)}`);
  });
  syncTabsA11y();
  applyI18n(box);
}
/** aria-selected + roving tabindex for the role="tab" buttons. */
function syncTabsA11y() {
  const tabs = [...document.querySelectorAll('#tab-buttons .tab-btn')];
  for (const tab of tabs) {
    const active = tab.dataset.page === state.page;
    tab.setAttribute('aria-selected', active ? 'true' : 'false');
    tab.tabIndex = active ? 0 : -1;
  }
}
/** Keyboard navigation on the tablist: ArrowLeft/Right, Home, End. */
document.getElementById('tab-buttons').addEventListener('keydown', (e) => {
  const tabs = [...document.querySelectorAll('#tab-buttons .tab-btn')];
  if (!tabs.length) return;
  const cur = tabs.findIndex((b) => b.dataset.page === state.page);
  let next = -1;
  if (e.key === 'ArrowRight') next = (cur + 1) % tabs.length;
  else if (e.key === 'ArrowLeft') next = (cur - 1 + tabs.length) % tabs.length;
  else if (e.key === 'Home') next = 0;
  else if (e.key === 'End') next = tabs.length - 1;
  if (next < 0) return;
  e.preventDefault();
  tabs[next].focus();
  show(tabs[next].dataset.page);
});
async function show(page) {
  state.page = page;
  document.querySelectorAll('.tab-btn').forEach((b) => b.classList.toggle('active', b.dataset.page === page));
  document.querySelectorAll('.page').forEach((p) => { p.hidden = p.id !== `page-${page}`; });
  syncTabsA11y();
  moveIndicator();

  // netmon polling only lives while its tab is open
  if (page === 'apps') startNetmonPolling();
  else stopNetmonPolling();

  // dev-mode easter egg: 10 quick clicks on the settings tab
  if (page === 'settings') {
    state.devClicks += 1;
    clearTimeout(state.devClickTimer);
    state.devClickTimer = setTimeout(() => { state.devClicks = 0; }, 2500);
    if (!state.config.dev_mode && state.devClicks >= 5) {
      toast(t('dev.hint', { count: 10 - state.devClicks }), 'warn', 1600);
    }
    if (state.devClicks >= 10) {
      state.devClicks = 0;
      if (!state.config.dev_mode) {
        await api('dev_enable');
        state.config.dev_mode = true;
        toast(t('dev.enabled'), 'ok');
        renderSettings();
      } else {
        toast(t('dev.already'), 'info');
      }
    }
  }

  switch (page) {
    case 'dashboard': await refreshProfiles(); break;
    case 'apps': await refreshNetmonOnce(); break;
    case 'lists': await refreshLists(); break;
    case 'diagnostics': await refreshDiag(); break;
    case 'dns': await refreshDnsPage(); break;
    case 'tgproxy': await refreshTgPage(); break;
    case 'extension': await refreshBridge(); break;
    case 'logs': await refreshLogs(); break;
    case 'settings': renderSettings(); break;
    case 'analysis': await ensureAnalysis(); bindNetcheck(); break;
  }
}

/* ═══ DASHBOARD ═══ */
async function refreshProfiles() {
  state.profiles = await api('list_profiles');
  renderProfiles();
  renderBenchProfileSelect();
}
function statusPill(view) {
  if (view.status === 'running') return `<span class="pill run">● ${esc(t('dashboard.status_running'))}${view.pid ? ' · ' + view.pid : ''}</span>`;
  if (view.status === 'service') return `<span class="pill ok">✓ ${esc(t('dashboard.status_service'))}</span>`;
  return `<span class="pill">${esc(t('dashboard.status_stopped'))}</span>`;
}
function healthPill(view) {
  const map = {
    Ready: ['ok', 'profile.health_ready'],
    MissingPath: ['warn', 'profile.health_missing_path'],
    MissingFolder: ['err', 'profile.health_missing_folder'],
    NoStrategies: ['warn', 'profile.health_no_strategies'],
    NoWinws: ['err', 'profile.health_no_winws'],
    NotAZapretFolder: ['err', 'profile.health_not_zapret'],
  };
  const [cls, key] = map[view.health?.state || 'MissingFolder'];
  return `<span class="pill ${cls}">${esc(t(key))}</span>`;
}
function renderProfiles() {
  const grid = document.getElementById('profiles-grid');
  if (!state.profiles.length) {
    grid.innerHTML = `<div class="card center-col"><p class="muted">${esc(t('dashboard.no_profiles'))}</p></div>`;
    return;
  }
  grid.innerHTML = state.profiles.map((p) => {
    const running = p.status === 'running' || p.status === 'service';
    const isActive = state.config.active_profile === p.id;
    const serviceBtn = p.service_installed
      ? `<button class="btn danger" data-act="service-remove" data-id="${p.id}">${esc(t('dashboard.remove_service'))}</button>`
      : `<button class="btn ghost" data-act="service-install" data-id="${p.id}" ${running ? 'disabled' : ''}>${esc(t('dashboard.install_service'))}</button>`;
    return `
    <div class="card profile-card">
      <div class="profile-head">
        <h4 title="${esc(p.name)}">${esc(p.name)}</h4>
        ${isActive ? `<span class="pill run">★ ${esc(t('dashboard.active'))}</span>` : ''}
      </div>
      <div class="profile-path">${esc(p.path)}</div>
      <div class="row gap-s wrap">${statusPill(p)} ${healthPill(p)}</div>
      <label class="field">
        <span class="field-label">${esc(t('dashboard.strategy'))} (${p.strategies_count})</span>
        <select data-act="strategy" data-id="${p.id}">${renderStrategyOptions(p)}</select>
      </label>
      <div class="profile-actions">
        ${running
          ? `<button class="btn danger" data-act="stop" data-id="${p.id}">■ ${esc(t('dashboard.stop'))}</button>`
          : `<button class="btn primary" data-act="start" data-id="${p.id}">▶ ${esc(t('dashboard.start'))}</button>`}
        ${serviceBtn}
        <button class="btn ghost" data-act="open" data-id="${p.id}">${esc(t('dashboard.open_folder'))}</button>
        <button class="btn ghost" data-act="edit" data-id="${p.id}">${esc(t('dashboard.edit'))}</button>
        <button class="btn ghost" data-act="remove" data-id="${p.id}">${esc(t('dashboard.remove_profile'))}</button>
      </div>
      <div class="profile-checks">
        <label class="check"><input type="checkbox" data-act="autostart" data-id="${p.id}" ${p.autostart ? 'checked' : ''}/><span>${esc(t('dashboard.autostart'))}</span></label>
        <label class="check"><input type="checkbox" data-act="shared" data-id="${p.id}" ${p.use_shared_lists ? 'checked' : ''}/><span>${esc(t('profile.use_shared'))}</span></label>
        ${!isActive ? `<label class="check"><input type="checkbox" data-act="activate" data-id="${p.id}"/><span>${esc(t('dashboard.make_active'))}</span></label>` : ''}
      </div>
    </div>`;
  }).join('');
}
function renderStrategyOptions(profile) {
  const strategies = state.strategies[profile.id] || [];
  if (!strategies.length) return `<option value="${esc(profile.selected_strategy)}">${esc(profile.selected_strategy)}</option>`;
  return strategies.map((s) => `<option value="${esc(s.file)}" ${s.file === profile.selected_strategy ? 'selected' : ''}>${esc(s.name)}</option>`).join('');
}
document.getElementById('profiles-grid')?.addEventListener('click', onProfileAction);
document.getElementById('profiles-grid')?.addEventListener('change', onProfileChange);

async function onProfileAction(e) {
  const btn = e.target.closest('button[data-act]');
  if (!btn) return;
  const id = btn.dataset.id;
  const act = btn.dataset.act;
  const profile = state.profiles.find((p) => p.id === id);
  if (!profile) return;
  try {
    if (act === 'start') {
      await api('start_profile', { profileId: id, strategy: profile.selected_strategy });
      await refreshProfiles();
    } else if (act === 'stop') {
      await api('stop_profile', { profileId: id });
      await refreshProfiles();
    } else if (act === 'service-install') {
      await api('install_service', { profileId: id, strategy: profile.selected_strategy });
      toast(t('zapret.service_installed', { running: '' }), 'ok');
      await refreshProfiles();
    } else if (act === 'service-remove') {
      confirmModal(t('dashboard.remove_service') + '?', async () => {
        await api('remove_service', { profileId: id });
        toast(t('zapret.service_removed'), 'ok');
        await refreshProfiles();
      });
    } else if (act === 'open') {
      await api('open_in_explorer', { path: profile.path });
    } else if (act === 'edit') {
      editProfileModal(profile);
    } else if (act === 'remove') {
      confirmModal(t('dashboard.remove_confirm', { name: profile.name }), async () => {
        await api('remove_profile', { profileId: id });
        await refreshProfiles();
      });
    }
  } catch { /* toasted */ }
}
async function onProfileChange(e) {
  const el = e.target;
  const id = el.dataset.id;
  const profile = state.profiles.find((p) => p.id === id);
  if (!profile) return;
  try {
    if (el.dataset.act === 'strategy') {
      profile.selected_strategy = el.value;
      await api('update_profile', { profile });
    } else if (el.dataset.act === 'autostart') {
      profile.autostart = el.checked;
      await api('update_profile', { profile });
    } else if (el.dataset.act === 'shared') {
      profile.use_shared_lists = el.checked;
      await api('update_profile', { profile });
      toast(t(el.checked ? 'lists.source_shared' : 'lists.source_individual'), 'info');
    } else if (el.dataset.act === 'activate' && el.checked) {
      await api('set_active_profile', { profileId: id });
      await refreshProfiles();
    }
  } catch { /* toasted */ }
}
function editProfileModal(profile) {
  openModal({
    title: t('dashboard.edit'),
    body: `
      <label class="field"><span class="field-label">${esc(t('common.name'))}</span>
        <input id="edit-name" value="${esc(profile.name)}" /></label>
      <label class="field mt-8"><span class="field-label">${esc(t('common.path'))}</span>
        <div class="row gap-s"><input id="edit-path" class="grow" value="${esc(profile.path)}" />
        <button class="btn ghost" id="edit-pick">${esc(t('profile.choose_folder'))}</button></div></label>`,
    actions: [
      {
        label: t('common.save'), kind: 'primary',
        onClick: async () => {
          profile.name = document.getElementById('edit-name').value.trim() || profile.name;
          profile.path = document.getElementById('edit-path').value.trim();
          closeModal();
          await api('update_profile', { profile });
          await refreshProfiles();
        },
      },
      { label: t('common.cancel') },
    ],
  });
  document.getElementById('edit-pick').onclick = async () => {
    const dialog = T.dialog();
    if (dialog) {
      const path = await dialog.open({ directory: true });
      if (path) document.getElementById('edit-path').value = path;
    }
  };
}
document.getElementById('btn-add-profile').onclick = () => {
  openModal({
    title: t('profile.new_title'),
    body: `
      <label class="field"><span class="field-label">${esc(t('common.name'))}</span>
        <input id="add-name" placeholder="${esc(t('profile.name_placeholder'))}" /></label>
      <label class="field mt-8"><span class="field-label">${esc(t('common.path'))}</span>
        <div class="row gap-s"><input id="add-path" class="grow" placeholder="${esc(t('profile.path_placeholder'))}" />
        <button class="btn ghost" id="add-pick">${esc(t('profile.choose_folder'))}</button></div></label>`,
    actions: [
      {
        label: t('common.add'), kind: 'primary',
        onClick: async () => {
          const name = document.getElementById('add-name').value.trim();
          const path = document.getElementById('add-path').value.trim();
          closeModal();
          await api('add_profile', { name, path });
          await refreshProfiles();
        },
      },
      { label: t('common.cancel') },
    ],
  });
  document.getElementById('add-pick').onclick = async () => {
    const dialog = T.dialog();
    if (dialog) {
      const path = await dialog.open({ directory: true });
      if (path) document.getElementById('add-path').value = path;
    }
  };
};

/* ═══ APPS (netmon) ═══ */
async function netmonToggle() {
  if (state.netmonRunning) {
    await api('netmon_stop');
    stopNetmonPolling();
  } else {
    const mode = document.getElementById('netmon-filter-mode').value;
    const name = document.getElementById('netmon-filter-name').value.trim();
    const filter = mode === 'name' && name ? { mode: 'name', value: name } : { mode: 'all' };
    await api('netmon_start', { filter });
    state.netmonEntries = [];
    startNetmonPolling();
  }
  await refreshNetmonOnce();
}
function netmonPollMs() {
  const v = Number(state.config?.netmon?.poll_interval_ms);
  return Number.isFinite(v) && v >= 200 ? v : 1200;
}
function startNetmonPolling() {
  stopNetmonPolling(); // guard against duplicate concurrent timers
  state.netmonTimer = setInterval(() => refreshNetmonOnce().catch(() => {}), netmonPollMs());
}
function stopNetmonPolling() {
  if (state.netmonTimer) clearInterval(state.netmonTimer);
  state.netmonTimer = null;
}
async function refreshNetmonOnce() {
  const status = await api('netmon_status');
  state.netmonRunning = status.running;
  if (state.netmonRunning) {
    [state.netmonSummary, state.netmonEntries] = await Promise.all([
      api('netmon_summary'),
      api('netmon_entries', { limit: 400 }),
    ]);
  }
  renderNetmon();
}
function renderNetmon() {
  const toggleBtn = document.getElementById('btn-netmon-toggle');
  toggleBtn.innerHTML = state.netmonRunning ? `■ ${esc(t('netmon.stop'))}` : `▶ ${esc(t('netmon.start'))}`;
  toggleBtn.classList.toggle('danger', state.netmonRunning);
  toggleBtn.classList.toggle('primary', !state.netmonRunning);

  const stateEl = document.getElementById('netmon-state');
  stateEl.className = `pill ${state.netmonRunning ? 'run' : ''}`;
  stateEl.textContent = state.netmonRunning ? t('netmon.tracking') : t('netmon.idle');
  document.getElementById('netmon-name-wrap').style.display =
    document.getElementById('netmon-filter-mode').value === 'name' ? '' : 'none';

  // summary cards
  const summary = document.getElementById('netmon-summary');
  if (!state.netmonSummary.length) {
    summary.innerHTML = `<div class="card"><p class="muted small">${esc(t('netmon.no_entries'))}</p></div>`;
  } else {
    summary.innerHTML = state.netmonSummary.map((a, i) => `
      <div class="card traffic-card" data-idx="${i}">
        <div class="traffic-head">
          <span class="dot ${a.active ? 'pulse' : ''}"></span>
          <span class="name">${esc(a.process)}</span>
          <span class="pill ${a.active ? 'run' : ''}">${a.active ? esc(t('netmon.active_now')) + ' · ' : 'PID '}${a.pid}</span>
        </div>
        <div class="traffic-stats">
          <span>${esc(t('netmon.connections'))}: <b>${a.connections}</b></span>
          <span>${esc(t('netmon.hits'))}: <b>${a.hits}</b></span>
          ${a.local_ips.length ? `<span>${esc(t('netmon.local_ips'))}: <b>${esc(a.local_ips.join(', '))}</b></span>` : ''}
        </div>
        ${a.top_remotes.length ? `<div class="traffic-stats"><span>${esc(t('netmon.top_hosts'))}: <b>${esc(a.top_remotes.join(' · '))}</b></span></div>` : ''}
      </div>`).join('');
  }

  // table — incremental, keyed by pid+process+proto+remote
  const filter = state.netmonTableFilter.toLowerCase();
  const rows = state.netmonEntries.filter((e) =>
    !filter || (e.process + ' ' + (e.remote || '') + ' ' + (e.host || '')).toLowerCase().includes(filter)).slice(-300);
  const countEl = document.getElementById('netmon-count');
  const countText = `${rows.length} / ${state.netmonEntries.length}`;
  if (countEl.textContent !== countText) countEl.textContent = countText;
  renderNetmonTable(rows);
}
function netmonRowKey(e) {
  return `${e.pid}|${e.process}|${e.proto}|${e.remote || e.local || ''}`;
}
function netmonRowData(e) {
  return [e.process, String(e.pid), e.proto, e.remote || e.local || '', e.host || '', String(e.hits), e.host || e.remote_ip || ''];
}
let netmonRowEls = new Map();
let netmonSnapshot = '';
function renderNetmonTable(rows) {
  // skip every DOM write while the serialized snapshot matches the previous one
  const snapshot = rows.map((e) => netmonRowKey(e) + '\u0000' + netmonRowData(e).join('\u0001')).join('\u0002');
  if (snapshot === netmonSnapshot) return;
  netmonSnapshot = snapshot;
  const tbody = document.querySelector('#netmon-table tbody');
  const used = new Set();
  for (const e of rows) {
    const key = netmonRowKey(e);
    const data = netmonRowData(e);
    used.add(key);
    let tr = netmonRowEls.get(key);
    if (!tr) {
      tr = document.createElement('tr');
      tr.dataset.key = key;
      for (let i = 0; i < 6; i++) tr.appendChild(document.createElement('td'));
      const actions = document.createElement('td');
      const mk = (glyph, titleKey, listId) => {
        const b = document.createElement('button');
        b.type = 'button';
        b.className = 'btn ghost icon-only';
        b.textContent = glyph;
        b.title = t(titleKey);
        b.setAttribute('aria-label', t(titleKey));
        b.dataset.list = listId;
        return b;
      };
      actions.appendChild(mk('＋', 'netmon.add_bypass', 'list-general'));
      actions.appendChild(mk('－', 'netmon.add_exclude', 'list-exclude'));
      tr.appendChild(actions);
      netmonRowEls.set(key, tr);
    }
    // patch only the changed cells
    const cells = tr.children;
    for (let i = 0; i < 6; i++) {
      if (cells[i].textContent !== data[i]) cells[i].textContent = data[i];
    }
    for (const b of cells[6].querySelectorAll('button[data-host]')) {
      if (b.dataset.host !== data[6]) b.dataset.host = data[6];
    }
    tbody.appendChild(tr); // keeps the row order in sync (moves existing nodes)
  }
  for (const [key, tr] of netmonRowEls) {
    if (!used.has(key)) { tr.remove(); netmonRowEls.delete(key); }
  }
}
document.getElementById('btn-netmon-toggle').onclick = () => netmonToggle().catch(() => {});
document.getElementById('btn-netmon-csv').onclick = async () => {
  const dialog = T.dialog();
  const path = dialog ? await dialog.save({ defaultPath: 'integra-session.csv' }) : 'integra-session.csv';
  if (!path) return;
  await api('netmon_export_csv', { path });
  toast(t('common.success'), 'ok');
};
document.getElementById('netmon-filter-mode').onchange = () => renderNetmon();
document.getElementById('netmon-table-filter').oninput = (e) => { state.netmonTableFilter = e.target.value; renderNetmon(); };
document.getElementById('netmon-table').addEventListener('click', async (e) => {
  const btn = e.target.closest('button[data-host]');
  if (!btn || !btn.dataset.host) return;
  if (!state.config.active_profile) { toast(t('lists.no_active_profile'), 'warn'); return; }
  try {
    const report = await api('list_add', { profileId: state.config.active_profile, listId: btn.dataset.list, entries: [btn.dataset.host] });
    toast(t('netmon.added_to_list', { domain: report.added[0] || btn.dataset.host, list: btn.dataset.list }), 'ok');
  } catch { /* toasted */ }
});

/* ── app card context menu (right-click) ── */
let ctxMenuCleanup = null;
function closeAppMenu() {
  if (ctxMenuCleanup) { ctxMenuCleanup(); ctxMenuCleanup = null; }
}
document.getElementById('netmon-summary').addEventListener('contextmenu', (e) => {
  const card = e.target.closest('.traffic-card');
  if (!card) return;
  e.preventDefault();
  const app = state.netmonSummary[Number(card.dataset.idx)];
  if (!app) return;
  openAppMenu(e.clientX, e.clientY, app);
});

function openAppMenu(x, y, app) {
  closeAppMenu();
  const menu = document.createElement('div');
  menu.className = 'ctx-menu';
  menu.setAttribute('role', 'menu');

  const items = [
    { label: t('netmon.add_bypass'), act: () => openIpPicker(app, 'list-general') },
    { label: t('netmon.add_exclude'), act: () => openIpPicker(app, 'list-exclude') },
    null,
    {
      label: t('netmon.copy_ip'),
      act: async () => {
        const addrs = app.top_remotes || [];
        if (!addrs.length) { toast(t('netmon.no_addresses'), 'warn'); return; }
        const ok = await copyToClipboard(addrs.join('\n'));
        toast(ok ? t('common.copied') : t('common.copy_failed'), ok ? 'ok' : 'err');
      },
    },
  ];
  for (const it of items) {
    if (!it) {
      const sep = document.createElement('div');
      sep.className = 'ctx-sep';
      menu.appendChild(sep);
      continue;
    }
    const btn = document.createElement('button');
    btn.className = 'ctx-item';
    btn.setAttribute('role', 'menuitem');
    btn.textContent = it.label;
    btn.onclick = () => { closeAppMenu(); it.act(); };
    menu.appendChild(btn);
  }
  document.body.appendChild(menu);
  const r = menu.getBoundingClientRect();
  menu.style.left = Math.max(8, Math.min(x, innerWidth - r.width - 8)) + 'px';
  menu.style.top = Math.max(8, Math.min(y, innerHeight - r.height - 8)) + 'px';

  const onDocDown = (ev) => { if (!menu.contains(ev.target)) closeAppMenu(); };
  const onKey = (ev) => { if (ev.key === 'Escape') closeAppMenu(); };
  const onScroll = () => closeAppMenu();
  ctxMenuCleanup = () => {
    menu.remove();
    document.removeEventListener('mousedown', onDocDown, true);
    document.removeEventListener('keydown', onKey);
    window.removeEventListener('blur', closeAppMenu);
    document.removeEventListener('scroll', onScroll, true);
  };
  setTimeout(() => {
    if (!ctxMenuCleanup) return;
    document.addEventListener('mousedown', onDocDown, true);
    document.addEventListener('keydown', onKey);
    window.addEventListener('blur', closeAppMenu);
    document.addEventListener('scroll', onScroll, true);
  }, 0);
}

function openIpPicker(app, listId) {
  const addrs = (app.top_remotes || []).filter((s) => s && s.trim());
  const listName = t('lists.name_' + listId);
  if (!addrs.length) { toast(t('netmon.no_addresses'), 'warn'); return; }
  const body = `
    <div class="ip-pick-list" id="ip-pick-list">
      ${addrs.map((a) => `<label class="ip-pick-row"><input type="checkbox" data-addr="${esc(a)}" checked /><span class="ip-pick-addrs">${esc(a)}</span></label>`).join('')}
    </div>
    <span class="ip-pick-count muted small">${esc(listName)} · ${addrs.length}</span>`;
  openModal({
    title: t('netmon.pick_title', { list: listName }),
    body,
    actions: [
      {
        label: t('netmon.select_all'),
        onClick: () => {
          document.querySelectorAll('#ip-pick-list input[type="checkbox"]').forEach((cb) => { cb.checked = true; });
          syncAddBtn();
        },
      },
      {
        label: t('netmon.add_selected'), kind: 'primary',
        onClick: async () => {
          const sel = [...document.querySelectorAll('#ip-pick-list input[type="checkbox"]:checked')].map((cb) => cb.dataset.addr);
          closeModal();
          if (!sel.length) { toast(t('netmon.no_addresses'), 'warn'); return; }
          if (!state.config.active_profile) { toast(t('lists.no_active_profile'), 'warn'); return; }
          try {
            const report = await api('list_add', { profileId: state.config.active_profile, listId, entries: sel });
            if (report.added.length) {
              toast(t('netmon.added_count', { count: report.added.length, list: listName }), 'ok');
            } else {
              toast(t('lists.report', { added: 0, dups: report.duplicates.length, invalid: report.invalid.length }), 'warn');
            }
          } catch { /* toasted */ }
        },
      },
      { label: t('common.cancel') },
    ],
  });
  const addBtn = document.getElementById('modal-actions').children[1];
  function syncAddBtn() {
    const picked = document.querySelectorAll('#ip-pick-list input[type="checkbox"]:checked').length;
    if (addBtn) addBtn.disabled = picked === 0;
  }
  document.getElementById('ip-pick-list').addEventListener('change', syncAddBtn);
  syncAddBtn();
}

/* ═══ LISTS ═══ */
const LIST_SPECS = [
  ['list-general', 'lists.name_list-general'],
  ['list-exclude', 'lists.name_list-exclude'],
  ['list-google', 'lists.name_list-google'],
  ['ipset-all', 'lists.name_ipset-all'],
  ['ipset-exclude', 'lists.name_ipset-exclude'],
];
async function refreshLists() {
  if (!state.config.active_profile) return;
  state.listMeta = await api('list_meta', { profileId: state.config.active_profile });
  state.listEntries = (await api('list_read', { profileId: state.config.active_profile, listId: state.listId })).entries;
  state.listSelected.clear();
  renderLists();
}
function renderLists() {
  const meta = state.listMeta;
  const profile = state.profiles.find((p) => p.id === state.config.active_profile);

  document.getElementById('lists-source-label').textContent =
    meta?.use_shared ? t('lists.source_shared') : t('lists.source_individual');

  document.querySelectorAll('#lists-source-switch button').forEach((b) => {
    b.classList.toggle('active', (b.dataset.shared === '1') === !!meta?.use_shared);
  });

  const tabs = document.getElementById('lists-tabs');
  tabs.innerHTML = LIST_SPECS.map(([id, key]) => {
    const count = meta?.counts?.[id] ?? 0;
    return `<button class="tab-btn ${id === state.listId ? 'active' : ''}" data-list="${id}">
      <span class="ico">${id === state.listId ? '▾' : '▸'}</span>
      <span>${esc(t(key))}</span><span class="pill">${count}</span></button>`;
  }).join('');
  tabs.querySelectorAll('button').forEach((b) => {
    b.onclick = async () => { state.listId = b.dataset.list; await refreshLists(); };
  });

  const filter = state.listFilter.toLowerCase();
  const rows = state.listEntries.filter((e) => !filter || e.toLowerCase().includes(filter));
  document.getElementById('lists-count').textContent = t('lists.entries', { count: state.listEntries.length });
  document.querySelector('#lists-table tbody').innerHTML = rows.map((entry, i) => `
    <tr>
      <td><input type="checkbox" data-entry="${esc(entry)}" ${state.listSelected.has(entry) ? 'checked' : ''}/></td>
      <td>${i + 1}</td>
      <td>${esc(entry)}</td>
    </tr>`).join('') || `<tr><td colspan="3" class="muted">${esc(t('lists.empty'))}</td></tr>`;
}
document.getElementById('lists-source-switch').addEventListener('click', async (e) => {
  const btn = e.target.closest('button[data-shared]');
  if (!btn) return;
  const useShared = btn.dataset.shared === '1';
  const profile = state.profiles.find((p) => p.id === state.config.active_profile);
  if (!profile || profile.use_shared_lists === useShared) return;
  profile.use_shared_lists = useShared;
  await api('update_profile', { profile });
  await refreshProfiles();
  await refreshLists();
});
document.getElementById('lists-filter').oninput = (e) => { state.listFilter = e.target.value; renderLists(); };
document.getElementById('lists-entries-wrap').addEventListener('change', (e) => {
  const cb = e.target;
  if (cb.type === 'checkbox' && cb.dataset.entry) {
    if (cb.checked) state.listSelected.add(cb.dataset.entry);
    else state.listSelected.delete(cb.dataset.entry);
  }
});
document.getElementById('btn-lists-add').onclick = async () => {
  const entries = document.getElementById('lists-add-input').value.split('\n').map((s) => s.trim()).filter(Boolean);
  if (!entries.length) return;
  if (!state.config.active_profile) { toast(t('lists.no_active_profile'), 'warn'); return; }
  try {
    const report = await api('list_add', { profileId: state.config.active_profile, listId: state.listId, entries });
    document.getElementById('lists-add-input').value = '';
    document.getElementById('lists-report').textContent = t('lists.report', {
      added: report.added.length, dups: report.duplicates.length, invalid: report.invalid.length,
    });
    await refreshLists();
  } catch { /* toasted */ }
};
document.getElementById('btn-lists-remove-selected').onclick = async () => {
  for (const entry of [...state.listSelected]) {
    await api('list_remove', { profileId: state.config.active_profile, listId: state.listId, entry });
  }
  await refreshLists();
};
document.getElementById('btn-lists-replace').onclick = () => {
  openModal({
    title: t('lists.add_entries'),
    body: `<textarea id="replace-area" rows="10">${esc(state.listEntries.join('\n'))}</textarea>`,
    actions: [
      {
        label: t('common.apply'), kind: 'primary',
        onClick: async () => {
          const lines = document.getElementById('replace-area').value.split('\n');
          // destructive: confirm first, then run with a busy state on the opener
          confirmModal(t('lists.replace_confirm'), async () => {
            await withBusy(document.getElementById('btn-lists-replace'), async () => {
              try {
                await api('list_replace', { profileId: state.config.active_profile, listId: state.listId, lines });
                await refreshLists();
                toast(t('common.success'), 'ok');
              } catch { /* toasted by api() */ }
            });
          });
        },
      },
      { label: t('common.cancel') },
    ],
  });
};

/* ═══ DIAGNOSTICS ═══ */
async function refreshDiag() {
  state.diagSites = await api('diag_sites');
  renderDiagSites();
  await refreshBench();
}
function renderDiagSites() {
  document.getElementById('diag-sites-list').innerHTML = state.diagSites.map((s) => `
    <span class="chip">${esc(s.name)} <span class="muted">${esc(s.host)}:${s.port}</span>
      ${s.id.startsWith('default') ? '' : `<span class="x" role="button" aria-label="${esc(t('common.close'))}" data-site="${esc(s.id)}">×</span>`}</span>`).join('');
  document.querySelectorAll('#diag-sites-list .x').forEach((x) => {
    x.onclick = async () => {
      state.diagSites = await api('diag_remove_site', { siteId: x.dataset.site });
      renderDiagSites();
    };
  });
}
document.getElementById('btn-diag-add-site').onclick = async () => {
  const name = document.getElementById('diag-site-name').value.trim();
  const host = document.getElementById('diag-site-host').value.trim();
  const port = parseInt(document.getElementById('diag-site-port').value, 10) || 443;
  if (!host) return;
  try {
    state.diagSites = await api('diag_add_site', { name, host, port });
    document.getElementById('diag-site-name').value = '';
    document.getElementById('diag-site-host').value = '';
    renderDiagSites();
  } catch { /* toasted */ }
};
document.getElementById('btn-diag-run').onclick = async () => {
  if (state.diagBusy) return;
  state.diagBusy = true;
  const btn = document.getElementById('btn-diag-run');
  btn.disabled = true;
  btn.textContent = t('diag.running');
  try {
    const kinds = ['dns', 'tcp', 'tls', 'http'].filter((k) => document.getElementById('k-' + k).checked);
    const rounds = parseInt(document.getElementById('diag-rounds').value, 10) || 2;
    const timeoutMs = (parseInt(document.getElementById('diag-timeout').value, 10) || 4) * 1000;
    state.diagResults = await api('diag_run', { kinds, rounds, timeoutMs });
    renderDiagResults();
  } finally {
    state.diagBusy = false;
    btn.disabled = false;
    btn.textContent = t('diag.run');
  }
};
function renderDiagResults() {
  const box = document.getElementById('diag-results');
  if (!state.diagResults) { box.innerHTML = ''; return; }
  box.innerHTML = state.diagResults.map(({ site, outcomes }) => `
    <div class="card site-card">
      <div class="site-head"><span class="dot ${outcomes.every((o) => o.ok) ? 'ok' : 'err'}"></span><b>${esc(site.name)}</b></div>
      <div class="kinds">${outcomes.map((o) => `
        <span class="kind ${o.ok ? 'ok' : 'err'}" title="${esc(o.error || '')}">
          ${o.kind.toUpperCase()} ${o.ok ? '✓' : '✕'}</span>`).join('')}</div>
      <div class="lat">${outcomes.filter((o) => o.ok && o.latency).map((o) =>
        `${o.kind.toUpperCase()}: ${t('diag.latency_ms', { ms: o.latency.avg_ms })} (${t('diag.min')} ${o.latency.min_ms} / ${t('diag.max')} ${o.latency.max_ms})`).join(' · ')}</div>
    </div>`).join('');
}

/* ── benchmark ── */
function renderBenchProfileSelect() {
  const select = document.getElementById('bench-profile');
  if (!select) return;
  select.innerHTML = state.profiles.map((p) => `<option value="${p.id}" ${p.id === state.config.active_profile ? 'selected' : ''}>${esc(p.name)}</option>`).join('');
}
async function refreshBench() {
  state.bench = await api('benchmark_status');
  renderBench();
}
function renderBench() {
  const { running, progress, result } = state.bench || {};
  document.getElementById('btn-bench-start').disabled = running;
  document.getElementById('btn-bench-cancel').hidden = !running;
  const stateEl = document.getElementById('bench-state');
  const bar = document.getElementById('bench-progress-bar');
  if (running && progress) {
    stateEl.textContent = t('bench.running', { strategy: progress.strategy, index: progress.index, total: progress.total })
      + ' · ' + ({ starting: '', warmup: t('bench.warmup'), checking: t('bench.checking'), restoring: t('bench.restore') }[progress.phase] || '');
    bar.hidden = false;
    bar.querySelector('.progress-fill').style.width = progress.total ? `${(progress.index / progress.total) * 100}%` : '5%';
  } else if (result) {
    stateEl.textContent = result.cancelled ? t('bench.cancelled') : t('bench.done', { strategy: result.best || '—' });
    bar.hidden = true;
  } else {
    stateEl.textContent = '';
    bar.hidden = true;
  }
  const box = document.getElementById('bench-results');
  if (!result || !result.entries.length) { box.innerHTML = ''; return; }
  box.innerHTML = result.entries.map((entry) => `
    <div class="bench-row ${entry.strategy === result.best ? 'best' : ''}">
      <span class="score">${entry.score}</span>
      <span class="strategy grow">${entry.strategy === result.best ? '★ ' : ''}${esc(entry.strategy)}</span>
      <span class="lat">${esc(t('bench.success'))}: ${entry.success_ratio}%${entry.avg_latency_ms ? ' · ' + t('bench.avg_latency') + ': ' + t('diag.latency_ms', { ms: entry.avg_latency_ms }) : ''}${entry.error ? ' · ' + esc(entry.error) : ''}</span>
      <div class="bench-bar"><div style="width:${entry.score}%"></div></div>
    </div>`).join('');
}
document.getElementById('btn-bench-start').onclick = async () => {
  const profileId = document.getElementById('bench-profile').value;
  await withBusy(document.getElementById('btn-bench-start'), async () => {
    try {
      await api('benchmark_start', { profileId });
      await refreshBench();
    } catch { /* toasted */ }
  });
};
document.getElementById('btn-bench-cancel').onclick = () => api('benchmark_cancel').catch(() => {});

/* ═══ EXTENSION ═══ */
async function refreshBridge() {
  state.bridge = await api('bridge_status');
  renderBridge();
  if (state.bridge.running && !state.pairing) await refreshPairing();
}
async function refreshPairing() {
  state.pairing = await api('bridge_pairing');
  renderBridge();
  clearInterval(state.pairingTimer);
  state.pairingTimer = setInterval(() => {
    const p = state.pairing;
    if (!p) { clearInterval(state.pairingTimer); state.pairingTimer = null; return; }
    if (p.expires_in > 0) p.expires_in -= 1; // clamp at 0 — never negative
    const el = document.getElementById('pairing-expires');
    if (el) el.textContent = t('extension.expires_in', { seconds: p.expires_in });
    updatePairButtons();
    if (p.expires_in <= 0) {
      clearInterval(state.pairingTimer);
      state.pairingTimer = null;
      if (state.page === 'extension') refreshPairing().catch(() => {});
    }
  }, 1000);
}
/** the pair button stays disabled while there is no valid (unexpired) code */
function updatePairButtons() {
  const valid = !!state.pairing && state.pairing.expires_in > 0;
  const copyBtn = document.getElementById('btn-pair-copy');
  if (copyBtn) copyBtn.disabled = !valid;
}
function renderBridge() {
  const bridge = state.bridge || { running: false, port: 0 };
  const stateEl = document.getElementById('bridge-state');
  stateEl.className = `pill big ${bridge.running ? 'ok' : 'err'}`;
  stateEl.textContent = bridge.running
    ? t('extension.bridge_running', { port: bridge.port })
    : t('extension.bridge_stopped');
  const btn = document.getElementById('btn-bridge-toggle');
  btn.textContent = bridge.running ? t('extension.disable_bridge') : t('extension.enable_bridge');
  btn.classList.toggle('danger', bridge.running);
  btn.classList.toggle('primary', !bridge.running);
  document.getElementById('pairing-box').hidden = !bridge.running;
  if (state.pairing) {
    document.getElementById('pairing-code').textContent = state.pairing.code;
    document.getElementById('pairing-expires').textContent = t('extension.expires_in', { seconds: state.pairing.expires_in });
  }
  updatePairButtons();
  document.getElementById('bridge-instructions').textContent = t('extension.instructions');
}
document.getElementById('btn-bridge-toggle').onclick = async () => {
  await withBusy(document.getElementById('btn-bridge-toggle'), async () => {
    try {
      if (state.bridge?.running) {
        await api('bridge_stop');
        state.pairing = null;
      } else {
        await api('bridge_start');
      }
      await refreshBridge();
    } catch { /* toasted */ }
  });
};
document.getElementById('btn-pair-regen').onclick = async () => {
  await withBusy(document.getElementById('btn-pair-regen'), async () => {
    try {
      state.pairing = await api('bridge_regenerate_code');
      renderBridge();
      toast(t('extension.code_regenerated'), 'ok', 1400);
    } catch { /* toasted */ }
  });
};
async function copyToClipboard(text) {
  try { await navigator.clipboard.writeText(text); return true; } catch { /* fall through */ }
  const ta = document.createElement('textarea');
  ta.value = text;
  ta.style.cssText = 'position:fixed;top:0;left:0;opacity:0';
  document.body.appendChild(ta);
  ta.select();
  let ok = false;
  try { ok = document.execCommand('copy'); } catch { /* noop */ }
  ta.remove();
  return ok;
}
document.getElementById('btn-pair-copy').onclick = async () => {
  const code = state.pairing?.code;
  if (!code) return;
  const ok = await copyToClipboard(code);
  if (ok) toast(t('common.copied'), 'ok', 1200);
  else toast(t('common.copy_failed'), 'err');
};

/* ═══ LOGS ═══ */
const LOG_LEVELS = ['ALL', 'DEBUG', 'INFO', 'SUCCESS', 'WARN', 'ERROR'];
const LOG_LIMIT = 500;
async function refreshLogs() {
  state.logs = await api('logs_snapshot', { limit: 400, level: state.logLevel });
  renderLogFilters();
  renderLogs();
}
function renderLogFilters() {
  // clicks are handled by the single delegated listener below — no per-button
  // onclick here (it used to trigger refreshLogs twice per click)
  const box = document.getElementById('logs-levels');
  box.innerHTML = LOG_LEVELS.map((l) => `<button type="button" data-level="${l}" class="${l === state.logLevel ? 'active' : ''}">${l}</button>`).join('');
}
function logLineHtml(entry) {
  return `<div class="log-line l-${esc(entry.level)}">
    <span class="ts">${esc(new Date(entry.ts).toLocaleTimeString())}</span>
    <span class="lvl">[${esc(entry.level)}]</span>
    <span class="src">[${esc(entry.source)}]</span>
    ${esc(entry.message)}
  </div>`;
}
function logsStickToBottom(box) {
  return document.getElementById('logs-autoscroll').checked &&
    box.scrollHeight - box.scrollTop - box.clientHeight < 40;
}
/** full rebuild — only on refresh (filter change / tab enter) or clear */
function renderLogs() {
  const box = document.getElementById('logs-list');
  box.innerHTML = state.logs.map(logLineHtml).join('') || `<div class="log-line muted">${esc(t('logs.empty'))}</div>`;
  logRendered = state.logs.length;
  if (document.getElementById('logs-autoscroll').checked) box.scrollTop = box.scrollHeight;
}
let logRendered = 0; // how many entries of state.logs are already in the DOM
/** live path: append the new entry, trim overflow beyond the limit from the top */
function appendLogEntry(entry) {
  const box = document.getElementById('logs-list');
  const stick = logsStickToBottom(box); // autoscroll only if already at the bottom
  const placeholder = box.querySelector('.log-line.muted');
  if (placeholder) placeholder.remove();
  box.insertAdjacentHTML('beforeend', logLineHtml(entry));
  logRendered += 1;
  const lines = box.querySelectorAll('.log-line');
  const overflow = lines.length - LOG_LIMIT;
  if (overflow > 0) {
    for (let i = 0; i < overflow; i++) lines[i].remove();
    logRendered -= overflow;
  }
  if (stick) box.scrollTop = box.scrollHeight;
}
document.getElementById('btn-logs-clear').onclick = async () => { await api('logs_clear'); state.logs = []; renderLogs(); };
document.getElementById('btn-logs-export').onclick = async () => {
  const dialog = T.dialog();
  const path = dialog ? await dialog.save({ defaultPath: 'integra-log.txt' }) : 'integra-log.txt';
  if (!path) return;
  await api('logs_export', { path });
  toast(t('common.success'), 'ok');
};
document.getElementById('logs-levels').addEventListener('click', (e) => {
  const btn = e.target.closest('button[data-level]');
  if (btn) { state.logLevel = btn.dataset.level; refreshLogs(); }
});

/* ═══ SETTINGS ═══ */
function colorRow(key, value) {
  return `
    <div class="color-row">
      <input type="color" value="${value || '#888888'}" data-color="${key}" ${value ? '' : 'disabled'} />
      <span class="grow">${esc(t('theme.color_' + key))}</span>
      <span class="reset" role="button" aria-label="${esc(t('theme.reset'))}" data-reset="${key}" title="↺">↺</span>
    </div>`;
}
async function renderSettings() {
  const cfg = state.config;
  const presets = (cfg.theme_presets || []).map((p) => `
    <span class="chip preset-chip ${cfg.theme === 'preset:' + p.id ? 'active' : ''}" data-preset="${p.id}" title="${esc(t('theme.apply'))}">
      ${esc(p.name)}<span class="x" role="button" aria-label="${esc(t('common.close'))}" data-del-preset="${p.id}">×</span></span>`).join('');

  const info = await api('app_info');
  const taskbar = cfg.taskbar;

  document.getElementById('settings-body').innerHTML = `
  <div class="settings-grid">
    <div class="card settings-card">
      <div class="settings-row"><span class="label">${esc(t('settings.language'))}</span>
        <select id="set-lang">
          <option value="ru" ${cfg.language === 'ru' ? 'selected' : ''}>Русский</option>
          <option value="en" ${cfg.language === 'en' ? 'selected' : ''}>English</option>
        </select></div>
      <div class="settings-row"><span class="label">${esc(t('settings.log_level'))}</span>
        <select id="set-loglevel">${LOG_LEVELS.map((l) => `<option value="${l}" ${cfg.log_level === l ? 'selected' : ''}>${l}</option>`).join('')}</select></div>
      <div class="settings-row"><span class="label">${esc(t('settings.close_to_tray'))}</span>
        <input type="checkbox" id="set-tray" ${cfg.close_to_tray ? 'checked' : ''}/></div>
      <div class="settings-row"><span class="label">${esc(t('settings.start_minimized'))}</span>
        <input type="checkbox" id="set-minimized" ${cfg.start_minimized ? 'checked' : ''}/></div>
      <div class="settings-row"><span class="label">${esc(t('settings.autostart_windows'))}</span>
        <input type="checkbox" id="set-autostart" ${cfg.autostart_windows ? 'checked' : ''}/></div>
      <div class="settings-row"><span class="label">${esc(t('settings.config_path'))}</span>
        <button class="btn ghost" id="btn-reveal-config">${esc(t('settings.open_config'))}</button></div>
      <div class="settings-row"><span class="label">${esc(t('settings.components'))}</span>
        <button class="btn ghost" id="btn-check-components">${esc(t('settings.check_components'))}</button></div>
      <p class="muted small">${esc(info.config_path)}</p>
      ${info.is_admin ? '' : `<button class="btn ghost" id="btn-admin">${esc(t('app.relaunch_admin'))}</button>`}
    </div>

    <div class="card settings-card">
      <h3>${esc(t('theme.customize'))}</h3>
      <p class="muted small">${esc(t('theme.hint'))}</p>
      <div class="settings-row"><span class="label">${esc(t('theme.base'))}</span>
        <select id="theme-base">${BUILTIN_THEMES.map((th) => `<option value="${th}" ${!cfg.theme.startsWith('preset:') && cfg.theme === th ? 'selected' : ''}>${esc(t('settings.theme_' + th))}</option>`).join('')}</select></div>
      <div class="color-grid mt-8">${THEME_COLOR_KEYS.map((key) => colorRow(key, state.themeEdit?.[key] || currentColorValue(key))).join('')}</div>
      <h3 class="mt-8">${esc(t('theme.images'))}</h3>
      <div class="settings-row"><span class="label">${esc(t('theme.tabbar_image'))}</span>
        <span class="row gap-s">
          <button class="btn ghost" id="pick-tabbar-img">${esc(t('theme.choose_image'))}</button>
          <button class="btn ghost" id="clear-tabbar-img">${esc(t('theme.clear_image'))}</button>
        </span></div>
      <div class="img-preview" id="preview-tabbar"></div>
      <div class="settings-row"><span class="label">${esc(t('theme.content_image'))}</span>
        <span class="row gap-s">
          <button class="btn ghost" id="pick-content-img">${esc(t('theme.choose_image'))}</button>
          <button class="btn ghost" id="clear-content-img">${esc(t('theme.clear_image'))}</button>
        </span></div>
      <div class="img-preview" id="preview-content"></div>
      <p class="muted small mt-8">${esc(t('theme.image_size_hint'))}</p>
      <h3 class="mt-8">${esc(t('theme.presets'))}</h3>
      <div class="chips">${presets || `<span class="muted small">—</span>`}</div>
      <div class="row gap-s mt-8">
        <input id="preset-name" class="grow" placeholder="${esc(t('theme.preset_name'))}" />
        <button class="btn primary" id="btn-save-preset">${esc(t('theme.save_preset'))}</button>
        <button class="btn ghost" id="btn-reset-theme">${esc(t('theme.reset'))}</button>
      </div>
    </div>

    <div class="card settings-card">
      <h3>${esc(t('taskbar.appearance'))}</h3>
      <div class="settings-row"><span class="label">${esc(t('taskbar.position'))}</span>
        <div class="segmented" id="tb-position">
          ${['top', 'bottom', 'left', 'right'].map((p) => `<button data-pos="${p}" class="${taskbar.position === p ? 'active' : ''}">${esc(t('taskbar.pos_' + p))}</button>`).join('')}
        </div></div>
      <div class="settings-row"><span class="label">${esc(t('taskbar.style'))}</span>
        <div class="segmented" id="tb-style">
          ${['full', 'compact', 'icons'].map((s) => `<button data-style="${s}" class="${taskbar.style === s ? 'active' : ''}">${esc(t('taskbar.style_' + s))}</button>`).join('')}
        </div></div>
      <div class="settings-row"><span class="label">${esc(t('taskbar.labels'))}</span>
        <input type="checkbox" id="tb-labels" ${taskbar.show_labels ? 'checked' : ''}/></div>
    </div>

    <div class="card settings-card">
      <h3>${esc(t('netmon.title'))}</h3>
      <div class="settings-row"><span class="label">${esc(t('netmon.poll_interval'))}</span>
        <input type="number" id="set-poll" min="200" max="30000" step="100" value="${cfg.netmon.poll_interval_ms}" class="w-24"/></div>
      <div class="settings-row"><span class="label">${esc(t('netmon.resolve_hosts'))}</span>
        <input type="checkbox" id="set-resolve" ${cfg.netmon.resolve_hosts ? 'checked' : ''}/></div>
      <div class="settings-row"><span class="label">${esc(t('netmon.auto_new'))}</span>
        <input type="checkbox" id="set-autonew" ${cfg.netmon.auto_include_new_processes ? 'checked' : ''}/></div>
      <div class="settings-row"><span class="label">${esc(t('settings.bridge_port'))}</span>
        <input type="number" id="set-bridge-port" min="1024" max="65535" value="${cfg.bridge.port}" class="w-24"/></div>
    </div>

    <div class="card settings-card">
      <h3>${esc(t('settings.maintenance'))}</h3>
      <p class="muted small">${esc(t('maint.auto_note'))}</p>
      <div class="settings-row"><span class="label">${esc(t('settings.auto_check'))}</span>
        <input type="checkbox" id="set-autocheck" ${cfg.auto_check_zapret ? 'checked' : ''}/></div>
      <div class="settings-row"><span class="label">${esc(t('settings.auto_update'))}</span>
        <input type="checkbox" id="set-autoupdate" ${cfg.auto_update_zapret ? 'checked' : ''}/></div>
      <div class="settings-row"><span class="label">${esc(t('common.version'))}</span>
        <span id="maint-version" class="muted small">…</span></div>
      <div class="row gap-s mt-8">
        <button class="btn ghost" id="btn-maint-check">${esc(t('maint.check'))}</button>
        <button class="btn primary" id="btn-maint-install" hidden></button>
      </div>
      <div class="progress" id="maint-progress" hidden><div class="progress-fill"></div></div>
      <p class="muted small" id="maint-status"></p>
    </div>

    ${cfg.dev_mode ? `
    <div class="card settings-card" style="border-color: color-mix(in srgb, var(--danger) 40%, transparent);">
      <h3>${esc(t('dev.title'))} ⚠</h3>
      <p class="muted small">${esc(t('dev.logs_desc'))}</p>
      <div class="row gap-s">
        <button class="btn primary" id="btn-dev-logs">${esc(t('dev.open_logs'))}</button>
        <button class="btn ghost" id="btn-dev-disable">${esc(t('common.disabled'))}</button>
      </div>
      <p class="muted small mt-8">${esc(info.dev_log_path)}</p>
    </div>` : ''}

    <div class="card settings-card">
      <h3>${esc(t('settings.about'))}</h3>
      <p class="muted small">${esc(t('settings.about_text'))}</p>
      <div class="settings-row"><span class="label">${esc(t('settings.developer'))}</span><span>6yxoiAngel</span></div>
      <div class="settings-row"><span class="label">${esc(t('settings.tester'))}</span><span>Mamont</span></div>
      <p class="muted small">v${esc(info.version)} · ${esc(info.os)}</p>
    </div>
  </div>`;

  bindSettings(info);
  await refreshMaintVersion();
  // initialize image previews from the active preset
  const activePreset = presetById(state.config.theme.startsWith('preset:') ? state.config.theme.slice(7) : '');
  if (activePreset) {
    for (const [slot, elId] of [['tabbar_image', 'preview-tabbar'], ['content_image', 'preview-content']]) {
      const path = activePreset[slot];
      if (path) {
        const url = await imageDataUrl(path);
        if (url) document.getElementById(elId).style.backgroundImage = `url("${url}")`;
      }
    }
  }
}

let analysisLoaded = false;
async function ensureAnalysis() {
  const body = document.getElementById('analysis-full');
  if (!body || analysisLoaded) return;
  try {
    const r = await fetch('analysis-content.html');
    if (!r.ok) throw new Error(r.status);
    body.innerHTML = await r.text();
    applyI18n(body); // translate the freshly injected document
    analysisLoaded = true;
  } catch (_) {
    body.innerHTML = `<div class="card"><p class="muted">${esc(t('analysis.load_failed'))}</p></div>`;
  }
}

let netcheckBound = false;
function bindNetcheck() {
  const btn = document.getElementById('btn-netcheck');
  if (!btn || netcheckBound) return;
  netcheckBound = true;
  btn.onclick = async () => {
    const st = document.getElementById('netcheck-state');
    const box = document.getElementById('netcheck-results');
    st.textContent = t('analysis.check_running');
    box.innerHTML = '';
    try {
      const r = await api('netcheck_run');
      st.textContent = t('analysis.verdict_label', { verdict: r.verdict });
      box.innerHTML = r.probes.map((p) => `
        <div class="settings-row">
          <span class="label">${esc(p.name)}</span>
          <span>${p.ok ? '✅' : '⛔'}</span>
          <span class="muted small">${esc(p.detail)}</span>
        </div>`).join('') + `<p class="muted small mt-8">${esc(r.tip)}</p>`;
    } catch (e) {
      st.textContent = t('analysis.error_label', { error: e?.message || e });
    }
  };
}

function currentColorValue(key) {
  // best-effort live value from the computed style for the color picker
  const value = getComputedStyle(document.body).getPropertyValue(themeVar(key)).trim();
  if (value.startsWith('#') && (value.length === 7 || value.length === 4)) return value;
  return null;
}

function bindSettings(info) {
  const cfg = state.config;
  const $ = (id) => document.getElementById(id);

  $('set-lang').onchange = async (e) => {
    await api('set_language', { language: e.target.value });
    state.config = await api('get_config');
    state.bundle = await api('i18n_bundle');
    document.documentElement.lang = e.target.value;
    applyI18n();
    buildTabs();
    document.querySelector('.tab-btn.active')?.classList.add('active');
    show(state.page);
  };
  $('set-loglevel').onchange = (e) => patchSettings({ logLevel: e.target.value });
  $('set-tray').onchange = (e) => patchSettings({ closeToTray: e.target.checked });
  $('set-minimized').onchange = (e) => patchSettings({ startMinimized: e.target.checked });
  $('set-autostart').onchange = (e) => patchSettings({ autostartWindows: e.target.checked });
  $('btn-reveal-config').onclick = () => api('reveal_config').catch(() => {});
  $('btn-check-components').onclick = () => openSetup().catch(() => {});
  $('btn-admin') && ($('btn-admin').onclick = () => api('relaunch_as_admin').catch(() => {}));

  $('theme-base').onchange = async (e) => {
    state.themeEdit = null;
    cfg.theme = e.target.value;
    await api('set_theme', { theme: cfg.theme });
    await applyTheme();
    renderSettings();
  };
  document.querySelectorAll('[data-color]').forEach((input) => {
    input.oninput = () => {
      if (input.disabled) return;
      state.themeEdit = state.themeEdit || {};
      state.themeEdit[input.dataset.color] = input.value;
      applyTheme();
    };
  });
  document.querySelectorAll('[data-reset]').forEach((x) => {
    x.onclick = () => {
      if (state.themeEdit) delete state.themeEdit[x.dataset.reset];
      applyTheme();
      renderSettings();
    };
  });
  const pickImage = async (slot) => {
    try {
      const dialog = T.dialog();
      if (!dialog) { toast(t('theme.image_dialog_missing'), 'err'); return; }
      const path = await dialog.open({ multiple: false, filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp'] }] });
      if (!path) return;
      const raw = await T.invoke('read_image_data_url', { path });
      const img = await loadImage(raw);
      const iw = img.naturalWidth, ih = img.naturalHeight;
      if (Math.max(iw, ih) > 2560) {
        state.themeEdit = state.themeEdit || {};
        state.themeEdit[slot] = path;
        openCropModal(slot, path, raw, img);
        return;
      }
      state.themeEdit = state.themeEdit || {};
      state.themeEdit[slot] = path;
      await applyTheme();
      const url = await imageDataUrl(path);
      if (url) {
        $(slot === 'tabbar_image' ? 'preview-tabbar' : 'preview-content').style.backgroundImage = `url("${url}")`;
        toast(t('theme.image_applied', { w: iw, h: ih }), 'ok', 1800);
      } else {
        delete state.themeEdit[slot];
        toast(t('theme.image_read_failed'), 'err');
      }
    } catch (e) {
      toast(t('theme.image_pick_failed'), 'err');
    }
  };

  const openCropModal = (slot, path, raw, img) => {
    const iw = img.naturalWidth, ih = img.naturalHeight;
    const maxW = Math.min(880, Math.max(320, innerWidth - 120));
    const maxH = Math.min(520, Math.max(240, innerHeight - 280));
    const k = Math.min(maxW / iw, maxH / ih, 1);
    const cw = Math.max(1, Math.round(iw * k));
    const ch = Math.max(1, Math.round(ih * k));
    const body = `
      <p class="muted small">${esc(t('theme.image_crop_hint', { w: iw, h: ih }))}</p>
      <div class="crop-wrap" data-crop-wrap style="position:relative;width:${cw}px;height:${ch}px;max-width:88vw">
        <canvas id="crop-canvas" width="${cw}" height="${ch}" style="display:block;width:${cw}px;height:${ch}px;border-radius:10px"></canvas>
        <div id="crop-sel" style="position:absolute;border:2px solid var(--accent);background:rgba(255,255,255,.12);box-shadow:0 0 0 9999px rgba(0,0,0,.45);cursor:move"></div>
      </div>`;
    openModal({
      title: t('theme.image_crop_title'),
      body,
      actions: [
        { label: t('theme.image_crop_apply'), kind: 'primary', onClick: async () => {
            const sel = document.getElementById('crop-sel');
            if (!sel) return;
            const sx = parseInt(sel.dataset.x, 10), sy = parseInt(sel.dataset.y, 10);
            const sw = parseInt(sel.dataset.w, 10), sh = parseInt(sel.dataset.h, 10);
            if (!sw || !sh) return;
            const out = document.createElement('canvas');
            out.width = Math.max(1, Math.round(sw / k));
            out.height = Math.max(1, Math.round(sh / k));
            const octx = out.getContext('2d');
            if (!octx) return;
            octx.drawImage(img, Math.round(sx / k), Math.round(sy / k), out.width, out.height, 0, 0, out.width, out.height);
            const dataUrl = out.toDataURL('image/jpeg', 0.9);
            try {
              const savedPath = await api('save_background_image', { dataUrl });
              state.themeEdit = state.themeEdit || {};
              state.themeEdit[slot] = savedPath;
              closeModal();
              await applyTheme();
              const url = await imageDataUrl(savedPath);
              if (url) {
                $(slot === 'tabbar_image' ? 'preview-tabbar' : 'preview-content').style.backgroundImage = `url("${url}")`;
                toast(t('theme.image_cropped'), 'ok', 1800);
              } else {
                delete state.themeEdit[slot];
                toast(t('theme.image_read_failed'), 'err');
              }
            } catch {
              toast(t('theme.image_save_failed'), 'err');
            }
        } },
        { label: t('theme.image_crop_skip'), onClick: () => {
            closeModal();
            optimizeImageDataUrl(raw).then(async (u) => {
              state.themeEdit = state.themeEdit || {};
              state.themeEdit[slot] = path;
              await applyTheme();
              if (u) {
                $(slot === 'tabbar_image' ? 'preview-tabbar' : 'preview-content').style.backgroundImage = `url("${u}")`;
                toast(t('theme.image_applied', { w: iw, h: ih }), 'ok', 1800);
              }
            });
        } },
      ],
    });
    const canvas = document.getElementById('crop-canvas');
    const ctx = canvas.getContext('2d');
    ctx.drawImage(img, 0, 0, cw, ch);
    const sel = document.getElementById('crop-sel');
    const sx0 = Math.round(cw * 0.08), sy0 = Math.round(ch * 0.12);
    const sw0 = Math.round(cw * 0.84), sh0 = Math.round(ch * 0.72);
    Object.assign(sel.dataset, { x: sx0, y: sy0, w: sw0, h: sh0 });
    const applySel = () => {
      const x = parseInt(sel.dataset.x, 10), y = parseInt(sel.dataset.y, 10);
      const w = parseInt(sel.dataset.w, 10), h = parseInt(sel.dataset.h, 10);
      sel.style.left = x + 'px'; sel.style.top = y + 'px';
      sel.style.width = w + 'px'; sel.style.height = h + 'px';
    };
    applySel();
    let mode = null, ox = 0, oy = 0;
    sel.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      const r = sel.getBoundingClientRect();
      mode = (e.clientX > r.right - 18 && e.clientY > r.bottom - 18) ? 'resize' : 'move';
      ox = e.clientX; oy = e.clientY;
      sel.setPointerCapture(e.pointerId);
    });
    sel.addEventListener('pointermove', (e) => {
      if (!mode) return;
      e.preventDefault();
      const dx = e.clientX - ox, dy = e.clientY - oy;
      ox = e.clientX; oy = e.clientY;
      let x = parseInt(sel.dataset.x, 10), y = parseInt(sel.dataset.y, 10);
      let w = parseInt(sel.dataset.w, 10), h = parseInt(sel.dataset.h, 10);
      if (mode === 'move') {
        x = Math.min(Math.max(0, x + dx), cw - w);
        y = Math.min(Math.max(0, y + dy), ch - h);
      } else {
        w = Math.min(Math.max(60, w + dx), cw - x);
        h = Math.min(Math.max(40, h + dy), ch - y);
      }
      Object.assign(sel.dataset, { x, y, w, h });
      applySel();
    });
    const endDrag = () => { mode = null; };
    sel.addEventListener('pointerup', endDrag);
    sel.addEventListener('pointercancel', endDrag);
  };
  $('pick-tabbar-img').onclick = () => pickImage('tabbar_image');
  $('pick-content-img').onclick = () => pickImage('content_image');
  $('clear-tabbar-img').onclick = () => { if (state.themeEdit) delete state.themeEdit.tabbar_image; $('preview-tabbar').style.backgroundImage = ''; applyTheme(); };
  $('clear-content-img').onclick = () => { if (state.themeEdit) delete state.themeEdit.content_image; $('preview-content').style.backgroundImage = ''; applyTheme(); };

  document.querySelectorAll('[data-preset]').forEach((chip) => {
    chip.onclick = async (e) => {
      if (e.target.dataset.delPreset) return;
      cfg.theme = 'preset:' + chip.dataset.preset;
      state.themeEdit = null;
      await api('set_theme', { theme: cfg.theme });
      await applyTheme();
      renderSettings();
      toast(t('theme.preset_applied'), 'ok');
    };
  });
  document.querySelectorAll('[data-del-preset]').forEach((x) => {
    x.onclick = async (e) => {
      e.stopPropagation();
      const preset = presetById(x.dataset.delPreset);
      // destructive: confirm before deleting a saved theme preset
      confirmModal(t('theme.preset_delete_confirm', { name: preset?.name || x.dataset.delPreset }), async () => {
        try {
          await api('theme_delete_preset', { presetId: x.dataset.delPreset });
          state.config = await api('get_config');
          await applyTheme();
          renderSettings();
          toast(t('theme.preset_deleted'), 'info');
        } catch { /* toasted */ }
      });
    };
  });
  $('btn-save-preset').onclick = async () => {
    const name = $('preset-name').value.trim();
    if (!name) { toast(t('theme.name_required'), 'warn'); return; }
    await api('theme_save_preset', {
      name,
      base: $('theme-base').value,
      colors: (() => {
        const c = { ...(state.themeEdit || {}) };
        delete c.tabbar_image;
        delete c.content_image;
        return c;
      })(),
      tabbarImage: state.themeEdit?.tabbar_image ?? null,
      contentImage: state.themeEdit?.content_image ?? null,
    });
    state.themeEdit = null;
    state.config = await api('get_config');
    await applyTheme();
    renderSettings();
    toast(t('theme.preset_saved', { name }), 'ok');
  };
  $('btn-reset-theme').onclick = async () => {
    state.themeEdit = null;
    cfg.theme = 'dark_accent';
    await api('set_theme', { theme: cfg.theme });
    await applyTheme();
    renderSettings();
    toast(t('theme.reset_done'), 'info');
  };

  $('tb-position').onclick = async (e) => {
    const btn = e.target.closest('button[data-pos]');
    if (!btn) return;
    cfg.taskbar.position = btn.dataset.pos;
    await patchSettings({ taskbar: cfg.taskbar });
    renderSettings();
  };
  $('tb-style').onclick = async (e) => {
    const btn = e.target.closest('button[data-style]');
    if (!btn) return;
    cfg.taskbar.style = btn.dataset.style;
    await patchSettings({ taskbar: cfg.taskbar });
    renderSettings();
  };
  $('tb-labels').onchange = async (e) => {
    cfg.taskbar.show_labels = e.target.checked;
    await patchSettings({ taskbar: cfg.taskbar });
  };

  $('set-poll').onchange = (e) => patchSettings({ netmonPollMs: parseInt(e.target.value, 10) || 1000 });
  $('set-resolve').onchange = (e) => patchSettings({ netmonResolveHosts: e.target.checked });
  $('set-autonew').onchange = (e) => patchSettings({ netmonAutoNew: e.target.checked });
  $('set-bridge-port').onchange = (e) => patchSettings({ bridgePort: parseInt(e.target.value, 10) || 8765 });
  $('set-autocheck').onchange = (e) => patchSettings({ autoCheckZapret: e.target.checked });
  $('set-autoupdate').onchange = (e) => patchSettings({ autoUpdateZapret: e.target.checked });

  $('btn-maint-check').onclick = checkUpdates;
  $('btn-maint-install').onclick = installUpdate;
  if (cfg.dev_mode) {
    $('btn-dev-logs').onclick = async () => {
      try { await api('open_dev_logs'); toast(t('dev.opened'), 'ok'); }
      catch { /* toasted */ }
    };
    $('btn-dev-disable').onclick = async () => {
      await api('dev_disable');
      cfg.dev_mode = false;
      renderSettings();
    };
  }
}

async function patchSettings(patch) {
  state.config = await api('update_settings', { patch });
  applyTaskbar();
  if (patch.taskbar) {
    toast(t('taskbar.saved'), 'ok', 1400);
    moveIndicator();
  }
}

async function refreshMaintVersion() {
  const profileId = state.config.active_profile;
  const el = document.getElementById('maint-version');
  if (!profileId || !el) return;
  state.installedVersion = await api('zapret_installed_version', { profileId });
  el.textContent = state.installedVersion || t('maint.not_installed_meta');
}
async function checkUpdates() {
  if (state.maintBusy) return;
  const status = document.getElementById('maint-status');
  status.textContent = '…';
  try {
    state.release = await api('zapret_check_update');
    const btn = document.getElementById('btn-maint-install');
    btn.hidden = false;
    btn.textContent = t('maint.download', { version: state.release.version });
    status.textContent = `${t('maint.latest', { version: state.release.version })}` +
      (state.release.published_at ? ` · ${t('maint.published', { date: state.release.published_at.slice(0, 10) })}` : '');
  } catch {
    status.textContent = '';
  }
}
async function installUpdate() {
  if (state.maintBusy || !state.release?.version) return;
  const profileId = state.config.active_profile;
  if (!profileId) return;
  state.maintBusy = true;
  const bar = document.getElementById('maint-progress');
  bar.hidden = false;
  bar.querySelector('.progress-fill').style.width = '0%';
  // contract C1: the backend now accepts only the version (no url argument)
  await withBusy(document.getElementById('btn-maint-install'), async () => {
    try {
      await api('zapret_install_update', { version: state.release.version });
    } catch (e) {
      toast(t('maint.network_error', { error: e?.message || String(e) }), 'err');
      bar.hidden = true;
    } finally {
      state.maintBusy = false;
    }
  });
}

/* ═══ ROBLOX DNS ═══ */
let dnsInited = false;
async function refreshDnsPage() {
  if (!dnsInited) { dnsInited = true; bindDns(); }
  await refreshDnsStatus();
}
async function refreshDnsStatus() {
  try {
    state.dnsAdapters = await api('dns_status');
    renderDnsStatus();
  } catch { /* toasted */ }
}
function renderDnsStatus() {
  const el = document.getElementById('dns-current');
  if (!el) return;
  const st = state.dnsAdapters;
  const warn = document.getElementById('dns-admin-warn');
  if (warn && st && !st.admin) {
    warn.hidden = false;
    warn.textContent = t('dns.not_admin');
  } else if (warn) warn.hidden = true;
  const adapters = Array.isArray(st?.adapters) ? st.adapters : (st?.adapters ? [st.adapters] : []);
  const doh = Array.isArray(st?.doh) ? st.doh : (st?.doh ? [st.doh] : []);
  let html = '';
  if (!adapters.length) {
    html = `<p class="muted small">${esc(t('dns.no_adapter'))}</p>`;
  } else {
    html = `<div class="dns-list">` + adapters.map((a) => `
      <div class="row gap wrap dns-addr">
        <span class="pill">${esc(a.InterfaceAlias || a.interfacealias || '')}</span>
        <span class="muted small grow">${esc((a.ServerAddresses || a.serveraddresses || '').split(',').join(', '))}</span>
      </div>`).join('') + `</div>`;
  }
  if (doh.length) {
    html += `<div class="muted small" style="margin-top:8px">DoH: ${doh.map((d) => `${esc(d.ServerAddress || d.serveraddress)} → ${esc(d.DohTemplate || d.dohtemplate || '?')}`).join('; ')}</div>`;
  }
  el.innerHTML = html;
}
function renderDnsBusy() {
  const st = document.getElementById('dns-bench-state');
  if (st) st.textContent = state.dnsBusy ? t('dns.bench_running') : '';
  const btn = document.getElementById('btn-dns-bench');
  if (btn) btn.disabled = state.dnsBusy;
}
async function runDnsBench() {
  if (state.dnsBusy) return;
  state.dnsBusy = true;
  renderDnsBusy();
  try {
    state.dnsBench = await api('dns_benchmark');
    renderDnsBench();
    toast(t('dns.bench_done'), 'ok', 1800);
  } catch { /* toasted */ } finally {
    state.dnsBusy = false;
    renderDnsBusy();
  }
}
function renderDnsBench() {
  const box = document.getElementById('dns-results');
  if (!box) return;
  if (!state.dnsBench?.length) { box.hidden = true; box.innerHTML = ''; return; }
  box.hidden = false;
  const top = state.dnsBench[0].score;
  box.innerHTML = state.dnsBench.map((r, i) => `
    <label class="dns-row ${r.score === top ? 'best' : ''}">
      <input type="radio" name="dns-pick" value="${i}" ${r.score === top ? 'checked' : ''} />
      <span class="dns-name">${esc(r.name)}</span>
      <span class="muted small">${esc(r.primary)} · ${esc(r.secondary)}</span>
      <span class="dns-metric">${r.rtt_ms != null ? t('diag.latency_ms', { ms: Math.round(r.rtt_ms) }) : '—'}</span>
      <span class="dns-metric">${r.resolve_ms != null ? t('diag.latency_ms', { ms: Math.round(r.resolve_ms) }) : '—'}</span>
      <span class="dns-metric">${Math.round(r.success_rate * 100)}%</span>
      <span class="dns-metric">${r.roblox_ok ? '✓' : '✕'}</span>
      <span class="dns-score">${r.score.toFixed(1)}${r.score === top ? ' ★' : ''}</span>
    </label>`).join('');
}
async function applyDns() {
  const sel = document.querySelector('input[name="dns-pick"]:checked');
  if (!sel) { toast(t('dns.pick_hint'), 'warn'); return; }
  const r = state.dnsBench[+sel.value];
  const useDoh = document.getElementById('dns-doh')?.checked ?? false;
  await withBusy(document.getElementById('btn-dns-apply'), async () => {
    try {
      await api('dns_apply', { primary: r.primary, secondary: r.secondary, use_doh: useDoh });
      toast(t('dns.applied', { name: r.name, primary: r.primary }), 'ok');
      refreshDnsStatus();
    } catch { /* toasted */ }
  });
}
async function resetDns() {
  confirmModal(t('dns.reset_confirm'), async () => {
    try {
      await api('dns_reset');
      toast(t('dns.resetted'), 'ok');
      refreshDnsStatus();
    } catch { /* toasted */ }
  }, true);
}
async function findRbx() {
  if (state.rbxBusy) return;
  state.rbxBusy = true;
  const stEl = document.getElementById('rbx-state');
  if (stEl) stEl.textContent = t('dns.hosts_analyzing');
  const btn = document.getElementById('btn-rbx-find');
  if (btn) btn.disabled = true;
  try {
    state.rbxCands = await api('roblox_find');
    renderRbx();
  } catch { /* toasted */ } finally {
    state.rbxBusy = false;
    if (btn) btn.disabled = false;
    if (stEl) stEl.textContent = '';
  }
}
function renderRbx() {
  const box = document.getElementById('rbx-results');
  if (!box) return;
  if (!state.rbxCands.length) {
    box.innerHTML = `<p class="muted small">${esc(t('dns.hosts_empty'))}</p>`;
    return;
  }
  box.innerHTML = state.rbxCands.map((c, i) => `
    <div class="dns-row">
      <span class="dns-name">${esc(c.ip)}</span>
      <span class="dns-metric">${t('diag.latency_ms', { ms: Math.round(c.rtt_ms) })}</span>
      <button type="button" class="btn ghost" data-rbx-apply="${i}">${esc(t('dns.hosts_apply'))}</button>
    </div>`).join('');
  box.querySelectorAll('[data-rbx-apply]').forEach((b) => {
    b.onclick = async () => {
      const c = state.rbxCands[+b.dataset.rbxApply];
      await withBusy(b, async () => {
        try {
          await api('roblox_hosts_apply', { ip: c.ip });
          toast(t('dns.hosts_done', { ip: c.ip }), 'ok');
          refreshRbxState();
        } catch { /* toasted */ }
      });
    };
  });
}
async function refreshRbxState() {
  try {
    const active = await api('roblox_hosts_state');
    const btn = document.getElementById('btn-rbx-revert');
    if (btn) btn.hidden = !active;
  } catch { /* toasted */ }
}
async function revertRbx() {
  confirmModal(t('dns.hosts_revert_confirm'), async () => {
    await withBusy(document.getElementById('btn-rbx-revert'), async () => {
      try {
        await api('roblox_hosts_revert');
        toast(t('dns.hosts_reverted'), 'ok');
        refreshRbxState();
      } catch { /* toasted */ }
    });
  }, true);
}
function bindDns() {
  const benchBtn = document.getElementById('btn-dns-bench');
  if (benchBtn) benchBtn.onclick = runDnsBench;
  const applyBtn = document.getElementById('btn-dns-apply');
  if (applyBtn) applyBtn.onclick = applyDns;
  const resetBtn = document.getElementById('btn-dns-reset');
  if (resetBtn) resetBtn.onclick = resetDns;
  const findBtn = document.getElementById('btn-rbx-find');
  if (findBtn) findBtn.onclick = findRbx;
  const revBtn = document.getElementById('btn-rbx-revert');
  if (revBtn) revBtn.onclick = revertRbx;
  refreshRbxState();
}

/* ═══ TG WS PROXY ═══ */
async function refreshTgPage() {
  if (!state.tgBound) { state.tgBound = true; bindTg(); }
  await refreshTg();
  startTgPoll();
}
function startTgPoll() {
  if (state.tgPollTimer) return;
  state.tgPollTimer = setInterval(async () => {
    if (state.page !== 'tgproxy') {
      clearInterval(state.tgPollTimer);
      state.tgPollTimer = null;
      return;
    }
    await refreshTgSoft();
  }, 2000);
}
async function refreshTgSoft() {
  try { state.tg = await api('tgproxy_status'); renderTg(); } catch { /* toasted */ }
}
async function refreshTg() {
  state.tg = await api('tgproxy_status');
  renderTg();
  await refreshTgLogs();
}
function renderTg() {
  const s = state.tg;
  if (!s) return;
  const setText = (id, v) => { const el = document.getElementById(id); if (el) el.textContent = v ?? '—'; };
  setText('tg-version', s.version || '—');
  setText('tg-venv', s.venv_ok ? t('common.yes') : t('common.no'));
  setText('tg-secret', s.secret || '—');
  const linkEl = document.getElementById('tg-link');
  if (linkEl) linkEl.value = s.link || '';
  const pathEl = document.getElementById('tg-path');
  if (pathEl && !pathEl.dataset.userEdited) pathEl.value = s.dir || '';
  const portEl = document.getElementById('tg-port');
  if (portEl && !portEl.dataset.userEdited) portEl.value = s.port;
  const running = !!s.running;
  const badge = document.getElementById('tg-running-badge');
  if (badge) { badge.hidden = !running; badge.textContent = '● ' + t('tgproxy.running'); }
  const startBtn = document.getElementById('btn-tg-start');
  const stopBtn = document.getElementById('btn-tg-stop');
  if (startBtn) startBtn.hidden = running;
  if (stopBtn) stopBtn.hidden = !running;
  const setupBtn = document.getElementById('btn-tg-setup');
  if (setupBtn) {
    setupBtn.disabled = !!s.setup_running;
    setupBtn.textContent = s.setup_running ? t('tgproxy.setup_running') : t('tgproxy.setup');
  }
  const dirHint = document.getElementById('tg-dir-hint');
  if (dirHint) dirHint.hidden = s.exists;
}
async function startTg() {
  const raw = document.getElementById('tg-port')?.value ?? '';
  const port = Number.parseInt(raw, 10);
  const errEl = document.getElementById('tg-port-error');
  const valid = Number.isInteger(port) && port >= 1 && port <= 65535;
  if (errEl) errEl.hidden = valid;
  if (!valid) { toast(t('common.port_invalid'), 'warn'); return; }
  await withBusy(document.getElementById('btn-tg-start'), async () => {
    try {
      // contract C2: the UI sends only the port. The secret is generated and
      // kept by the backend; the UI renders it exclusively from tgproxy_status.
      await api('tgproxy_start', { port });
      toast(t('tgproxy.started'), 'ok');
      await refreshTg();
    } catch { /* toasted */ }
  });
}
async function stopTg() {
  await withBusy(document.getElementById('btn-tg-stop'), async () => {
    try {
      await api('tgproxy_stop');
      toast(t('tgproxy.stopped'), 'info');
      await refreshTg();
    } catch { /* toasted */ }
  });
}
async function setupTg() {
  try {
    await api('tgproxy_setup');
    toast(t('tgproxy.setup_started'), 'info');
    startTgLogPoll();
  } catch { /* toasted */ }
}
function startTgLogPoll() {
  if (state.tgLogTimer) return;
  state.tgLogTimer = setInterval(async () => {
    if (state.page !== 'tgproxy') {
      clearInterval(state.tgLogTimer);
      state.tgLogTimer = null;
      return;
    }
    const busy = state.tg?.setup_running || state.tg?.running;
    if (!busy) { clearInterval(state.tgLogTimer); state.tgLogTimer = null; return; }
    await refreshTgLogs();
  }, 1500);
}
async function refreshTgLogs() {
  try { state.tgLogs = await api('tgproxy_logs', { n: 250 }); renderTgLogs(); } catch { /* toasted */ }
}
function renderTgLogs() {
  const pre = document.getElementById('tg-logs');
  if (!pre) return;
  pre.textContent = state.tgLogs.join('\n') || '—';
  pre.scrollTop = pre.scrollHeight;
}
function bindTg() {
  const startBtn = document.getElementById('btn-tg-start');
  if (startBtn) startBtn.onclick = startTg;
  const stopBtn = document.getElementById('btn-tg-stop');
  if (stopBtn) stopBtn.onclick = stopTg;
  const setupBtn = document.getElementById('btn-tg-setup');
  if (setupBtn) setupBtn.onclick = setupTg;
  const browseBtn = document.getElementById('btn-tg-browse');
  if (browseBtn) {
    browseBtn.onclick = async () => {
      try {
        const picked = await T.dialog().open({ directory: true, title: t('profile.choose_folder') });
        if (picked && !Array.isArray(picked)) {
          await api('tgproxy_set_path', { path: picked });
          const pathEl = document.getElementById('tg-path');
          if (pathEl) { pathEl.value = picked; pathEl.dataset.userEdited = '1'; }
          toast(t('tgproxy.path_saved'), 'ok', 1800);
          await refreshTg();
        }
      } catch { /* toasted */ }
    };
  }
  const copyBtn = document.getElementById('btn-tg-copy');
  if (copyBtn) {
    copyBtn.onclick = async () => {
      const link = document.getElementById('tg-link')?.value || '';
      if (!link) return;
      try {
        await navigator.clipboard.writeText(link);
        toast(t('common.copied'), 'ok', 1400);
      } catch {
        toast(t('common.copy_failed'), 'err');
      }
    };
  }
  const refreshLogs = document.getElementById('btn-tg-logs-refresh');
  if (refreshLogs) refreshLogs.onclick = refreshTgLogs;
  const pathEl = document.getElementById('tg-path');
  if (pathEl) {
    pathEl.addEventListener('input', () => { pathEl.dataset.userEdited = '1'; });
    pathEl.addEventListener('change', async () => {
      const v = pathEl.value.trim();
      if (v) {
        try { await api('tgproxy_set_path', { path: v }); await refreshTg(); } catch { /* toasted */ }
      }
    });
  }
  const portEl = document.getElementById('tg-port');
  if (portEl) {
    portEl.addEventListener('input', () => {
      portEl.dataset.userEdited = '1';
      const errEl = document.getElementById('tg-port-error');
      if (errEl) errEl.hidden = true;
    });
  }
}

/* ── help ── */
document.querySelectorAll('.help-btn').forEach((btn) => {
  btn.onclick = () => {
    openModal({
      title: t('help.title'),
      body: `<div class="help-text">${esc(t('help.' + btn.dataset.help))}</div>`,
      actions: [{ label: t('help.close'), kind: 'primary' }],
    });
  };
});
document.getElementById('modal-backdrop').addEventListener('click', (e) => {
  if (e.target.id === 'modal-backdrop') closeModal();
});

/* ── backend events ── */
async function bindEvents() {
  await T.listen('logs', (event) => {
    const entry = event.payload;
    if (state.page === 'logs') {
      const levelsMatch = state.logLevel === 'ALL' || entry.level === state.logLevel;
      if (levelsMatch) {
        state.logs.push(entry);
        if (state.logs.length > LOG_LIMIT) state.logs.shift();
        appendLogEntry(entry);
      }
    }
  });
  await T.listen('profile-event', () => {
    if (state.page === 'dashboard') refreshProfiles().catch(() => {});
  });
  await T.listen('benchmark', async (event) => {
    state.bench.progress = event.payload;
    state.bench.running = !['done', 'cancelled'].includes(event.payload.phase);
    if (!state.bench.running) await refreshBench().catch(() => {});
    else if (state.page === 'diagnostics') renderBench();
  });
  await T.listen('zapret-update', (event) => {
    const payload = event.payload;
    if (payload.phase === 'check') return;
    const bar = document.getElementById('maint-progress');
    const status = document.getElementById('maint-status');
    if (payload.phase === 'download' && bar) {
      bar.hidden = false;
      bar.querySelector('.progress-fill').style.width = `${payload.percent}%`;
      if (status) status.textContent = t('maint.downloading', { percent: payload.percent });
    } else if (payload.phase === 'extract' && status) {
      status.textContent = t('maint.extracting');
    } else if (payload.phase === 'done') {
      if (bar) bar.hidden = true;
      if (status) status.textContent = t('maint.updated', { version: payload.version });
      toast(status?.textContent || 'OK', 'ok');
      refreshMaintVersion();
    } else if (payload.phase === 'error') {
      if (bar) bar.hidden = true;
      if (status) status.textContent = payload.message;
      toast(payload.message, 'err');
    }
  });
}

/* ═══ FIRST-RUN SETUP / COMPONENT CHECK ═══ */
async function maybeShowSetup() {
  if (state.config?.first_run_done || state.setupShown) return;
  state.setupShown = true;
  await openSetup();
}
async function openSetup() {
  const checks = await api('system_check');
  document.getElementById('setup-title').textContent = t('setup.title');
  document.getElementById('setup-subtitle').textContent = t('setup.subtitle');
  document.getElementById('setup-continue').textContent = t('setup.continue');
  document.getElementById('setup-list').innerHTML = checks.map((c) => {
    const ok = c.ok;
    const warn = c.id === 'admin' && !ok;
    const ico = ok ? '✓' : warn ? '⚠' : '✕';
    const cls = ok ? 'ok' : warn ? 'warn' : 'err';
    const act = ok ? '' : (warn
      ? `<button class="btn ghost" data-setup-act="admin">${esc(t('setup.action_relaunch'))}</button>`
      : `<button class="btn ghost" data-setup-act="${esc(c.id)}">${esc(t('setup.action_install'))}</button>`);
    return `
      <div class="setup-row">
        <span class="setup-ico ${cls}">${ico}</span>
        <div class="setup-body">
          <div class="setup-name">${esc(t('setup.check_' + c.id))}</div>
          ${c.detail ? `<div class="setup-detail">${esc(c.detail)}</div>` : ''}
          <div class="setup-detail ${cls}">${esc(ok ? t('setup.status_ok') : t('setup.status_missing'))}</div>
        </div>
        ${act}
      </div>`;
  }).join('');
  document.getElementById('setup-overlay').hidden = false;
  document.querySelectorAll('[data-setup-act]').forEach((b) => {
    b.onclick = async () => {
      const act = b.dataset.setupAct;
      try {
        if (act === 'webview2') {
          // contract C3: external http(s) links go through open_url
          await api('open_url', { url: 'https://go.microsoft.com/fwlink/?linkid=2124701' });
        } else if (act === 'vcredist') {
          await api('open_url', { url: 'https://aka.ms/vs/17/release/vc_redist.x64.exe' });
        } else if (act === 'admin') {
          await api('relaunch_as_admin');
        } else if (act === 'zapret') {
          closeSetup();
          await show('settings');
          toast(t('setup.zapret_hint'), 'info', 4000);
        }
      } catch { /* toasted */ }
    };
  });

  // If everything is already installed, remember that and close the window
  // automatically — the next launches load straight into the app.
  const allOk = checks.every((c) => c.ok);
  if (allOk && !state.config?.first_run_done) {
    try {
      await api('complete_first_run');
      state.config.first_run_done = true;
    } catch { /* toasted */ }
    setTimeout(() => {
      if (!document.getElementById('setup-overlay').hidden) {
        closeSetup();
        toast(t('setup.all_ready'), 'ok', 2000);
      }
    }, 1200);
  }
}
function closeSetup() { document.getElementById('setup-overlay').hidden = true; }
document.getElementById('setup-continue').onclick = async () => {
  try {
    await api('complete_first_run');
    state.config.first_run_done = true;
  } catch { /* toasted */ }
  closeSetup();
};

/* ── window geometry persistence ── */
let geometryTimer = null;
function scheduleGeometrySave() {
  clearTimeout(geometryTimer);
  geometryTimer = setTimeout(async () => {
    try {
      const w = window.__TAURI__?.window?.getCurrentWindow?.();
      if (!w) return;
      const pos = await w.outerPosition();
      const size = await w.innerSize();
      const maximized = await w.isMaximized();
      await T.invoke('save_window_state', {
        x: pos.x, y: pos.y, width: size.width, height: size.height, maximized,
      });
    } catch { /* best effort */ }
  }, 800);
}

/* ── boot ── */
async function boot() {
  [state.bundle, state.config, state.appInfo] = await Promise.all([
    api('i18n_bundle'),
    api('get_config'),
    api('app_info'),
  ]);
  const verEl = document.getElementById('brand-version');
  if (verEl && state.appInfo?.version) verEl.textContent = 'v' + state.appInfo.version;
  state.profiles = await api('list_profiles');
  for (const p of state.profiles) {
    try { state.strategies[p.id] = await T.invoke('list_strategies', { profileId: p.id }); } catch { state.strategies[p.id] = []; }
  }
  buildTabs();
  await applyTheme();
  applyI18n();
  await show('dashboard');

  await bindEvents();

  maybeShowSetup().catch(() => {});

  window.addEventListener('resize', moveIndicator);
  window.addEventListener('resize', scheduleGeometrySave);
  document.documentElement.addEventListener('mouseup', scheduleGeometrySave);

  console.info('[integra] UI ready');
}
boot().catch((e) => {
  console.error(e);
  document.body.insertAdjacentHTML('beforeend', `<div class="toast err" style="position:fixed;top:12px;left:12px;z-index:99">boot: ${esc(e?.message || e)}</div>`);
});