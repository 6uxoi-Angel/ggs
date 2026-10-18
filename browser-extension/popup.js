// popup.js — Integra extension popup (ES module).
// Flow: pairing (6-digit code → /pair) → status + profile select →
// per-domain actions and bulk link capture (via the service worker).

import {
  DEFAULT_PORT,
  bridgeRequest,
  clearState,
  isPairCode,
  loadState,
  profileOption,
  qs,
  saveState,
} from './api.js';

// ── i18n (navigator.language, fallback RU) ──────────────────

const MESSAGES = {
  ru: {
    lang: 'ru',
    appSub: 'Оболочка для Zapret',
    pairHint: 'Открой Integra → вкладка «Расширение», скопируй код привязки и введи его здесь. Код действует 15 минут.',
    pairBtn: 'Подключить',
    pairBtnBusy: 'Подключаю…',
    pairFoot: 'Мост работает только на этом компьютере (127.0.0.1).',
    portLabel: 'Не удалось подключиться. Укажи порт моста Integra:',
    portCheck: 'Проверить',
    checking: 'Проверяю…',
    errBadCodeFormat: 'Код привязки — 6 цифр.',
    errBadCode: 'Неверный код привязки. Проверь его в приложении и попробуй снова.',
    errNetwork: 'Integra не запущена или недоступна. Запусти приложение и повтори попытку.',
    errPort: 'Порт {port} не отвечает. Убедись, что Integra запущена, и проверь порт в настройках приложения.',
    errGeneric: 'Что-то пошло не так. Попробуй ещё раз.',
    okPaired: 'Готово! Расширение привязано к Integra.',
    zapretOn: 'Zapret работает',
    zapretOff: 'Zapret остановлен',
    zapretUnknown: 'Состояние неизвестно',
    strategy: 'Стратегия',
    counts: 'Списки',
    countsEmpty: 'списки пусты',
    profile: 'Профиль',
    profileDefault: 'По умолчанию',
    profileLoading: '…',
    target: 'Цель',
    domainPlaceholder: 'example.com (пусто — текущий сайт)',
    addBypass: 'В обход',
    addExclude: 'В исключения',
    remove: 'Убрать',
    capture: 'Захватить все ссылки вкладки',
    captureBusy: 'Собираю ссылки…',
    footPort: 'Порт {port} · API v{api}',
    disconnect: 'Отключить',
    refreshTip: 'Обновить',
    bannerDisconnected: 'Мост недоступен — Integra не запущена. Запусти приложение и нажми «Обновить».',
    bannerTokenStale: 'Привязка недействительна (токен отклонён). Привяжи расширение заново.',
    bannerRestart: 'Изменения сохранены. Перезапусти zapret в Integra, чтобы они применились.',
    rePair: 'Привязать заново',
    addedMsg: 'Добавлено: {a} · дубликаты: {d} · некорректные: {i}',
    removedMsg: 'Убрано из: {lists}',
    removedNone: 'Домен не найден в списках — убирать нечего.',
    capturedNone: 'Ссылки не найдены — на этой странице нечего захватывать.',
    capturedMsg: 'Найдено доменов: {n}. {detail}',
    errEmptyDomain: 'Нет домена: открой сайт или введи адрес вручную.',
    errNoTab: 'Не удалось определить активную вкладку.',
    errInject: 'На этой странице нельзя захватывать ссылки (служебная страница браузера).',
    errAddFailed: 'Не удалось изменить список. Проверь адрес и попробуй ещё раз.',
    countBypass: 'В обход',
    countExclude: 'Исключения',
    countListGeneral: 'Обход (общий)',
    countListExclude: 'Исключённые',
  },
  en: {
    lang: 'en',
    appSub: 'Zapret companion app',
    pairHint: 'Open Integra → the “Extension” tab, copy the pairing code and enter it here. The code is valid for 15 minutes.',
    pairBtn: 'Connect',
    pairBtnBusy: 'Connecting…',
    pairFoot: 'The bridge only listens on this machine (127.0.0.1).',
    portLabel: 'Could not connect. Enter the Integra bridge port:',
    portCheck: 'Check',
    checking: 'Checking…',
    errBadCodeFormat: 'The pairing code is 6 digits.',
    errBadCode: 'Wrong pairing code. Double-check it in the app and try again.',
    errNetwork: 'Integra is not running or unreachable. Start the app and try again.',
    errPort: 'Port {port} is not responding. Make sure Integra is running and check the port in the app settings.',
    errGeneric: 'Something went wrong. Please try again.',
    okPaired: 'Done! The extension is paired with Integra.',
    zapretOn: 'Zapret is running',
    zapretOff: 'Zapret is stopped',
    zapretUnknown: 'Status unknown',
    strategy: 'Strategy',
    counts: 'Lists',
    countsEmpty: 'lists are empty',
    profile: 'Profile',
    profileDefault: 'Default',
    profileLoading: '…',
    target: 'Target',
    domainPlaceholder: 'example.com (empty — current site)',
    addBypass: 'Bypass',
    addExclude: 'Exclude',
    remove: 'Remove',
    capture: 'Capture all links of the tab',
    captureBusy: 'Collecting links…',
    footPort: 'Port {port} · API v{api}',
    disconnect: 'Disconnect',
    refreshTip: 'Refresh',
    bannerDisconnected: 'Bridge unreachable — Integra is not running. Start the app and press “Refresh”.',
    bannerTokenStale: 'Pairing is no longer valid (token rejected). Please pair the extension again.',
    bannerRestart: 'Changes saved. Restart zapret in Integra to apply them.',
    rePair: 'Pair again',
    addedMsg: 'Added: {a} · duplicates: {d} · invalid: {i}',
    removedMsg: 'Removed from: {lists}',
    removedNone: 'The domain was not in any list — nothing to remove.',
    capturedNone: 'No links found — there is nothing to capture on this page.',
    capturedMsg: 'Domains found: {n}. {detail}',
    errEmptyDomain: 'No domain: open a website or type an address manually.',
    errNoTab: 'Could not detect the active tab.',
    errInject: 'Links cannot be captured on this page (a browser service page).',
    errAddFailed: 'Could not update the list. Check the address and try again.',
    countBypass: 'Bypass',
    countExclude: 'Exclusions',
    countListGeneral: 'Bypass (general)',
    countListExclude: 'Excluded',
  },
};

