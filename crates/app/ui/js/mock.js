/* Integra mock backend — only active when the real Tauri API is absent.
 * Lets the UI be previewed/debugged in a plain browser (and in devtools).
 */
(function () {
  if (window.__TAURI__) return;

  const now = () => new Date().toISOString();
  let seq = 0;

  const state = {
    config: {
      schema: 3,
      language: 'ru',
      theme: 'dark_accent',
      profiles: [
        {
          id: 'ab12cd34',
          name: 'Основной',
          path: 'C:/zapret-discord-youtube',
          selected_strategy: 'general (ALT).bat',
          autostart: true,
          enabled: true,
          service_expected: false,
          use_shared_lists: false,
        },
        {
          id: 'ef56ab78',
          name: 'Discord',
          path: 'C:/zapret-alt',
          selected_strategy: 'general FAKE TLS.bat',
          autostart: false,
          enabled: true,
          service_expected: false,
          use_shared_lists: true,
        },
      ],
      active_profile: 'ab12cd34',
      bridge: { enabled: true, port: 8765, token: 'mock-token-0000' },
      netmon: { poll_interval_ms: 1000, resolve_hosts: true, auto_include_new_processes: true, max_entries: 5000 },
      diagnostics: {
        custom_sites: [
          { id: 'cs1', name: 'Steam', host: 'store.steampowered.com', port: 443 },
        ],
      },
      window: { width: 1180, height: 760, x: null, y: null, maximized: false },
      start_minimized: false,
      autostart_minimized: true,
      close_to_tray: true,
      autostart_windows: false,
      log_level: 'ALL',
      dev_mode: false,
      taskbar: { position: 'top', style: 'full', show_labels: true },
      theme_presets: [
        {
          id: 'preset1', name: 'Янтарь',
          colors: { accent: '#f59e0b', background: '#14100c' },
          tabbar_image: null, content_image: null,
        },
      ],
      auto_check_zapret: true,
      auto_install_zapret: false,
      auto_update_zapret: false,
    },
    lists: {
      'list-general': ['youtube.com', 'discord.com', 'example.com'],
      'list-exclude': ['rutube.ru'],
      'list-google': ['googlevideo.com'],
      'ipset-all': ['157.240.0.0/16'],
      'ipset-exclude': [],
    },
    strategies: [
      { file: 'general (ALT).bat', name: 'general (ALT)', category: 'advanced' },
      { file: 'general FAKE TLS.bat', name: 'general FAKE TLS', category: 'tls' },
      { file: 'general.bat', name: 'general', category: 'basic' },
    ],
    logs: [],
    netmonRunning: false,
    netmonEntries: [],
    processes: ['r5apex.exe', 'discord.exe', 'chrome.exe', 'steam.exe'],
    benchmark: { running: false, progress: null, result: null },
    pairing: { code: '482913', expires_in: 842 },
  };

  const LOG_SOURCES = ['app', 'zapret', 'netmon', 'bridge', 'diag', 'app::settings'];
  const listeners = {};
  function pushLog(level, source, message) {
    state.logs.push({ ts: now(), level, source, message });
    if (state.logs.length > 5000) state.logs.shift();
    for (const fn of listeners['logs'] || []) {
      try { fn({ payload: { ts: now(), level, source, message } }); } catch (e) { /* noop */ }
    }
  }
  pushLog('INFO', 'app', 'Integra 3.0.0 starting (mock)');
  pushLog('SUCCESS', 'bridge', 'HTTP-мост слушает 127.0.0.1:8765');
  pushLog('DEBUG', 'app::settings', 'config loaded from %APPDATA%/Integra/config.json');

  const tauri = {
    core: {
      async invoke(cmd, args = {}) {
        await new Promise((r) => setTimeout(r, 60 + Math.random() * 120));
        // The real Tauri runtime maps JS camelCase arg names to Rust
        // snake_case params; emulate that (keeping the original keys too).
        const normalized = {};
        for (const [key, value] of Object.entries(args)) {
          normalized[key] = value;
          const snake = key.replace(/([A-Z])/g, (m) => '_' + m.toLowerCase());
          if (snake !== key) normalized[snake] = value;
        }
        return handle(cmd, normalized);
      },
    },
    event: {
      listen(event, handler) {
        (listeners[event] = listeners[event] || []).push(handler);
        return Promise.resolve(() => {});
      },
    },
    dialog: {
      async open(opts = {}) {
        if ((opts.filters || []).some((f) => (f.extensions || []).includes('png'))) {
          return 'C:/mock/wallpaper.jpg';
        }
        return 'C:/mock-folder';
      },
      async save(opts = {}) {
        return 'C:/mock/' + (opts.defaultPath || 'export.txt');
      },
    },
  };

  function health(p) {
    return p.path.includes('zapret') ? { state: 'Ready', detail: true } : { state: 'MissingFolder' };
  }

  function handle(cmd, a) {
    switch (cmd) {
      case 'i18n_bundle': {
        const data = window.__I18N_DATA__ || {};
        const index = state.config.language === 'en' ? 1 : 0;
        const out = {};
        for (const [key, pair] of Object.entries(data)) out[key] = pair[index];
        return out;
      }
      case 'get_config':
        return state.config;
      case 'set_language':
        state.config.language = a.language; return null;
      case 'set_theme':
        state.config.theme = a.theme; return null;
      case 'update_settings':
        Object.assign(state.config, {}); return state.config;
      case 'save_window_state': return null;
      case 'app_info':
        return {
          version: '3.0.0-mock', config_path: 'C:/Users/mock/AppData/Roaming/Integra/config.json',
          dev_log_path: 'C:/Users/mock/AppData/Local/Integra/logs/dev.log',
          shared_lists_dir: 'C:/Users/mock/AppData/Local/Integra/shared',
          is_admin: false, os: 'windows',
        };
      case 'relaunch_as_admin': return null;

      case 'list_profiles':
        return state.config.profiles.map((p) => ({
          ...p,
          ...health(p),
          status: p.id === 'ab12cd34' ? 'running' : 'stopped',
          pid: p.id === 'ab12cd34' ? 4242 : 0,
          service_installed: false,
          service_running: false,
          strategies_count: state.strategies.length,
        }));
      case 'add_profile': {
        const p = {
          id: Math.random().toString(16).slice(2, 10),
          name: a.name || 'Zapret',
          path: a.path,
          selected_strategy: 'general.bat',
          autostart: false, enabled: true, service_expected: false, use_shared_lists: false,
        };
        state.config.profiles.push(p);
        return { ...p, ...health(p), status: 'stopped', pid: 0, service_installed: false, service_running: false, strategies_count: state.strategies.length };
      }
      case 'update_profile': {
        const i = state.config.profiles.findIndex((x) => x.id === a.profile.id);
        if (i >= 0) state.config.profiles[i] = a.profile;
        const p = state.config.profiles[i];
        return { ...p, ...health(p), status: 'stopped', pid: 0, service_installed: false, service_running: false, strategies_count: state.strategies.length };
      }
      case 'remove_profile':
        state.config.profiles = state.config.profiles.filter((p) => p.id !== a.profile_id);
        return null;
      case 'set_active_profile':
        state.config.active_profile = a.profile_id; return null;
      case 'list_strategies':
        return state.strategies;
      case 'start_profile':
        pushLog('SUCCESS', 'zapret', `Стратегия ${a.strategy || 'general.bat'} запущена (winws PID 4242) (mock)`);
        return 4242;
      case 'stop_profile':
        pushLog('INFO', 'zapret', 'Остановлено процессов: 1 (mock)');
        return 1;
      case 'profile_status': {
        const p = state.config.profiles.find((x) => x.id === a.profile_id);
        return {
          ...p, ...health(p), status: p && p.id === 'ab12cd34' ? 'running' : 'stopped',
          pid: 4242, service_installed: false, service_running: false, strategies_count: state.strategies.length,
        };
      }
      case 'install_service':
      case 'remove_service':
        return null;
      case 'open_in_explorer':
      case 'reveal_config':
        return null;

      case 'list_read':
        return {
          id: a.list_id,
          entries: [...(state.lists[a.list_id] || [])],
          count: (state.lists[a.list_id] || []).length,
        };
      case 'list_meta':
        return {
          use_shared: !!state.config.profiles.find((p) => p.id === a.profile_id)?.use_shared_lists,
          counts: Object.fromEntries(Object.entries(state.lists).map(([k, v]) => [k, v.length])),
        };
      case 'list_add': {
        const list = state.lists[a.list_id] || (state.lists[a.list_id] = []);
        const report = { added: [], duplicates: [], invalid: [] };
        for (const raw of a.entries || []) {
          const e = raw.trim().toLowerCase();
          if (!e) continue;
          if (list.includes(e)) report.duplicates.push(e);
          else { list.push(e); report.added.push(e); }
        }
        return report;
      }
      case 'list_remove': {
        const list = state.lists[a.list_id] || [];
        const i = list.indexOf(a.entry.toLowerCase());
        if (i >= 0) list.splice(i, 1);
        return i >= 0;
      }
      case 'list_replace': {
        state.lists[a.list_id] = [...new Set((a.lines || []).map((l) => l.trim().toLowerCase()).filter(Boolean))];
        return { added: [...state.lists[a.list_id]], duplicates: [], invalid: [] };
      }
      case 'list_remove_from_all':
        for (const k of Object.keys(state.lists)) {
          const i = state.lists[k].indexOf(a.entry.toLowerCase());
          if (i >= 0) state.lists[k].splice(i, 1);
        }
        return [];

      case 'netmon_start':
        state.netmonRunning = true;
        state.netmonEntries = [];
        return null;
      case 'netmon_stop':
        state.netmonRunning = false;
        return { entries: state.netmonEntries.length, processes_seen: state.processes.length, duration_secs: 42 };
      case 'netmon_status':
        return {
          running: state.netmonRunning,
          summary: state.netmonRunning ? { entries: state.netmonEntries.length, processes_seen: state.processes.length, duration_secs: 42 } : null,
        };
      case 'netmon_entries':
        return state.netmonEntries.slice(-(a.limit || 500));
      case 'netmon_summary':
        return state.processes.map((name, i) => ({
          pid: 1000 + i * 4,
          process: name,
          connections: 3 + ((seq * 7 + i * 13) % 40),
          hits: 10 + i * 25,
          active: i < 2,
          local_ips: ['192.168.1.10'],
          top_remotes: [name === 'discord.exe' ? 'discord.com' : '93.184.216.' + (i + 1) + ':443'],
        }));
      case 'netmon_processes':
        return state.processes.map((name, i) => ({ pid: 1000 + i * 4, name, exe: null }));
      case 'netmon_export_csv':
        pushLog('SUCCESS', 'netmon', 'CSV → ' + a.path + ' (mock)');
        return null;

      case 'diag_sites':
        return [
          { id: 'default-discord', name: 'Discord', host: 'discord.com', port: 443 },
          { id: 'default-youtube', name: 'YouTube', host: 'youtube.com', port: 443 },
          { id: 'default-github', name: 'GitHub', host: 'github.com', port: 443 },
          ...state.config.diagnostics.custom_sites.map((s) => ({ id: s.id, name: s.name, host: s.host, port: s.port })),
        ];
      case 'diag_add_site': {
        const site = { id: 'cs' + ++seq, name: a.name, host: a.host, port: a.port };
        state.config.diagnostics.custom_sites.push(site);
        return site;
      }
      case 'diag_remove_site':
        state.config.diagnostics.custom_sites = state.config.diagnostics.custom_sites.filter((s) => s.id !== a.site_id);
        return null;
      case 'diag_run': {
        const sites = handle('diag_sites');
        return sites.map((site) => ({
          site,
          outcomes: (a.kinds || []).map((k) => {
            const ok = Math.random() > 0.25;
            return {
              kind: k,
              ok,
              latency: ok ? { min_ms: 20, avg_ms: 35, max_ms: 60, rounds: 2, successes: 2 } : null,
              addrs: k === 'dns' && ok ? ['93.184.216.34'] : [],
              error: ok ? null : 'Таймаут',
            };
          }),
        }));
      }

      case 'benchmark_start':
        state.benchmark = {
          running: true,
          progress: { phase: 'starting', strategy: 'general.bat', index: 1, total: 3 },
          result: null,
        };
        return null;
      case 'benchmark_status':
        return state.benchmark;
      case 'benchmark_cancel':
        return null;

      case 'bridge_status':
        return { running: true, port: state.config.bridge.port, enabled: state.config.bridge.enabled };
      case 'bridge_start':
        return state.config.bridge.port;
      case 'bridge_stop':
        return null;
      case 'bridge_pairing':
        return { ...state.pairing };
      case 'bridge_regenerate_code':
        state.pairing = { code: String(100000 + Math.floor(Math.random() * 900000)), expires_in: 900 };
        return { ...state.pairing };

      case 'logs_snapshot': {
        const level = (a.level || 'ALL').toUpperCase();
        const filtered = level === 'ALL' ? state.logs : state.logs.filter((l) => l.level === level);
        return filtered.slice(-(a.limit || 300));
      }
      case 'logs_clear':
        state.logs = []; return null;
      case 'logs_export':
        pushLog('SUCCESS', 'logs', 'exported → ' + a.path + ' (mock)');
        return null;

      case 'dev_enable':
        state.config.dev_mode = true;
        pushLog('WARN', 'app::dev', 'DEVELOPER MODE ENABLED (mock)');
        return null;
      case 'dev_disable':
        state.config.dev_mode = false; return null;
      case 'open_dev_logs':
        pushLog('INFO', 'app::dev', 'dev console attached (mock)');
        return null;

      case 'theme_presets':
        return state.config.theme_presets;
      case 'theme_save_preset': {
        const preset = {
          id: 'p' + ++seq,
          name: a.name,
          base: a.base || 'dark_accent',
          colors: a.colors || {},
          tabbar_image: a.tabbarImage ?? null,
          content_image: a.contentImage ?? null,
        };
        state.config.theme_presets.push(preset);
        state.config.theme = 'preset:' + preset.id;
        return preset;
      }
      case 'theme_delete_preset':
        state.config.theme_presets = state.config.theme_presets.filter((p) => p.id !== a.preset_id);
        return null;
      case 'read_image_data_url':
        return 'data:image/svg+xml;base64,' + btoa('<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"><defs><linearGradient id="g"><stop offset="0" stop-color="#f59e0b"/><stop offset="1" stop-color="#1d1712"/></linearGradient></defs><rect width="120" height="80" fill="url(#g)"/></svg>');

      case 'zapret_check_update':
        return { tag: 'v1.6.2', version: '1.6.2', name: '1.6.2', published_at: '2025-01-01T00:00:00Z', download_url: 'https://example.com/zapret.zip', size: 12345678 };
      case 'zapret_installed_version':
        return '1.6.1';
      case 'zapret_install_update':
        pushLog('SUCCESS', 'maint', 'Zapret ' + a.version + ' установлен (mock)');
        return null;

      default:
        throw new Error('mock: unknown command ' + cmd);
    }
  }

  // Fake netmon entry stream while "tracking".
  setInterval(() => {
    if (!state.netmonRunning) return;
    const name = state.processes[seq % state.processes.length];
    const entry = {
      proto: ['tcp4', 'udp4'][seq % 2],
      pid: 1000 + (seq % state.processes.length) * 4,
      process: name,
      exe: null,
      local: '192.168.1.10:' + (50000 + (seq % 400)),
      remote: '93.184.216.' + (1 + (seq % 8)) + ':443',
      remote_ip: '93.184.216.' + (1 + (seq % 8)),
      remote_port: 443,
      host: name === 'discord.exe' ? 'gateway.discord.gg' : null,
      service: 'HTTPS',
      state: 'estab',
      first_seen: now(),
      last_seen: now(),
      hits: 1 + (seq % 5),
    };
    state.netmonEntries.push(entry);
    if (state.netmonEntries.length > 800) state.netmonEntries.shift();
    seq++;
  }, 700);

  // Benchmark progress simulation.
  const phases = ['starting', 'warmup', 'checking', 'restoring'];
  let benchIdx = 0;
  setInterval(() => {
    if (!state.benchmark.running) return;
    benchIdx++;
    if (benchIdx > 6) {
      state.benchmark = {
        running: false,
        progress: { phase: 'done', strategy: '', index: 3, total: 3 },
        result: {
          entries: [
            { strategy: 'general (ALT).bat', score: 92, success_ratio: 100, avg_latency_ms: 120, error: null },
            { strategy: 'general FAKE TLS.bat', score: 88, success_ratio: 100, avg_latency_ms: 180, error: null },
            { strategy: 'general.bat', score: 40, success_ratio: 33, avg_latency_ms: null, error: null },
          ],
          best: 'general (ALT).bat',
          cancelled: false,
          restored_strategy: 'general (ALT).bat',
        },
      };
      benchIdx = 0;
      return;
    }
    state.benchmark.progress = {
      phase: phases[Math.min(2, Math.floor(benchIdx / 2))],
      strategy: state.strategies[benchIdx % 3].file,
      index: Math.min(3, Math.floor(benchIdx / 2) + 1),
      total: 3,
    };
  }, 1500);

  window.__TAURI__ = tauri;
  document.getElementById('mock-banner').hidden = false;
  console.info('[integra-mock] mock backend installed');
})();
