//! RU/EN translations. Single source of truth for backend messages and the
//! frontend bundle (the UI fetches the whole map via the `i18n_bundle` command).

use parking_lot::RwLock;
use std::collections::HashMap;

pub const LANGUAGES: [&str; 2] = ["ru", "en"];

static CURRENT: RwLock<&str> = RwLock::new("ru");

/// (key, ru, en)
static TABLE: &[(&str, &str, &str)] = &[
    // ── common ────────────────────────────────────────────────
    ("app.brand", "Integra", "Integra"),
    ("common.ok", "ОК", "OK"),
    ("common.cancel", "Отмена", "Cancel"),
    ("common.save", "Сохранить", "Save"),
    ("common.delete", "Удалить", "Delete"),
    ("common.add", "Добавить", "Add"),
    ("common.remove", "Убрать", "Remove"),
    ("common.refresh", "Обновить", "Refresh"),
    ("common.close", "Закрыть", "Close"),
    ("common.search", "Поиск", "Search"),
    ("common.loading", "Загрузка…", "Loading…"),
    ("common.error", "Ошибка", "Error"),
    ("common.success", "Готово", "Done"),
    ("common.yes", "Да", "Yes"),
    ("common.no", "Нет", "No"),
    ("common.enabled", "Включено", "Enabled"),
    ("common.disabled", "Выключено", "Disabled"),
    ("common.all", "Все", "All"),
    ("common.copy", "Копировать", "Copy"),
    ("common.copied", "Скопировано", "Copied"),
    ("common.copy_failed", "Не удалось скопировать", "Copy failed"),
    ("common.open", "Открыть", "Open"),
    ("common.name", "Имя", "Name"),
    ("common.path", "Путь", "Path"),
    ("common.status", "Состояние", "Status"),
    ("common.actions", "Действия", "Actions"),
    ("common.version", "Версия", "Version"),
    ("common.apply", "Применить", "Apply"),
    ("common.port_invalid", "Порт должен быть 1–65535", "Port must be 1–65535"),
    ("common.soon", "Появится в следующей итерации", "Coming in the next iteration"),
    ("dns.not_admin", "Для изменения DNS нужны права администратора", "Administrator rights are required to change DNS"),
    ("dns.no_adapter", "Не найдено активных сетевых адаптеров", "No active network adapters found"),
    ("tgproxy.not_found", "Папка TG WS Proxy не найдена. Укажите путь в настройках раздела.", "TG WS Proxy folder not found. Set the path in the section settings."),
    ("tgproxy.already_running", "Прокси уже запущен", "Proxy is already running"),
    ("tgproxy.setup_running", "Установка зависимостей уже идёт…", "Dependency installation is already running…"),
    ("nav.dns", "Roblox DNS", "Roblox DNS"),
    ("nav.tgproxy", "TG WS Proxy", "TG WS Proxy"),
    ("dns.title", "Roblox DNS Manager", "Roblox DNS Manager"),
    ("dns.subtitle", "Автовыбор лучшего DNS и фикс иконок Roblox (tr.rbxcdn.com)", "Auto-pick the best DNS and fix Roblox images (tr.rbxcdn.com)"),
    ("dns.current_title", "Текущий DNS", "Current DNS"),
    ("dns.bench_title", "Тест DNS-серверов", "DNS server benchmark"),
    ("dns.use_doh", "Включить DoH (Windows 11)", "Enable DoH (Windows 11)"),
    ("dns.run_bench", "Протестировать DNS", "Test DNS servers"),
    ("dns.apply_selected", "Применить выбранный DNS", "Apply selected DNS"),
    ("dns.reset", "Сбросить на автоматический", "Reset to automatic"),
    ("dns.bench_running", "Тестируем DNS-серверы…", "Testing DNS servers…"),
    ("dns.bench_done", "Тест завершён", "Benchmark finished"),
    ("dns.pick_hint", "Сначала протестируйте DNS и выберите сервер", "Run the benchmark and pick a server first"),
    ("dns.applied", "DNS {name} применён ({primary})", "DNS {name} applied ({primary})"),
    ("dns.reset_confirm", "Сбросить DNS на автоматический (DHCP) и убрать DoH?", "Reset DNS to automatic (DHCP) and remove DoH?"),
    ("dns.resetted", "DNS сброшен на DHCP", "DNS reset to DHCP"),
    ("dns.hosts_title", "Фикс иконок Roblox (hosts)", "Roblox image fix (hosts)"),
    ("dns.hosts_desc", "Привяжет IP-адрес CDN Roblox к файлу hosts — работает даже если провайдер перехватывает DNS. IP подбирается по минимальному пингу.", "Pins the Roblox CDN IP to your hosts file — works even when the ISP intercepts DNS. The IP is picked by lowest ping."),
    ("dns.hosts_analyze", "Найти лучший IP", "Find best IP"),
    ("dns.hosts_revert", "Убрать из hosts", "Remove from hosts"),
    ("dns.hosts_analyzing", "Поиск лучшего IP…", "Finding best IP…"),
    ("dns.hosts_empty", "Не удалось получить IP-адреса CDN. Проверьте сеть.", "Could not get CDN IP addresses. Check your network."),
    ("dns.hosts_apply", "Применить в hosts", "Apply to hosts"),
    ("dns.hosts_done", "IP {ip} добавлен в hosts (бэкап создан)", "IP {ip} added to hosts (backup created)"),
    ("dns.hosts_revert_confirm", "Убрать запись Roblox из файла hosts?", "Remove the Roblox entry from the hosts file?"),
    ("dns.hosts_reverted", "Запись Roblox убрана из hosts", "Roblox entry removed from hosts"),
    ("dns.hosts_note", "Не запускайте Zapret одновременно с привязкой hosts: обход DPI может добавить пинг (по комментариям к гайду).", "Don't run Zapret together with hosts pinning: the DPI bypass can add ping (per the guide comments)."),
    ("tgproxy.title", "TG WS Proxy", "TG WS Proxy"),
    ("tgproxy.subtitle", "Локальный MTProto-прокси для Telegram через WebSocket", "Local MTProto proxy for Telegram over WebSocket"),
    ("tgproxy.status_title", "Состояние прокси", "Proxy status"),
    ("tgproxy.start", "Запустить", "Start"),
    ("tgproxy.stop", "Остановить", "Stop"),
    ("tgproxy.setup", "Установить зависимости", "Install dependencies"),
    ("tgproxy.setup_started", "Установка запущена — следите за логом", "Installation started — watch the log"),
    ("tgproxy.started", "TG WS Proxy запущен", "TG WS Proxy started"),
    ("tgproxy.stopped", "TG WS Proxy остановлен", "TG WS Proxy stopped"),
    ("tgproxy.running", "работает", "running"),
    ("tgproxy.venv", "Окружение Python", "Python environment"),
    ("tgproxy.port", "Порт", "Port"),
    ("tgproxy.secret", "Secret", "Secret"),
    ("tgproxy.link", "Ссылка подключения", "Connection link"),
    ("tgproxy.logs_title", "Лог прокси", "Proxy log"),
    ("tgproxy.path_saved", "Путь сохранён", "Path saved"),
    ("help.dns", "Roblox DNS Manager: выбирает лучший DNS под вашу сеть и чинит иконки Roblox.\\n\\n• Тест DNS — замеряет задержку TCP и реальный резолв каждого сервера (включая tr.rbxcdn.com — CDN Roblox) и ставит балл. Выберите сервер и нажмите «Применить». Включите DoH на Windows 11, чтобы DNS-запросы шли по HTTPS.\\n• Сбросить — вернуть автоматический DNS провайдера (DHCP) и убрать DoH.\\n• Фикс иконок — находит IP-адреса CDN Roblox через лучшие резолверы, замеряет пинг до каждого и привязывает лучший в файл hosts. Если Zapret включён, пинг может вырасти — используйте либо то, либо другое.\\n\\nТребуются права администратора для изменения DNS и hosts.", "Roblox DNS Manager: picks the best DNS for your network and fixes Roblox images.\\n\\n• DNS test — measures TCP latency and real resolution per resolver (including tr.rbxcdn.com — Roblox CDN) and scores it. Select a server and hit Apply. Enable DoH on Windows 11 to send DNS over HTTPS.\\n• Reset — restore the ISP automatic DNS (DHCP) and remove DoH.\\n• Image fix — finds Roblox CDN IPs via the best resolvers, pings each, and pins the fastest one to the hosts file. If Zapret is running, ping may rise — use one or the other.\\n\\nAdministrator rights are required to change DNS and hosts."),
    ("help.tgproxy", "TG WS Proxy — локальный MTProto-прокси для Telegram Desktop: трафик идёт через WebSocket (TLS), данные зашифрованы, сторонние серверы не нужны.\\n\\n• Установить зависимости — создаёт .venv в папке проекта и ставит нужные пакеты (httpx[http2], cryptography, certifi).\\n• Запустить — поднимает прокси на 127.0.0.1:порт (по умолчанию 1443) скрытым процессом. Secret генерируется автоматически.\\n• Ссылка подключения — скопируйте её и отправьте в Telegram («Избранное»): Telegram Desktop настроит прокси сам. Либо вручную: Настройки → Продвинутые → Тип подключения → Прокси → MTProto, сервер 127.0.0.1, порт, secret.\\n\\nАнтивирусы могут ложно срабатывать на упакованные сборки — работайте из исходников (этот раздел именно так и делает).", "TG WS Proxy is a local MTProto proxy for Telegram Desktop: traffic goes over WebSocket (TLS), data stays encrypted, no third-party servers needed.\\n\\n• Install dependencies — creates .venv in the project folder and installs the required packages (httpx[http2], cryptography, certifi).\\n• Start — runs the proxy on 127.0.0.1:port (default 1443) as a hidden process. The secret is generated automatically.\\n• Connection link — copy it and send it to Telegram (Saved Messages): Telegram Desktop configures the proxy by itself. Or manually: Settings → Advanced → Connection type → Proxy → MTProto, server 127.0.0.1, port, secret.\\n\\nAntivirus engines may falsely flag packed builds — work from source (this section does exactly that)."),
    // ── navigation ────────────────────────────────────────────
    ("nav.dashboard", "Профили", "Dashboard"),
    ("nav.apps", "Приложения", "Apps"),
    ("nav.lists", "Списки", "Lists"),
    ("nav.diagnostics", "Диагностика", "Diagnostics"),
    ("nav.extension", "Расширение", "Extension"),
    ("nav.logs", "Логи", "Logs"),
    ("nav.settings", "Настройки", "Settings"),
    // ── dashboard / profiles ──────────────────────────────────
    ("dashboard.title", "Профили Zapret", "Zapret profiles"),
    ("dashboard.subtitle", "Несколько установок, каждая со своей стратегией и списками", "Multiple installations, each with its own strategy and lists"),
    ("dashboard.add_profile", "Добавить профиль", "Add profile"),
    ("dashboard.no_profiles", "Профилей пока нет. Добавьте папку с установленным Zapret.", "No profiles yet. Add a folder with an installed Zapret."),
    ("dashboard.start", "Запустить", "Start"),
    ("dashboard.stop", "Остановить", "Stop"),
    ("dashboard.strategy", "Стратегия", "Strategy"),
    ("dashboard.status_stopped", "Остановлен", "Stopped"),
    ("dashboard.status_running", "Работает", "Running"),
    ("dashboard.status_service", "Служба", "Service"),
    ("dashboard.install_service", "Установить службу", "Install service"),
    ("dashboard.remove_service", "Удалить службу", "Remove service"),
    ("dashboard.active", "Активный", "Active"),
    ("dashboard.make_active", "Сделать активным", "Make active"),
    ("dashboard.autostart", "Автозапуск стратегии", "Autostart strategy"),
    ("dashboard.open_folder", "Открыть папку", "Open folder"),
    ("dashboard.edit", "Настроить", "Edit"),
    ("dashboard.remove_profile", "Удалить профиль", "Remove profile"),
    ("dashboard.remove_confirm", "Удалить профиль «{name}»? Файлы Zapret останутся на диске.", "Remove profile “{name}”? Zapret files stay on disk."),
    ("profile.new_title", "Новый профиль", "New profile"),
    ("profile.name_placeholder", "Например: Основной", "e.g. Main"),
    ("profile.path_placeholder", "Путь к папке Zapret (где general*.bat и winws.exe)", "Path to the Zapret folder (with general*.bat and winws.exe)"),
    ("profile.choose_folder", "Выбрать папку…", "Choose folder…"),
    ("profile.health_ready", "Готов к работе", "Ready"),
    ("profile.health_missing_path", "Путь не указан", "Path is empty"),
    ("profile.health_missing_folder", "Папка не найдена", "Folder not found"),
    ("profile.health_no_strategies", "Нет general*.bat в папке", "No general*.bat in the folder"),
    ("profile.health_no_winws", "winws.exe не найден (проверьте карантин антивируса)", "winws.exe not found (check antivirus quarantine)"),
    ("profile.health_not_zapret", "Это не похоже на папку Zapret", "This does not look like a Zapret folder"),
    ("profile.duplicate_path", "Эта папка уже добавлена как профиль", "This folder is already added as a profile"),
    // ── zapret manager ────────────────────────────────────────
    ("zapret.windows_only", "Управление Zapret доступно только в Windows", "Zapret control is Windows-only"),
    ("zapret.path_missing", "Папка Zapret не найдена", "Zapret folder not found"),
    ("zapret.no_bat", "В папке нет стратегий general*.bat", "No general*.bat strategies in the folder"),
    ("zapret.no_winws", "winws.exe не найден", "winws.exe not found"),
    ("zapret.admin_start", "Для запуска нужны права администратора", "Administrator rights are required to start"),
    ("zapret.admin_install", "Для установки службы нужны права администратора", "Administrator rights are required to install the service"),
    ("zapret.admin_remove", "Для удаления службы нужны права администратора", "Administrator rights are required to remove the service"),
    ("zapret.service_conflict", "Служба Zapret уже установлена — сначала удалите её", "Zapret service is installed — remove it first"),
    ("zapret.strategy_missing", "Стратегия {strategy} не найдена", "Strategy {strategy} not found"),
    ("zapret.exit_code", "Запуск завершился с кодом {code}. {suffix}", "Launcher exited with code {code}. {suffix}"),
    ("zapret.timeout", "winws.exe не стартовал за отведённое время.{suffix}", "winws.exe did not start in time.{suffix}"),
    ("zapret.output", "Вывод: {details}", "Output: {details}"),
    ("zapret.launch_hint", "Проверьте путь к Zapret и права администратора.", "Check the Zapret path and administrator rights."),
    ("zapret.start_failed", "Не удалось запустить: {error}", "Start failed: {error}"),
    ("zapret.started", "Стратегия {strategy} запущена (winws PID {pid})", "Strategy {strategy} started (winws PID {pid})"),
    ("zapret.stopped_count", "Остановлено процессов: {count}", "Stopped processes: {count}"),
    ("zapret.already_stopped", "Уже остановлено", "Already stopped"),
    ("zapret.stop_partial", "Остановлено частично: {errors}", "Partially stopped: {errors}"),
    ("zapret.install_failed", "Установка службы не удалась: {error}", "Service installation failed: {error}"),
    ("zapret.install_error", "Служба не появилась после установки. {details}", "Service did not appear after install. {details}"),
    ("zapret.service_installed", "Служба Zapret установлена{running}", "Zapret service installed{running}"),
    ("zapret.and_started", " и запущена", " and started"),
    ("zapret.remove_failed", "Не удалось удалить службу: {error}", "Service removal failed: {error}"),
    ("zapret.remove_retry", "Служба всё ещё присутствует — попробуйте ещё раз", "Service still present — try again"),
    ("zapret.service_removed", "Служба Zapret удалена", "Zapret service removed"),
    ("zapret.service_windows_only", "Службы доступны только в Windows", "Services are Windows-only"),
    ("zapret.unknown_error", "Неизвестная ошибка", "Unknown error"),
    ("zapret.path_changed", "Путь Zapret изменён: {path}", "Zapret path changed: {path}"),
    ("zapret.not_admin", "Приложению нужны права администратора", "The app requires administrator rights"),
    ("windows.batch_no_winws", "В стратегии не найдена строка запуска winws.exe", "No winws.exe launch line found in the strategy"),
    // ── netmon ────────────────────────────────────────────────
    ("netmon.title", "Монитор приложений", "App monitor"),
    ("netmon.subtitle", "Куда отправляют запросы игры и программы — чтобы добавить их в обход", "Where games and apps send requests — so you can add them to the bypass"),
    ("netmon.start", "Начать отслеживание", "Start tracking"),
    ("netmon.stop", "Остановить", "Stop"),
    ("netmon.tracking", "Отслеживание активно", "Tracking active"),
    ("netmon.idle", "Остановлено", "Idle"),
    ("netmon.filter_all", "Все процессы", "All processes"),
    ("netmon.filter_name", "По имени процесса", "By process name"),
    ("netmon.filter_hint", "Например: apex, discord, telegram — новые процессы с таким именем подхватятся автоматически", "e.g. apex, discord, telegram — new matching processes are picked up automatically"),
    ("netmon.process", "Процесс", "Process"),
    ("netmon.host", "Хост", "Host"),
    ("netmon.remote", "Адрес", "Remote"),
    ("netmon.proto", "Протокол", "Protocol"),
    ("netmon.first_seen", "Первое соединение", "First seen"),
    ("netmon.hits", "Соединений", "Connections"),
    ("netmon.add_bypass", "В обход", "To bypass"),
    ("netmon.add_exclude", "В исключения", "To exclude"),
    ("netmon.export_csv", "Экспорт CSV", "Export CSV"),
    ("netmon.no_entries", "Соединений пока нет. Запустите игру или приложение.", "No connections yet. Launch the game or app."),
    ("netmon.session_started", "Отслеживание запущено ({filter})", "Tracking started ({filter})"),
    ("netmon.session_stopped", "Отслеживание остановлено, записей: {count}", "Tracking stopped, entries: {count}"),
    ("netmon.already_running", "Отслеживание уже идёт", "Tracking is already running"),
    ("netmon.not_running", "Отслеживание не запущено", "Tracking is not running"),
    ("netmon.added_to_list", "{domain} → {list}", "{domain} → {list}"),
    ("netmon.resolve_hosts", "Определять домены (reverse DNS)", "Resolve hostnames (reverse DNS)"),
    ("netmon.auto_new", "Подхватывать новые процессы по имени", "Auto-include new processes by name"),
    ("netmon.poll_interval", "Интервал опроса, мс", "Poll interval, ms"),
    // ── lists ─────────────────────────────────────────────────
    ("lists.title", "Списки доменов и IP", "Domain & IP lists"),
    ("lists.subtitle", "Изменения применяются после перезапуска стратегии или службы", "Changes apply after restarting the strategy or service"),
    ("lists.name_list-general", "Обход (general)", "Bypass (general)"),
    ("lists.name_list-exclude", "Исключения", "Exclude"),
    ("lists.name_list-google", "Google", "Google"),
    ("lists.name_ipset-all", "IP/CIDR все", "IP/CIDR all"),
    ("lists.name_ipset-exclude", "IP/CIDR исключения", "IP/CIDR exclude"),
    ("lists.entries", "Записей: {count}", "Entries: {count}"),
    ("lists.add_entries", "Добавить записи", "Add entries"),
    ("lists.bulk_placeholder", "По одному в строке: домены, URL или IP/CIDR", "One per line: domains, URLs or IP/CIDR"),
    ("lists.report", "Добавлено: {added}, повторов: {dups}, ошибок: {invalid}", "Added: {added}, duplicates: {dups}, invalid: {invalid}"),
    ("lists.remove_selected", "Удалить выбранное", "Remove selected"),
    ("lists.empty", "Список пуст", "List is empty"),
    ("lists.filter_placeholder", "Фильтр…", "Filter…"),
    ("lists.no_active_profile", "Профиль не выбран. Добавьте или выберите профиль в разделе «Профили».", "No active profile. Add or select a profile in the Profiles section."),
    ("lists.restart_hint", "Перезапустите стратегию, чтобы изменения вступили в силу", "Restart the strategy for changes to take effect"),
    // ── diagnostics ───────────────────────────────────────────
    ("diag.title", "Диагностика сети", "Network diagnostics"),
    ("diag.subtitle", "DNS, TCP (а позже TLS/HTTP) проверки и бенчмарк стратегий", "DNS, TCP (later TLS/HTTP) checks and strategy benchmark"),
    ("diag.run", "Проверить", "Run checks"),
    ("diag.running", "Проверка…", "Checking…"),
    ("diag.dns", "DNS", "DNS"),
    ("diag.tcp", "TCP", "TCP"),
    ("diag.tls", "TLS", "TLS"),
    ("diag.http", "HTTP", "HTTP"),
    ("diag.site", "Сайт", "Site"),
    ("diag.ok", "Доступен", "Reachable"),
    ("diag.fail", "Недоступен", "Unreachable"),
    ("diag.latency", "Задержка", "Latency"),
    ("diag.min", "мин", "min"),
    ("diag.avg", "сред", "avg"),
    ("diag.max", "макс", "max"),
    ("diag.rounds", "Повторов", "Rounds"),
    ("diag.add_site", "Добавить свой сайт", "Add custom site"),
    ("diag.custom_sites", "Свои сайты", "Custom sites"),
    ("diag.default_sites", "Встроенные сайты", "Built-in sites"),
    ("diag.timeout", "Таймаут, c", "Timeout, s"),
    ("diag.benchmark", "Бенчмарк стратегий", "Strategy benchmark"),
    ("diag.no_sites", "Нет сайтов для проверки", "No sites to check"),
    ("diag.cancelled", "Отменено", "Cancelled"),
    ("network.dns_failed", "DNS не разрешается", "DNS resolution failed"),
    ("network.timeout", "Таймаут", "Timeout"),
    ("network.unreachable", "Нет соединения", "Unreachable"),
    // ── extension / bridge ────────────────────────────────────
    ("extension.title", "Браузерное расширение", "Browser extension"),
    ("extension.subtitle", "Добавляйте сайты в обход прямо из браузера", "Add sites to the bypass right from the browser"),
    ("extension.pairing_code", "Код привязки", "Pairing code"),
    ("extension.regenerate", "Новый код", "New code"),
    ("extension.code_regenerated", "Новый код сгенерирован", "New code generated"),
    ("extension.expires_in", "Истекает через {seconds} с", "Expires in {seconds}s"),
    ("extension.bridge_running", "Мост запущен на 127.0.0.1:{port}", "Bridge running on 127.0.0.1:{port}"),
    ("extension.bridge_stopped", "Мост остановлен", "Bridge stopped"),
    ("extension.enable_bridge", "Включить мост", "Enable bridge"),
    ("extension.instructions", "1. Соберите/установите расширение Integra (папка browser-extension).\n2. Откройте страницу расширений браузера и включите режим разработчика.\n3. «Загрузить распакованное» → выберите папку browser-extension.\n4. Откройте расширение и введите код привязки выше.", "1. Build/install the Integra extension (browser-extension folder).\n2. Open the browser extensions page and enable Developer mode.\n3. Click “Load unpacked” and select the browser-extension folder.\n4. Open the extension and enter the pairing code above."),
    ("bridge.already_running", "Мост уже запущен (порт {port})", "Bridge already running (port {port})"),
    ("bridge.start_failed", "Не удалось запустить мост: {error}", "Bridge start failed: {error}"),
    ("bridge.started", "Мост запущен на порту {port}", "Bridge started on port {port}"),
    ("bridge.log_started", "HTTP-мост слушает 127.0.0.1:{port}", "HTTP bridge listening on 127.0.0.1:{port}"),
    ("bridge.log_stopped", "HTTP-мост остановлен", "HTTP bridge stopped"),
    ("bridge.invalid_origin", "Запрос с недопустимым Origin отклонён", "Request with invalid Origin rejected"),
    ("bridge.bad_pair_code", "Неверный или просроченный код привязки", "Invalid or expired pairing code"),
    ("bridge.bad_mode", "Режим должен быть bypass или exclude", "Mode must be bypass or exclude"),
    ("bridge.auth_required", "Требуется токен (Bearer)", "Token required (Bearer)"),
    ("bridge.route_missing", "Неизвестный маршрут", "Unknown route"),
    ("bridge.bad_length", "Некорректный размер тела запроса", "Invalid body length"),
    ("bridge.bad_json", "Тело запроса не JSON", "Body is not valid JSON"),
    ("bridge.json_object", "Ожидался JSON-объект", "Expected a JSON object"),
    ("bridge.connected", "Расширение подключилось к мосту", "Extension connected to the bridge"),
    ("bridge.domain_log", "Домен {domain} добавлен в {mode} (расширение)", "Domain {domain} added to {mode} (extension)"),
    ("bridge.domain_removed_log", "Домен {domain} удалён из списков (расширение)", "Domain {domain} removed from lists (extension)"),
    ("bridge.bulk_log", "Массовое добавление: {added} домен(ов) в {mode} (расширение)", "Bulk add: {added} domain(s) to {mode} (extension)"),
    // ── logs ──────────────────────────────────────────────────
    ("logs.title", "Журнал событий", "Event log"),
    ("logs.clear", "Очистить", "Clear"),
    ("logs.filter_level", "Уровень", "Level"),
    ("logs.empty", "Пока пусто", "Nothing yet"),
    ("logs.export", "Экспорт", "Export"),
    ("logs.autoscroll", "Автопрокрутка", "Auto-scroll"),
    // ── settings ──────────────────────────────────────────────
    ("settings.title", "Настройки", "Settings"),
    ("settings.language", "Язык интерфейса", "Interface language"),
    ("settings.check_components", "Проверка компонентов", "Check components"),
    ("settings.components", "Компоненты", "Components"),
    ("setup.title", "Проверка компонентов", "Component check"),
    ("setup.subtitle", "Проверяем, готово ли окружение для работы Integra. Недостающие компоненты можно установить прямо отсюда.", "Checking that your environment is ready for Integra. Missing components can be installed right here."),
    ("setup.continue", "Продолжить", "Continue"),
    ("setup.status_ok", "Готово", "Ready"),
    ("setup.status_missing", "Требуется внимание", "Action needed"),
    ("setup.all_ready", "Все компоненты на месте", "All components are ready"),
    ("setup.check_webview2", "WebView2 Runtime", "WebView2 Runtime"),
    ("setup.check_vcredist", "Visual C++ Redistributable", "Visual C++ Redistributable"),
    ("setup.check_admin", "Права администратора", "Administrator rights"),
    ("setup.check_zapret", "Zapret (профиль)", "Zapret (profile)"),
    ("setup.action_install", "Установить", "Install"),
    ("setup.action_relaunch", "Перезапустить с правами", "Relaunch as admin"),
    ("setup.zapret_hint", "Добавьте профиль Zapret на вкладке «Профили» или скачайте его в разделе обслуживания.", "Add a Zapret profile on the Profiles tab or download it in the maintenance section."),
    ("settings.theme", "Тема", "Theme"),
    ("settings.theme_light", "Светлая", "Light"),
    ("settings.theme_dark", "Тёмная", "Dark"),
    ("settings.theme_dark_accent", "Тёмная акцентная", "Dark accent"),
    ("settings.start_minimized", "Запускать свёрнутой", "Start minimized"),
    ("settings.close_to_tray", "Сворачивать в трей при закрытии", "Minimize to tray on close"),
    ("settings.autostart_windows", "Автозапуск с Windows", "Autostart with Windows"),
    ("settings.log_level", "Уровень логов", "Log level"),
    ("settings.bridge_port", "Порт моста", "Bridge port"),
    ("settings.config_path", "Файл конфигурации", "Configuration file"),
    ("settings.open_config", "Показать файл", "Reveal file"),
    ("settings.maintenance", "Обслуживание Zapret", "Zapret maintenance"),
    ("settings.auto_check", "Проверять наличие Zapret при запуске", "Check for Zapret on startup"),
    ("settings.auto_install", "Устанавливать Zapret при отсутствии", "Install Zapret when missing"),
    ("settings.auto_update", "Обновлять Zapret автоматически", "Update Zapret automatically"),
    ("settings.about", "О программе", "About"),
    ("settings.about_text", "Integra 3 — GUI для Zapret на Rust + Tauri. Не является частью проекта Zapret.", "Integra 3 — a Rust + Tauri GUI for Zapret. Not part of the Zapret project."),
    ("settings.developer", "Разработчик", "Developer"),
    ("settings.tester", "Тестировщик", "Tester"),
    // ── app / core errors ─────────────────────────────────────
    ("app.admin_required_title", "Нужны права администратора", "Administrator rights required"),
    ("app.admin_required_body", "Не удалось повысить привилегии: {error}", "Elevation failed: {error}"),
    ("app.already_running", "Integra уже запущена", "Integra is already running"),
    ("app.single_instance_focus", "Окно Integra активировано", "Integra window activated"),
    ("core.elevation_windows_only", "Повышение привилегий доступно только в Windows", "Elevation is Windows-only"),
    ("core.elevation_failed_code", "ShellExecute вернул код {code}", "ShellExecute returned code {code}"),
    ("core.enter_domain", "Введите домен или адрес", "Enter a domain or address"),
    ("core.cannot_parse_domain", "Не удалось разобрать адрес", "Cannot parse the address"),
    ("core.ip_not_allowed", "IP-адреса здесь не поддерживаются", "IP addresses are not allowed here"),
    ("core.invalid_domain", "Некорректное доменное имя", "Invalid domain name"),
    ("core.invalid_ip_or_cidr", "Некорректный IP или CIDR", "Invalid IP or CIDR"),
    // ── developer mode ────────────────────────────────────────
    ("dev.title", "Режим разработчика", "Developer mode"),
    ("dev.enabled", "Режим разработчика включён", "Developer mode enabled"),
    ("dev.already", "Режим разработчика уже включён", "Developer mode already enabled"),
    ("dev.hint", "Ещё {count} кликов до режима разработчика", "{count} more clicks to developer mode"),
    ("dev.open_logs", "Открыть dev логи", "Open dev logs"),
    ("dev.opened", "Окно dev-логов открыто", "Dev logs window opened"),
    ("dev.open_failed", "Не удалось открыть окно логов: {error}", "Failed to open the logs window: {error}"),
    ("dev.logs_desc", "Каждое событие приложения пишется в dev.log: уровень, источник (модуль кода) и время. Отдельное окно консоли читает файл в реальном времени — удобно для анализа ошибок.", "Every app event is written to dev.log: level, source (code module) and time. A separate console window tails the file in real time — handy for error analysis."),
    ("dev.webview_console", "Также открыта консоль WebView (DevTools)", "The WebView console (DevTools) was opened too"),
    ("dev.section", "Разработчику", "Developer"),
    // ── taskbar ───────────────────────────────────────────────
    ("taskbar.appearance", "Панель вкладок", "Tab bar"),
    ("taskbar.position", "Расположение", "Position"),
    ("taskbar.pos_top", "Сверху", "Top"),
    ("taskbar.pos_bottom", "Снизу", "Bottom"),
    ("taskbar.pos_left", "Слева", "Left"),
    ("taskbar.pos_right", "Справа", "Right"),
    ("taskbar.style", "Стиль", "Style"),
    ("taskbar.style_full", "Обычный", "Full"),
    ("taskbar.style_compact", "Компактный", "Compact"),
    ("taskbar.style_icons", "Только иконки", "Icons only"),
    ("taskbar.labels", "Подписи вкладок", "Tab labels"),
    ("taskbar.saved", "Вид панели сохранён", "Tab bar layout saved"),
    // ── theme customization ───────────────────────────────────
    ("theme.customize", "Кастомизация темы", "Theme customization"),
    ("theme.hint", "Меняйте цвета отдельных элементов, ставьте фоновые изображения и сохраняйте наборы как пресеты.", "Tweak per-element colors, set background images and save the result as presets."),
    ("theme.base", "Базовая тема", "Base theme"),
    ("theme.presets", "Пресеты", "Presets"),
    ("theme.preset_name", "Название пресета", "Preset name"),
    ("theme.save_preset", "Сохранить как пресет", "Save as preset"),
    ("theme.delete_preset", "Удалить", "Delete"),
    ("theme.apply", "Применить", "Apply"),
    ("theme.preset_saved", "Пресет «{name}» сохранён", "Preset “{name}” saved"),
    ("theme.preset_deleted", "Пресет удалён", "Preset deleted"),
    ("theme.preset_applied", "Тема применена", "Theme applied"),
    ("theme.colors", "Цвета элементов", "Element colors"),
    ("theme.color_accent", "Акцент", "Accent"),
    ("theme.color_background", "Фон приложения", "App background"),
    ("theme.color_surface", "Фон панелей", "Panel background"),
    ("theme.color_tabbar", "Фон панели вкладок", "Tab bar background"),
    ("theme.color_tab", "Фон вкладки", "Tab background"),
    ("theme.color_text", "Текст", "Text"),
    ("theme.color_muted", "Приглушённый текст", "Muted text"),
    ("theme.color_border", "Границы", "Borders"),
    ("theme.color_success", "Успех", "Success"),
    ("theme.color_warning", "Предупреждение", "Warning"),
    ("theme.color_danger", "Ошибка", "Danger"),
    ("theme.images", "Фоновые изображения", "Background images"),
    ("theme.tabbar_image", "Картинка за панелью вкладок", "Tab bar background image"),
    ("theme.content_image", "Картинка за содержимым вкладки", "Tab content background image"),
    ("theme.choose_image", "Выбрать изображение…", "Choose image…"),
    ("theme.clear_image", "Убрать", "Clear"),
    ("theme.image_applied", "Изображение {w}×{h}px применено", "Image {w}×{h}px applied"),
    ("theme.image_cropped", "Область изображения применена", "Image region applied"),
    ("theme.image_size_hint", "Панель вкладок: широкие изображения (примерно 16:9, ширина от 1600 px). Фон контента: примерно 1920×1080 px. Файл до 8 МБ. Большие картинки уменьшаются автоматически, при выборе можно выделить нужную область.", "Tab bar: wide images (about 16:9, width from 1600 px). Content background: about 1920×1080 px. File up to 8 MB. Large images are scaled down automatically; you can also select the region to use."),
    ("theme.image_crop_title", "Выделите область фона", "Select background region"),
    ("theme.image_crop_hint", "Оригинал {w}×{h}px. Перетаскивайте рамку, растягивайте за угол. Использована будет только выделенная область.", "Original {w}×{h}px. Drag the frame, resize by the corner. Only the selected region will be used."),
    ("theme.image_crop_apply", "Применить область", "Apply region"),
    ("theme.image_crop_skip", "Пропустить", "Skip"),
    ("theme.image_save_failed", "Не удалось сохранить изображение", "Failed to save image"),
    ("theme.image_read_failed", "Не удалось прочитать изображение", "Failed to read image"),
    ("theme.image_pick_failed", "Не удалось открыть диалог выбора файла", "Failed to open file dialog"),
    ("theme.image_dialog_missing", "Диалог выбора файла недоступен", "File dialog unavailable"),
    ("theme.reset", "Сбросить кастомизацию", "Reset customization"),
    ("theme.reset_done", "Кастомизация сброшена", "Customization reset"),
    ("theme.image_too_large", "Файл больше 8 МБ", "The file is larger than 8 MB"),
    ("theme.name_required", "Введите название пресета", "Enter a preset name"),
    // ── shared lists ──────────────────────────────────────────
    ("lists.shared", "Общие списки", "Shared lists"),
    ("lists.individual", "Индивидуальные", "Individual"),
    ("lists.use_shared", "Общие списки (для всех профилей)", "Shared lists (across profiles)"),
    ("lists.shared_hint", "Общие списки лежат отдельно от папки Zapret. Любой профиль с включённой опцией использует их вместо своих.", "Shared lists live outside the Zapret folder. Any profile with the option enabled uses them instead of its own."),
    ("lists.source_shared", "Источник: общие списки", "Source: shared lists"),
    ("lists.source_individual", "Источник: папка профиля", "Source: profile folder"),
    ("profile.use_shared", "Общие списки", "Shared lists"),
    // ── help ──────────────────────────────────────────────────
    ("help.title", "Как это работает", "How it works"),
    ("help.close", "Понятно", "Got it"),
    ("help.dashboard", "Профили — независимые установки Zapret, у каждой своя папка, стратегия и списки.\n\n• Добавить профиль — укажите имя и папку, где лежат general*.bat и winws.exe (обычно это zapret-discord-youtube).\n• Запустить/Остановить — старт стратегии через скрытый BAT; для winws (WinDivert) нужны права администратора, Windows покажет запрос UAC.\n• Стратегия — выпадающий список файлов general*.bat. Переключение перезапускает winws с новым набором параметров.\n• Установить службу — зарегистрировать стратегию как службу Windows: работает в фоне без окна. Пока служба установлена, обычный запуск заблокирован.\n• Автозапуск — стратегия стартует сама при открытии Integra.\n• Общие списки — профиль будет использовать общие списки доменов вместо своих.\n• Сделать активным — этот профиль считается основным: его статус показывается в трее и его данные видит расширение по умолчанию.\n• Индикатор здоровья показывает, похожа ли папка на установку Zapret.", "Profiles are independent Zapret installations, each with its own folder, strategy and lists.\n\n• Add profile — point to a folder containing general*.bat and winws.exe (usually zapret-discord-youtube).\n• Start/Stop — runs the strategy through a hidden BAT; winws (WinDivert) needs administrator rights, Windows will show a UAC prompt.\n• Strategy — the dropdown lists general*.bat files. Switching restarts winws with the new parameters.\n• Install service — registers the strategy as a Windows service so it runs in the background. While the service is installed, normal runs are blocked.\n• Autostart — the strategy starts automatically when Integra opens.\n• Shared lists — the profile uses the shared domain lists instead of its own.\n• Make active — this profile becomes the primary one: its status is shown in the tray and the extension reads it by default.\n• The health badge tells whether the folder looks like a Zapret install."),
    ("help.apps", "Монитор приложений отвечает на вопрос «куда ходит эта программа?».\n\n• Начать отслеживание — Integra начинает снимать таблицы TCP/UDP-соединений системы (тот же источник, что у TCPView) и записывать уникальные подключения.\n• Фильтр — «Все процессы» или по имени: подстрока в имени процесса (например apex, discord). Новые процессы с таким именем подхватываются автоматически: можно нажать запись, а потом запустить игру.\n• Карточки сверху — активные приложения, которые сейчас забирают трафик: число соединений, ваши IP, основные адреса. Пульсирующая точка = соединения live.\n• Таблица — детальная история: протокол, адреса, домен (reverse DNS), метка порта (HTTPS, Steam, Game…), счётчик повторов.\n• «В обход»/«В исключения» — добавить хост из строки сразу в соответствующий список.\n• Экспорт CSV — сохранить сессию в файл.\n\nДанные не уходят из системы: reverse-DNS и таблицы соединений обрабатываются локально.", "The app monitor answers “where does this program connect to?”.\n\n• Start tracking — Integra polls the system TCP/UDP tables (same source as TCPView) and records unique connections.\n• Filter — “All processes” or by name: a substring of the process name (apex, discord). New matching processes are picked up automatically: press record first, then launch the game.\n• Top cards — apps currently consuming traffic: connection count, your IPs, top remotes. A pulsing dot means live connections.\n• Table — detailed history: protocol, addresses, hostname (reverse DNS), port label (HTTPS, Steam, Game…), hit counter.\n• “To bypass”/“To exclude” — add the host from a row straight to the matching list.\n• Export CSV — save the session to a file.\n\nNothing leaves your machine: reverse DNS and connection tables are processed locally."),
    ("help.lists", "Списки — то, что winws не трогает и что трогает обязательно.\n\n• Обход (list-general) — домены/IP, к которым применяется обход DPI.\n• Исключения (list-exclude) — то, что обходить не нужно (например, российские CDN).\n• Остальные списки — специфичные наборы отдельных стратегий.\n\n• Переключатель «Общие/Индивидуальные» — выберите источник: общие списки для всех профилей или папка именно этого профиля.\n• Добавить записи — по одной в строке: домен, URL (превратится в домен), IP или CIDR. Валидация и дедупликация автоматические.\n• Заменить всё — вставить весь список целиком; повторы уберутся.\n• Удалить выбранное — очистка выбранных строк.\n\nИзменения вступают в силу после перезапуска стратегии или службы — подсказка об этом всегда под списком.", "Lists define what winws must bypass and what it must leave alone.\n\n• Bypass (list-general) — domains/IPs the DPI bypass applies to.\n• Exclude (list-exclude) — things that must not be bypassed (e.g. local CDNs).\n• Other lists — specific sets used by some strategies.\n\n• The “Shared/Individual” switch picks the source: shared lists used by every profile, or this profile's own folder.\n• Add entries — one per line: a domain, a URL (normalized to domain), an IP or CIDR. Validation and dedup are automatic.\n• Replace all — paste the whole list; duplicates are removed.\n• Remove selected — clear the selected lines.\n\nChanges take effect after restarting the strategy or service — the hint below the list reminds you."),
    ("help.diagnostics", "Диагностика показывает, где именно ломается доступ.\n\n• Проверить — для каждого сайта выполняется цепочка: DNS → TCP → TLS → HTTP. Зелёный чип — этап пройден, красный — упал, серый — не выбран.\n• DNS — резолвится ли домен. TCP — устанавливается ли соединение с портом 443. TLS — настоящий TLS-handshake (rustls): если он ломается, DPI режет именно шифрованное рукопожатие. HTTP — HEAD-запрос по HTTPS.\n• Задержки — min/avg/max по rounds попыткам каждого этапа.\n• Свои сайты — добавьте то, что важно именно вам (игра, мессенджер).\n\nБенчмарк стратегий:\n• Запустить — Integra сама переберёт все general*.bat профиля: стоп → старт → прогрев → проверки сайтов → балл. Лучшей стратегии присваивается ★.\n• После прогона исходная стратегия (и её работа) восстанавливается. Отмена доступна в любой момент.\n• Пока установлена служба, бенчмарк недоступен — сначала удалите её.", "Diagnostics shows exactly where access breaks.\n\n• Run checks — for every site a chain runs: DNS → TCP → TLS → HTTP. Green chip — passed, red — failed, grey — not selected.\n• DNS — does the domain resolve. TCP — can we connect to port 443. TLS — a real TLS handshake (rustls): if it fails, DPI is cutting the encrypted handshake. HTTP — a HEAD request over HTTPS.\n• Latency — min/avg/max across rounds for each stage.\n• Custom sites — add what matters to you (a game, a messenger).\n\nStrategy benchmark:\n• Run — Integra iterates every general*.bat of the profile: stop → start → warmup → site checks → score. The best strategy gets a ★.\n• After the run the original strategy (and its state) is restored. You can cancel at any moment.\n• While a service is installed the benchmark is unavailable — remove the service first."),
    ("help.extension", "Браузерное расширение добавляет сайты в списки прямо из Chrome/Edge.\n\n1. Включите мост кнопкой ниже.\n2. Скопируйте код привязки (он живёт 15 минут, обновляется по кнопке).\n3. В браузере: страница расширений → режим разработчика → «Загрузить распакованное» → папка browser-extension рядом с приложением.\n4. Откройте попап расширения и введите код.\n\nПосле привязки расширение умеет: показывать статус Zapret, добавлять текущий сайт в обход или в исключения, убирать его из списков и захватывать сразу все ссылки открытой вкладки (массовое добавление).\n\nБезопасность: мост слушает только 127.0.0.1, все запросы требуют токен, домены проходят ту же валидацию, что и в UI.", "The browser extension adds sites to the lists right from Chrome/Edge.\n\n1. Enable the bridge with the button below.\n2. Copy the pairing code (it lives 15 minutes; regenerate any time).\n3. In the browser: extensions page → Developer mode → “Load unpacked” → the browser-extension folder next to the app.\n4. Open the extension popup and enter the code.\n\nAfter pairing the extension can: show the Zapret status, add the current site to bypass or exclude, remove it from the lists, and capture every link of the open tab (bulk add).\n\nSecurity: the bridge listens on 127.0.0.1 only, every request needs the token, domains go through the same validation as in the UI."),
    ("help.logs", "Журнал событий приложения.\n\n• Уровень — фильтр: ALL, DEBUG, INFO, SUCCESS, WARN, ERROR.\n• Стриминг — новые записи появляются сразу, включённый автоскролл держит конец списка.\n• Экспорт — сохранить журнал в файл.\n• Очистить — очистить текущий буфер.\n\nНужны технические детали вплоть до модулей кода? Включите режим разработчика (10 быстрых кликов по вкладке «Настройки») и откройте dev логи — отдельное окно консоли с полным потоком событий.", "The application event log.\n\n• Level — the filter: ALL, DEBUG, INFO, SUCCESS, WARN, ERROR.\n• Streaming — new entries appear instantly; auto-scroll keeps the end of the list in view.\n• Export — save the log to a file.\n• Clear — clear the current buffer.\n\nNeed technical details down to code modules? Enable developer mode (10 quick clicks on the “Settings” tab) and open the dev logs — a separate console window with the full event stream."),
    ("help.settings", "Настройки приложения.\n\n• Язык/Тема — переключаются мгновенно и сохраняются.\n• Кастомизация темы — цвета каждого элемента, фоновые изображения, пресеты.\n• Панель вкладок — расположение (сверху/снизу/слева/справа), стиль (обычный/компактный/иконки).\n• Поведение — запуск свёрнутой, сворачивание в трей, автозапуск с Windows, уровень логов.\n• Мост — порт локального API для расширения.\n• Монитор — интервал опроса, reverse DNS, авто-подхват процессов.\n• Обслуживание Zapret — проверка и установка новых релизов Flowseal со переносом списков и backup/rollback.\n• Разработчику — появляется после включения режима разработчика.", "Application settings.\n\n• Language/Theme — switch instantly and persist.\n• Theme customization — per-element colors, background images, presets.\n• Tab bar — position (top/bottom/left/right), style (full/compact/icons).\n• Behavior — start minimized, close to tray, autostart with Windows, log level.\n• Bridge — the port of the local extension API.\n• Monitor — poll interval, reverse DNS, process auto-inclusion.\n• Zapret maintenance — check and install new Flowseal releases with list migration and backup/rollback.\n• Developer — appears after developer mode is enabled."),
    // ── netmon traffic summary ────────────────────────────────
    ("netmon.active_apps", "Активные приложения", "Active apps"),
    ("netmon.active_now", "активно", "active"),
    ("netmon.connections", "Соединений", "Connections"),
    ("netmon.local_ips", "Ваши IP", "Your IPs"),
    ("netmon.top_hosts", "Основные адреса", "Top remotes"),
    ("netmon.summary_hint", "Приложения, которые сейчас забирают трафик — отсортированы по числу соединений.", "Apps currently consuming traffic — sorted by connections."),
    ("netmon.new_processes", "Новых процессов: {count}", "New processes: {count}"),
    ("netmon.copy_ip", "Скопировать IP", "Copy IP"),
    ("netmon.pick_title", "Добавить в «{list}» — выберите адреса", "Add to “{list}” — select addresses"),
    ("netmon.select_all", "Выбрать все", "Select all"),
    ("netmon.add_selected", "Добавить выбранные", "Add selected"),
    ("netmon.added_count", "Добавлено: {count} → {list}", "Added: {count} → {list}"),
    ("netmon.no_addresses", "Нет адресов для добавления", "No addresses to add"),
    // ── benchmark ─────────────────────────────────────────────
    ("bench.title", "Бенчмарк стратегий", "Strategy benchmark"),
    ("bench.desc", "Перебирает все general*.bat профиля: стоп → старт → прогрев → проверки сайтов → балл. Исходное состояние восстанавливается автоматически.", "Iterates every general*.bat of the profile: stop → start → warmup → site checks → score. The original state is restored automatically."),
    ("bench.start", "Запустить бенчмарк", "Run benchmark"),
    ("bench.cancel", "Отменить", "Cancel"),
    ("bench.running", "Проверяется: {strategy} ({index}/{total})", "Testing: {strategy} ({index}/{total})"),
    ("bench.best", "Лучшая стратегия", "Best strategy"),
    ("bench.score", "Балл", "Score"),
    ("bench.done", "Бенчмарк завершён. Лучшая: {strategy}", "Benchmark finished. Best: {strategy}"),
    ("bench.cancelled", "Бенчмарк отменён", "Benchmark cancelled"),
    ("bench.conflict", "Установлена служба — сначала удалите её: бенчмарк работает в обычном режиме.", "A service is installed — remove it first: the benchmark runs in normal mode."),
    ("bench.warmup", "Прогрев…", "Warming up…"),
    ("bench.checking", "Проверка сайтов…", "Checking sites…"),
    ("bench.restore", "Восстановление исходного состояния…", "Restoring original state…"),
    ("bench.success", "Успешных", "Successful"),
    ("bench.avg_latency", "Средняя задержка", "Average latency"),
    ("bench.error", "Ошибка: {error}", "Error: {error}"),
    ("bench.no_strategies", "В профиле нет стратегий general*.bat", "No general*.bat strategies in the profile"),
    // ── zapret maintenance ────────────────────────────────────
    ("maint.check", "Проверить обновления", "Check for updates"),
    ("maint.latest", "Последний релиз: {version}", "Latest release: {version}"),
    ("maint.installed", "Установлена: {version}", "Installed: {version}"),
    ("maint.not_installed_meta", "Версия не записана (установка вне Integra)", "Version unknown (installed outside Integra)"),
    ("maint.up_to_date", "У вас последняя версия", "You are on the latest version"),
    ("maint.download", "Скачать и установить {version}", "Download & install {version}"),
    ("maint.downloading", "Скачивание… {percent}%", "Downloading… {percent}%"),
    ("maint.extracting", "Распаковка…", "Extracting…"),
    ("maint.migrating", "Перенос списков…", "Migrating lists…"),
    ("maint.updated", "Zapret {version} установлен, списки перенесены", "Zapret {version} installed, lists migrated"),
    ("maint.failed", "Не удалось: {error}", "Failed: {error}"),
    ("maint.rollback", "Ошибка установки — исходная установка восстановлена", "Install failed — the original installation was restored"),
    ("maint.published", "Опубликовано: {date}", "Published: {date}"),
    ("maint.running_profile", "Профиль запущен — остановите его перед обновлением", "The profile is running — stop it before updating"),
    ("maint.network_error", "Ошибка сети: {error}", "Network error: {error}"),
    // ── misc additions ────────────────────────────────────────
    ("app.relaunch_admin", "Перезапустить от администратора", "Restart as administrator"),
    ("app.relaunched", "Запрошен перезапуск с правами администратора", "Elevated restart requested"),
    ("app.confirm_exit", "Выйти из Integra?", "Quit Integra?"),
    ("diag.kinds", "Проверки", "Checks"),
    ("diag.tls_note", "TLS — настоящий handshake (rustls), HTTP — HEAD по HTTPS.", "TLS — a real handshake (rustls), HTTP — HEAD over HTTPS."),
    ("diag.latency_ms", "{ms} мс", "{ms} ms"),
    ("diag.sites_for", "Сайты для проверки", "Sites to check"),
];