const langKey = (navigator.language || '').toLowerCase().startsWith('en') ? 'en' : 'ru';
const T = MESSAGES[langKey] || MESSAGES.ru; // fallback: RU

function t(key, vars = {}) {
  let s = T[key] ?? key;
  for (const [k, v] of Object.entries(vars)) s = s.replaceAll(`{${k}}`, String(v));
  return s;
}

// ── state / elements ─────────────────────────────────────────

const state = {
  token: null,
  port: DEFAULT_PORT,
  profile: '',
  captureMode: 'bypass',
};
let probedPort = DEFAULT_PORT; // port verified by /health during pairing
let tabHost = null;            // hostname of the active tab
let busy = false;

const $ = (id) => document.getElementById(id);
const els = {
  viewPair: $('view-pair'), viewMain: $('view-main'),
  pairSub: $('pair-sub'), pairHint: $('pair-hint'), pairCode: $('pair-code'),
  pairBtn: $('pair-btn'), pairError: $('pair-error'), pairSuccess: $('pair-success'),
  pairFoot: $('pair-foot'), pairPortRow: $('pair-port-row'), pairPortLabel: $('pair-port-label'),
  pairPort: $('pair-port'), pairPortCheck: $('pair-port-check'),
  zapretDot: $('zapret-dot'), zapretState: $('zapret-state'),
  strategyValue: $('strategy-value'), counts: $('counts'),
  profileSelect: $('profile-select'),
  targetHost: $('target-host'), domainInput: $('domain-input'),
  addBypass: $('add-bypass'), addExclude: $('add-exclude'), removeDomain: $('remove-domain'),
  segBypass: $('seg-bypass'), segExclude: $('seg-exclude'), captureBtn: $('capture-btn'),
  banner: $('banner'), refreshBtn: $('refresh-btn'),
  footMeta: $('foot-meta'), disconnectBtn: $('disconnect-btn'),
  toast: $('toast'),
  labelStrategy: $('label-strategy'), labelCounts: $('label-counts'),
  labelProfile: $('label-profile'), labelTarget: $('label-target'),
};

