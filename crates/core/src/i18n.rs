//! RU/EN translations. Single source of truth for backend messages and the
//! frontend bundle (the UI fetches the whole map via the `i18n_bundle` command).

use parking_lot::RwLock;
use std::collections::HashMap;

pub const LANGUAGES: [&str; 2] = ["ru", "en"];

static CURRENT: RwLock<&str> = RwLock::new("ru");

/// (key, ru, en)
static TABLE: &[(&str, &str, &str)] = &[
    // ── common ────────────────────────────────────────────────
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
    ("common.open", "Открыть", "Open"),
    ("common.name", "Имя", "Name"),
    ("common.path", "Путь", "Path"),
    ("common.status", "Состояние", "Status"),
    ("common.actions", "Действия", "Actions"),
    ("common.version", "Версия", "Version"),
    ("common.apply", "Применить", "Apply"),
    ("common.port_invalid", "Порт должен быть 1–65535", "Port must be 1–65535"),
    ("common.soon", "Появится в следующей итерации", "Coming in the next iteration"),
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
    // ── settings ──────────────────────────────────────────────
    ("settings.title", "Настройки", "Settings"),
    ("settings.language", "Язык интерфейса", "Interface language"),
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
];

pub fn set_language(code: &str) -> &'static str {
    let normalized = if LANGUAGES.contains(&code) { code } else { "ru" };
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
        Some((_, ru, en)) => if lang == "en" { *en } else { *ru },
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

    #[test]
    fn defaults_to_russian_and_switches() {
        assert_eq!(tr("nav.dashboard"), "Профили");
        set_language("en");
        assert_eq!(tr("nav.dashboard"), "Dashboard");
        set_language("zz"); // invalid → ru
        assert_eq!(tr("nav.dashboard"), "Профили");
    }

    #[test]
    fn interpolates_args() {
        set_language("ru");
        assert_eq!(
            tr_with("zapret.started", &[("strategy", "general.bat"), ("pid", "4242")]),
            "Стратегия general.bat запущена (winws PID 4242)"
        );
    }

    #[test]
    fn unknown_key_returns_key() {
        assert_eq!(tr("no.such.key"), "no.such.key");
    }

    #[test]
    fn bundle_covers_all_keys() {
        set_language("en");
        let bundle = bundle();
        assert_eq!(bundle.len(), TABLE.len());
        assert_eq!(bundle["nav.apps"], "Apps");
    }
}