pub fn set_language(code: &str) -> &'static str {
    let normalized = if LANGUAGES.contains(&code) {
        code
    } else {
        "ru"
    };
    // SAFETY: only &'static str values from LANGUAGES are stored.
    let leaked: &'static str = Box::leak(normalized.to_string().into_boxed_str());
    *CURRENT.write() = leaked;
    leaked
}

pub fn language() -> String {
    CURRENT.read().to_string()
}

/// Translate a key for the current language.
pub fn tr(key: &str) -> String {
    tr_with(key, &[])
}

/// Translate with `{placeholder}` interpolation.
pub fn tr_with(key: &str, args: &[(&str, &str)]) -> String {
    let lang = *CURRENT.read();
    let entry = TABLE.iter().find(|(k, _, _)| *k == key);
    let template = match entry {
        Some((_, ru, en)) => {
            if lang == "en" {
                *en
            } else {
                *ru
            }
        }
        None => key,
    };
    let mut out = template.to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// Full bundle for the frontend: key → localized string.
pub fn bundle() -> HashMap<&'static str, &'static str> {
    let lang = *CURRENT.read();
    TABLE
        .iter()
        .map(|(k, ru, en)| (*k, if lang == "en" { *en } else { *ru }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The current language is process-global state; serialize the tests
    /// that touch it so parallel execution stays deterministic.
    static LANG_LOCK: Mutex<()> = Mutex::new(());

    fn lang_guard() -> std::sync::MutexGuard<'static, ()> {
        LANG_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn defaults_to_russian_and_switches() {
        let _guard = lang_guard();
        set_language("ru");
        assert_eq!(tr("nav.dashboard"), "Профили");
        set_language("en");
        assert_eq!(tr("nav.dashboard"), "Dashboard");
        set_language("zz"); // invalid → ru
        assert_eq!(tr("nav.dashboard"), "Профили");
    }

    #[test]
    fn interpolates_args() {
        let _guard = lang_guard();
        set_language("ru");
        assert_eq!(
            tr_with(
                "zapret.started",
                &[("strategy", "general.bat"), ("pid", "4242")]
            ),
            "Стратегия general.bat запущена (winws PID 4242)"
        );
    }

    #[test]
    fn unknown_key_returns_key() {
        assert_eq!(tr("no.such.key"), "no.such.key");
    }

    #[test]
    fn bundle_covers_all_keys() {
        let _guard = lang_guard();
        set_language("en");
        let bundle = bundle();
        assert_eq!(bundle.len(), TABLE.len());
        assert_eq!(bundle["nav.apps"], "Apps");
        // every help entry is present and non-trivial
        assert!(bundle["help.dashboard"].len() > 100);
        set_language("ru");
    }
}