// ── tiny UI helpers ──────────────────────────────────────────

function show(el) { el.classList.remove('hidden'); }
function hide(el) { el.classList.add('hidden'); }

let toastTimer = null;
function toast(text, kind = '') {
  els.toast.textContent = text;
  els.toast.className = `toast ${kind}`;
  show(els.toast);
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => hide(els.toast), 4200);
}

function showBanner(text, { error = false, actionText = null, onAction = null } = {}) {
  els.banner.textContent = '';
  els.banner.className = `banner ${error ? 'err' : ''}`;
  els.banner.append(document.createTextNode(text));
  if (actionText && onAction) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'link-btn';
    btn.style.marginLeft = '6px';
    btn.textContent = actionText;
    btn.addEventListener('click', onAction);
    els.banner.append(btn);
  }
  show(els.banner);
}

function hideBanner() { hide(els.banner); }

function setBusy(value) {
  busy = value;
  for (const b of [els.pairBtn, els.addBypass, els.addExclude, els.removeDomain, els.captureBtn, els.pairPortCheck]) {
    b.disabled = value;
  }
}

function chip(label, count) {
  const el = document.createElement('span');
  el.className = 'chip';
  el.append(document.createTextNode(`${label}: `));
  const b = document.createElement('b');
  b.textContent = String(count);
  el.append(b);
  return el;
}

function countLabel(key) {
  const map = {
    bypass: 'countBypass', exclude: 'countExclude',
    'list-general': 'countListGeneral', 'list-exclude': 'countListExclude',
  };
  return map[key] ? t(map[key]) : key;
}

// ── pairing view ─────────────────────────────────────────────

function applyStaticTexts() {
  document.documentElement.lang = T.lang;
  els.pairSub.textContent = t('appSub');
  els.pairHint.textContent = t('pairHint');
  els.pairBtn.textContent = t('pairBtn');
  els.pairFoot.textContent = t('pairFoot');
  els.pairPortLabel.textContent = t('portLabel');
  els.pairPortCheck.textContent = t('portCheck');
  els.refreshBtn.title = t('refreshTip');
  els.labelStrategy.textContent = t('strategy');
  els.labelCounts.textContent = t('counts');
  els.labelProfile.textContent = t('profile');
  els.labelTarget.textContent = t('target');
  els.domainInput.placeholder = t('domainPlaceholder');
  els.addBypass.textContent = t('addBypass');
  els.addExclude.textContent = t('addExclude');
  els.removeDomain.textContent = t('remove');
  els.segBypass.textContent = t('addBypass');
  els.segExclude.textContent = t('addExclude');
  els.captureBtn.textContent = t('capture');
  els.disconnectBtn.textContent = t('disconnect');
  document.title = 'Integra';
}

function pairError(text) {
  els.pairError.textContent = text;
  show(els.pairError);
  hide(els.pairSuccess);
}

async function probeHealth(port) {
  const res = await bridgeRequest(port, '/health');
  return !res.network && res.ok;
}

async function probeAndReport() {
  const ok = await probeHealth(probedPort);
  if (ok) {
    hide(els.pairPortRow);
    hide(els.pairError);
  } else {
    els.pairPortLabel.textContent = t('portLabel');
    show(els.pairPortRow);
    pairError(t('errNetwork'));
  }
  return ok;
}

