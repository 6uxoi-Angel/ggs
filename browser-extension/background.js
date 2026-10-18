// background.js — Integra extension service worker (MV3, ES module).
// Owns the "capture all links of the active tab" flow: it injects a collector
// into the page, normalizes the hosts and bulk-adds them via POST /api/domains.

import { bridgeRequest, loadState, normalizeDomains } from './api.js';

/** Injected into the page: must stay fully self-contained (no closures). */
function collectLinks() {
  const hosts = new Set();
  if (location.hostname) hosts.add(location.hostname);
  for (const a of document.querySelectorAll('a[href]')) {
    try {
      const u = new URL(a.href, location.href);
      if (u.protocol === 'http:' || u.protocol === 'https:') hosts.add(u.hostname);
    } catch {
      // malformed href — skip
    }
  }
  return [...hosts];
}

async function getActiveTab() {
  const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
  return tab ?? null;
}

async function extractHosts(tabId) {
  // allFrames first (captures links inside iframes), fall back to main frame.
  try {
    const results = await chrome.scripting.executeScript({
      target: { tabId, allFrames: true },
      func: collectLinks,
    });
    return results.flatMap((r) => (Array.isArray(r?.result) ? r.result : []));
  } catch {
    const results = await chrome.scripting.executeScript({
      target: { tabId, allFrames: false },
      func: collectLinks,
    });
    return results.flatMap((r) => (Array.isArray(r?.result) ? r.result : []));
  }
}

async function handleCaptureAll({ mode = 'bypass' } = {}) {
  const tab = await getActiveTab();
  if (!tab || typeof tab.id !== 'number' || tab.id < 0) {
    return { ok: false, error: 'no_tab' };
  }

  let raw;
  try {
    raw = await extractHosts(tab.id);
  } catch {
    return { ok: false, error: 'inject' };
  }

  const domains = normalizeDomains(raw);
  if (domains.length === 0) {
    return { ok: false, error: 'empty' };
  }

  const { token, port, profile } = await loadState();
  if (!token) return { ok: false, error: 'unpaired' };

  const body = { mode: mode === 'exclude' ? 'exclude' : 'bypass', domains };
  if (profile) body.profile = profile;

  const res = await bridgeRequest(port, '/api/domains', { method: 'POST', token, body });
  if (res.network) return { ok: false, error: 'network' };
  if (res.status === 401) return { ok: false, error: 'auth' };
  if (!res.ok) return { ok: false, error: 'api', detail: res.data?.error ?? null };

  return { ok: true, count: domains.length, report: res.data ?? {} };
}

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (!message || message.type !== 'captureAll') return false;
  handleCaptureAll(message)
    .then(sendResponse)
    .catch((err) => sendResponse({ ok: false, error: 'internal', detail: String(err?.message ?? err) }));
  return true; // async response
});
