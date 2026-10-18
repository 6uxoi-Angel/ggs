// api.js — shared helpers for the Integra browser extension.
// Imported by both the popup (popup.js) and the service worker (background.js).
// Talks to the Integra loopback bridge (see crates/bridge/src/lib.rs, API v2).

export const DEFAULT_PORT = 8765;
export const MAX_BULK_DOMAINS = 2000; // server-side limit, see bridge lib.rs
const REQUEST_TIMEOUT_MS = 8000;

export const STORAGE_KEYS = ['token', 'port', 'profile', 'captureMode', 'apiVersion'];

// ── storage ──────────────────────────────────────────────────

export function loadState() {
  return chrome.storage.local.get(STORAGE_KEYS);
}

export function saveState(patch) {
  return chrome.storage.local.set(patch);
}

export function clearState() {
  return chrome.storage.local.remove(STORAGE_KEYS);
}

// ── bridge HTTP ──────────────────────────────────────────────

export function bridgeBase(port) {
  const p = Number(port) > 0 ? Number(port) : DEFAULT_PORT;
  return `http://127.0.0.1:${p}`;
}

export function qs(params = {}) {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== null && value !== '') search.set(key, String(value));
  }
  const s = search.toString();
  return s ? `?${s}` : '';
}

/**
 * Minimal bridge client. Never throws: network failures are reported as
 * { status: 0, network: true } so callers can show a friendly message.
 */
export async function bridgeRequest(port, path, { method = 'GET', body, token } = {}) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  try {
    const headers = {};
    if (body !== undefined) headers['Content-Type'] = 'application/json';
    if (token) headers['Authorization'] = `Bearer ${token}`;
    const res = await fetch(bridgeBase(port) + path, {
      method,
      headers,
      body: body !== undefined ? JSON.stringify(body) : undefined,
      signal: controller.signal,
    });
    let data = null;
    try {
      data = await res.json();
    } catch {
      data = null; // non-JSON response — treat as protocol error upstream
    }
    return { status: res.status, ok: res.ok, data };
  } catch {
    return { status: 0, ok: false, data: null, network: true };
  } finally {
    clearTimeout(timer);
  }
}

// ── domain normalization (mirror of integra-core rules) ─────

/**
 * Normalize a list of URLs / hostnames into lowercase hostnames suitable for
 * the bridge: hostname only (scheme, port, path, userinfo are dropped), `www`
 * is kept as-is, deduplicated, capped at MAX_BULK_DOMAINS.
 * Local/loopback hosts and obvious non-domains are skipped — the server would
 * reject them anyway (≥ 2 labels required).
 */
export function normalizeDomains(items) {
  const seen = new Set();
  const out = [];
  for (const item of items) {
    const host = rawToHostname(item);
    if (!host || seen.has(host)) continue;
    seen.add(host);
    out.push(host);
    if (out.length >= MAX_BULK_DOMAINS) break;
  }
  return out;
}

export function rawToHostname(raw) {
  if (raw === null || raw === undefined) return '';
  let value = String(raw).trim().toLowerCase();
  if (!value) return '';
  try {
    if (value.includes('://')) {
      value = new URL(value).hostname;
    } else if (/^[a-z][a-z0-9+.-]*:\/\//.test(value)) {
      value = new URL(value).hostname;
    }
  } catch {
    return '';
  }
  // drop :port (except IPv6 literals, which are rejected below anyway)
  value = value.replace(/:\d+$/, '');
  value = value.replace(/\.+$/, ''); // trailing dot (FQDN form)
  if (!value || value.includes(':')) return '';        // IPv6 / garbage
  if (value === 'localhost' || value.endsWith('.localhost')) return '';
  if (/^127\./.test(value) || value === '0.0.0.0') return '';
  if (!value.includes('.')) return '';                  // single-label hosts
  if (!/^[a-z0-9.-]+$/.test(value)) return '';          // punycode/URL hostname is always ASCII
  if (value.split('.').some((label) => !label || label.startsWith('-') || label.endsWith('-'))) return '';
  return value;
}

/** True if the string looks like a 6-digit pairing code. */
export function isPairCode(value) {
  return /^\d{6}$/.test(String(value || '').trim());
}

/** Build a profile option {value, label} from whatever the bridge returns. */
export function profileOption(entry) {
  if (typeof entry === 'string') return { value: entry, label: entry };
  if (entry && typeof entry === 'object') {
    const id = entry.id ?? entry.key ?? '';
    const name = entry.name ?? entry.title ?? entry.label ?? id;
    return { value: String(id), label: String(name) };
  }
  return null;
}