async function checkPort() {
  const port = Number(els.pairPort.value);
  if (!Number.isInteger(port) || port < 1024 || port > 65535) {
    pairError(t('errPort', { port: els.pairPort.value || '?' }));
    return;
  }
  els.pairPortCheck.disabled = true;
  els.pairPortCheck.textContent = t('checking');
  const ok = await probeHealth(port);
  els.pairPortCheck.disabled = false;
  els.pairPortCheck.textContent = t('portCheck');
  if (ok) {
    probedPort = port;
    hide(els.pairPortRow);
    hide(els.pairError);
  } else {
    pairError(t('errPort', { port }));
  }
}

async function doPair() {
  const code = els.pairCode.value.trim();
  if (!isPairCode(code)) {
    pairError(t('errBadCodeFormat'));
    return;
  }
  hide(els.pairError);
  setBusy(true);
  els.pairBtn.textContent = t('pairBtnBusy');

  const res = await bridgeRequest(probedPort, '/pair', { method: 'POST', body: { code } });

  setBusy(false);
  els.pairBtn.textContent = t('pairBtn');

  if (res.network) {
    show(els.pairPortRow);
    pairError(t('errNetwork'));
    return;
  }
  if (res.status === 401) {
    pairError(t('errBadCode'));
    return;
  }
  if (!res.ok || !res.data?.token) {
    pairError(t('errGeneric'));
    return;
  }

  state.token = res.data.token;
  state.port = Number(res.data.port) || probedPort;
  state.profile = '';
  await saveState({ token: state.token, port: state.port, profile: '', apiVersion: res.data.api_version ?? 2 });

  els.pairSuccess.textContent = t('okPaired');
  show(els.pairSuccess);
  hide(els.pairError);
  setTimeout(() => {
    hide(els.viewPair);
    showMain();
    refreshAll();
  }, 550);
}

function showPair() {
  hide(els.viewMain);
  show(els.viewPair);
  els.pairCode.value = '';
  hide(els.pairSuccess);
  hide(els.pairPortRow);
  hide(els.pairError);
  els.pairBtn.textContent = t('pairBtn');
  els.pairCode.focus();
  probeAndReport();
}

// ── main view ────────────────────────────────────────────────

function showMain() {
  hide(els.viewPair);
  show(els.viewMain);
  els.zapretState.textContent = t('profileLoading');
  els.strategyValue.textContent = '—';
  els.footMeta.textContent = t('footPort', { port: state.port, api: '?' });
}

function renderStatus(data) {
  const running = data.zapret_running ?? data.running ?? data.active ?? null;
  els.zapretDot.className = `dot ${running === true ? 'on' : running === false ? 'off' : ''}`;
  els.zapretState.textContent = running === true ? t('zapretOn') : running === false ? t('zapretOff') : t('zapretUnknown');

  const strategy = data.strategy ?? data.strategy_name ?? data.active_strategy ?? data.active_profile;
  els.strategyValue.textContent = strategy ? String(strategy) : '—';

  els.counts.textContent = '';
  const counts = data.counts;
  if (counts && typeof counts === 'object' && Object.keys(counts).length > 0) {
    for (const [key, value] of Object.entries(counts)) {
      if (typeof value === 'number') els.counts.append(chip(countLabel(key), value));
    }
  } else {
    const empty = document.createElement('span');
    empty.className = 'k';
    empty.textContent = t('countsEmpty');
    els.counts.append(empty);
  }

  const port = data.bridge_port ?? state.port;
  const api = data.api_version ?? 2;
  els.footMeta.textContent = t('footPort', { port, api });
}

function renderProfiles(list) {
  const current = state.profile;
  els.profileSelect.textContent = '';
  const def = document.createElement('option');
  def.value = '';
  def.textContent = t('profileDefault');
  els.profileSelect.append(def);

  const seen = new Set(['']);
  for (const entry of list ?? []) {
    const opt = profileOption(entry);
    if (!opt || seen.has(opt.value)) continue;
    seen.add(opt.value);
    const option = document.createElement('option');
    option.value = opt.value;
    option.textContent = opt.label;
    els.profileSelect.append(option);
  }
  els.profileSelect.value = seen.has(current) ? current : '';
  state.profile = els.profileSelect.value;
}

async function refreshAll({ preserveBanner = false } = {}) {
  if (!preserveBanner) hideBanner();

  const { status, ok, data, network } = await bridgeRequest(
    state.port, `/api/status${qs({ profile: state.profile })}`, { token: state.token },
  );

  if (network) {
    showBanner(t('bannerDisconnected'), { error: true });
    els.zapretDot.className = 'dot';
    els.zapretState.textContent = t('zapretUnknown');
    return;
  }
  if (status === 401) {
    showBanner(t('bannerTokenStale'), { error: true, actionText: t('rePair'), onAction: forgetPairing });
    return;
  }
  if (!ok || !data) {
    showBanner(t('errGeneric'), { error: true });
    return;
  }

  hideBanner();
  renderStatus(data);

  const profilesRes = await bridgeRequest(state.port, '/api/profiles', { token: state.token });
  if (profilesRes.ok && profilesRes.data?.profiles) {
    renderProfiles(profilesRes.data.profiles);
  }
}

async function detectTab() {
  try {
    const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
    const url = tab?.url ? new URL(tab.url) : null;
    tabHost = url && (url.protocol === 'http:' || url.protocol === 'https:') ? url.hostname : null;
  } catch {
    tabHost = null;
  }
  updateTarget();
}

function currentTarget() {
  return els.domainInput.value.trim().toLowerCase() || tabHost || '';
}

function updateTarget() {
  const target = currentTarget();
  els.targetHost.textContent = target || '—';
}

function reportOutcome(reports) {
  const { report, capturedCount = null } = reports;
  const added = Array.isArray(report.added) ? report.added.length : 0;
  const duplicates = Array.isArray(report.duplicates) ? report.duplicates.length : 0;
  const invalid = Array.isArray(report.invalid) ? report.invalid.length : 0;

  if (capturedCount !== null) {
    const detail = added > 0 ? t('addedMsg', { a: added, d: duplicates, i: invalid }) : '';
    toast(t('capturedMsg', { n: capturedCount, detail }), added > 0 ? 'ok' : '');
  } else {
    toast(t('addedMsg', { a: added, d: duplicates, i: invalid }), added > 0 ? 'ok' : '');
  }
  if (report.restart_required) showBanner(t('bannerRestart'));
}

async function addDomain(mode) {
  const domain = currentTarget();
  if (!domain) {
    toast(t('errEmptyDomain'), 'err');
    return;
  }
  setBusy(true);
  const res = await bridgeRequest(state.port, '/api/domain', {
    method: 'POST',
    token: state.token,
    body: { mode, domain, ...(state.profile ? { profile: state.profile } : {}) },
  });
  setBusy(false);

  if (res.network) { showBanner(t('bannerDisconnected'), { error: true }); return; }
  if (res.status === 401) { showBanner(t('bannerTokenStale'), { error: true, actionText: t('rePair'), onAction: forgetPairing }); return; }
  if (!res.ok || !res.data?.ok) { toast(t('errAddFailed'), 'err'); return; }

  await refreshAll({ preserveBanner: true });
  reportOutcome({ report: res.data });
}

async function removeDomain() {
  const domain = currentTarget();
  if (!domain) {
    toast(t('errEmptyDomain'), 'err');
    return;
  }
  setBusy(true);
  const res = await bridgeRequest(
    state.port,
    `/api/domain${qs({ domain, profile: state.profile })}`,
    { method: 'DELETE', token: state.token },
  );
  setBusy(false);

  if (res.network) { showBanner(t('bannerDisconnected'), { error: true }); return; }
  if (res.status === 401) { showBanner(t('bannerTokenStale'), { error: true, actionText: t('rePair'), onAction: forgetPairing }); return; }
  if (!res.ok || !res.data?.ok) { toast(t('errAddFailed'), 'err'); return; }

  await refreshAll({ preserveBanner: true });
  const removed = Array.isArray(res.data.removed_from) ? res.data.removed_from : [];
  if (removed.length > 0) {
    toast(t('removedMsg', { lists: removed.join(', ') }), 'ok');
    if (res.data.restart_required) showBanner(t('bannerRestart'));
  } else {
    toast(t('removedNone'));
  }
}

async function captureLinks() {
  setBusy(true);
  els.captureBtn.textContent = t('captureBusy');
  let resp;
  try {
    resp = await chrome.runtime.sendMessage({ type: 'captureAll', mode: state.captureMode });
  } catch {
    resp = { ok: false, error: 'internal' };
  }
  setBusy(false);
  els.captureBtn.textContent = t('capture');

  if (!resp || resp.ok !== true) {
    const err = resp?.error ?? 'internal';
    if (err === 'network') showBanner(t('bannerDisconnected'), { error: true });
    else if (err === 'auth' || err === 'unpaired') showBanner(t('bannerTokenStale'), { error: true, actionText: t('rePair'), onAction: forgetPairing });
    else if (err === 'no_tab') toast(t('errNoTab'), 'err');
    else if (err === 'inject') toast(t('errInject'), 'err');
    else if (err === 'empty') toast(t('capturedNone'));
    else toast(t('errGeneric'), 'err');
    return;
  }

  await refreshAll({ preserveBanner: true });
  reportOutcome({ report: resp.report ?? {}, capturedCount: resp.count ?? null });
}

async function forgetPairing() {
  await clearState();
  state.token = null;
  state.profile = '';
  probedPort = state.port;
  showPair();
}

function setCaptureMode(mode) {
  state.captureMode = mode === 'exclude' ? 'exclude' : 'bypass';
  els.segBypass.classList.toggle('active', state.captureMode === 'bypass');
  els.segExclude.classList.toggle('active', state.captureMode === 'exclude');
  saveState({ captureMode: state.captureMode });
}

// ── events / init ────────────────────────────────────────────

function bindEvents() {
  els.pairBtn.addEventListener('click', doPair);
  els.pairCode.addEventListener('keydown', (e) => { if (e.key === 'Enter') doPair(); });
  els.pairCode.addEventListener('input', () => {
    els.pairCode.value = els.pairCode.value.replace(/\D/g, '').slice(0, 6);
  });
  els.pairPortCheck.addEventListener('click', checkPort);
  els.pairPort.addEventListener('keydown', (e) => { if (e.key === 'Enter') checkPort(); });

  els.refreshBtn.addEventListener('click', () => refreshAll());
  els.profileSelect.addEventListener('change', async () => {
    state.profile = els.profileSelect.value;
    await saveState({ profile: state.profile });
    refreshAll();
  });
  els.domainInput.addEventListener('input', updateTarget);
  els.domainInput.addEventListener('keydown', (e) => { if (e.key === 'Enter') addDomain('bypass'); });
  els.addBypass.addEventListener('click', () => addDomain('bypass'));
  els.addExclude.addEventListener('click', () => addDomain('exclude'));
  els.removeDomain.addEventListener('click', removeDomain);
  els.segBypass.addEventListener('click', () => setCaptureMode('bypass'));
  els.segExclude.addEventListener('click', () => setCaptureMode('exclude'));
  els.captureBtn.addEventListener('click', captureLinks);
  els.disconnectBtn.addEventListener('click', forgetPairing);
}

async function init() {
  applyStaticTexts();
  bindEvents();

  const stored = await loadState();
  state.token = stored.token ?? null;
  state.port = Number(stored.port) || DEFAULT_PORT;
  state.profile = stored.profile ?? '';
  probedPort = state.port;
  setCaptureMode(stored.captureMode === 'exclude' ? 'exclude' : 'bypass');

  if (state.token) {
    showMain();
    await detectTab();
    refreshAll();
  } else {
    showPair();
    await detectTab();
  }
}

init();
