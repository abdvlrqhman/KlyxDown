mod bridge;
mod browser;
mod commands;
mod preview;
mod remote;
mod settings;
mod system;
mod tray;

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64},
    },
    time::Duration,
};

use klyxdown_engine::{Config, Engine, Event};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_window_state::StateFlags;
use tokio::sync::broadcast::error::RecvError;

/// Passed by the OS when the app starts at login.
const HIDDEN_ARG: &str = "--hidden";
/// Passed by the browser helper when it has to start the app: stay in the tray.
const BACKGROUND_ARG: &str = "--background";

pub struct AppState {
    pub engine: Engine,
    pub settings: Mutex<settings::Settings>,
    pub online: AtomicBool,
    pub launched_hidden: bool,
    pub watch_gen: Arc<AtomicU64>,
    /// Set once the UI has loaded. The window is never shown before that, so a blank or
    /// failed page can never appear.
    pub ui_ready: AtomicBool,
    /// Something asked for the window before the UI was ready.
    pub show_requested: AtomicBool,
    /// Browser hand-offs that arrived before the UI was ready.
    pub pending: Mutex<bridge::Pending>,
    pub decisions: bridge::Decisions,
    /// The browser extension has talked to us since launch.
    pub extension_seen: AtomicBool,
    pub finish_action: system::FinishAction,
    /// Found by `check_update`, installed by `install_update`.
    pub update: Mutex<Option<tauri_plugin_updater::Update>>,
    /// The user chose to download on this metered connection anyway (until it changes).
    pub metered_allowed: AtomicBool,
    /// A download waiting for its fresh link from the browser.
    pub refresh_wait: Mutex<Option<bridge::RefreshWait>>,
    /// The local feed for the preview player.
    pub preview: Option<preview::Preview>,
    /// Set to end the site grabber early.
    pub grab_stop: Arc<AtomicBool>,
    /// Sites with saved sign-ins; the engine reads their passwords from the keychain.
    pub sites: Arc<std::sync::RwLock<Vec<settings::Site>>>,
    /// The phone remote, while it is on.
    pub remote: Mutex<Option<remote::Running>>,
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "klyxdown=info,klyxdown_engine=info".into()),
        )
        .init();

    tauri::Builder::default()
        // First plugin on purpose: a second launch focuses this window instead of starting a new app.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !bridge::capture_from_args(app, &args) {
                tray::show_main(app);
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_ARG]),
        ))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                // The UI decides when to show the window, so never restore visibility.
                .with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();
            let settings = settings::load(&handle);
            let sites = Arc::new(std::sync::RwLock::new(settings.sites.clone()));
            let lookup = sites.clone();
            let credentials: klyxdown_engine::auth::Credentials = Arc::new(move |host| {
                let sites = lookup.read().unwrap_or_else(|e| e.into_inner());
                let site = settings::site_for(&sites, host)?;
                let password = settings::keychain_entry(site).and_then(|e| e.get_password());
                password
                    .inspect_err(|e| tracing::warn!(host, "no saved password: {e}"))
                    .ok()
                    .map(|p| (site.user.clone(), p))
            });
            let config = Config {
                max_active: settings.max_active,
                mode: settings.mode,
                limit: settings.limit,
                proxy: settings.proxy(),
                quota: settings.quota(),
                seed_ratio: settings.seed_ratio,
                scan: settings.scan(),
                credentials: Some(credentials),
                data_dir: system::data_dir(&handle).ok(),
                db_path: system::data_dir(&handle).ok().map(|d| {
                    let _ = std::fs::create_dir_all(&d);
                    d.join("downloads.db")
                }),
                ..Config::default()
            };
            // The engine spawns its scheduler, so build it inside the async runtime.
            let engine = tauri::async_runtime::block_on(async move { Engine::new(config) });
            let feed = engine.clone();
            let preview = tauri::async_runtime::block_on(preview::start(feed))
                .inspect_err(|e| tracing::warn!("no preview player: {e}"))
                .ok();
            let args: Vec<String> = std::env::args().collect();
            let launched_hidden = args.iter().any(|a| a == BACKGROUND_ARG)
                || (settings.start_hidden && args.iter().any(|a| a == HIDDEN_ARG));
            // Paint the window in the theme's chassis color before the webview loads (no flash).
            if let Some(window) = app.get_webview_window("main") {
                let _ =
                    window.set_background_color(Some(settings::theme_background(&settings.theme)));
            }

            app.manage(AppState {
                engine: engine.clone(),
                settings: Mutex::new(settings),
                online: AtomicBool::new(true),
                launched_hidden,
                watch_gen: Arc::new(AtomicU64::new(0)),
                ui_ready: AtomicBool::new(false),
                show_requested: AtomicBool::new(false),
                pending: Mutex::new(bridge::Pending::default()),
                decisions: bridge::Decisions::default(),
                extension_seen: AtomicBool::new(false),
                finish_action: system::FinishAction::default(),
                update: Mutex::new(None),
                metered_allowed: AtomicBool::new(false),
                refresh_wait: Mutex::new(None),
                preview,
                grab_stop: Arc::default(),
                sites,
                remote: Mutex::new(None),
            });
            commands::apply_remote(&handle);
            system::register_magnets(&handle);
            tray::create(&handle)?;
            bridge::start(handle.clone());
            system::start(handle.clone());
            bridge::capture_from_args(&handle, &args);
            let register = handle.clone();
            std::thread::spawn(move || bridge::register_native_host(&register));
            forward_engine_events(handle, engine);
            Ok(())
        })
        .on_window_event(|window, event| {
            let Some(state) = window.try_state::<AppState>() else {
                return;
            };
            let (to_tray_on_close, to_tray_on_minimize) = {
                let s = state.settings.lock().unwrap_or_else(|e| e.into_inner());
                (s.close_to_tray, s.minimize_to_tray)
            };
            match event {
                WindowEvent::CloseRequested { api, .. } if to_tray_on_close => {
                    api.prevent_close();
                    let _ = window.hide();
                }
                // Catches OS-initiated minimize too (taskbar click, Win+Down).
                WindowEvent::Resized(_)
                    if to_tray_on_minimize && window.is_minimized().unwrap_or(false) =>
                {
                    let _ = window.hide();
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::probe,
            commands::suggest_dir,
            commands::file_hash,
            commands::expand_batch,
            commands::read_metalink,
            commands::list_zip,
            commands::refresh_link,
            commands::wait_for_link,
            commands::import_links,
            commands::capture_done,
            commands::extension_seen,
            commands::open_project_page,
            commands::set_finish_action,
            commands::run_finish_action,
            commands::is_portable,
            commands::check_update,
            commands::install_update,
            commands::add,
            commands::pause,
            commands::resume,
            commands::restart,
            commands::remove,
            commands::pause_all,
            commands::resume_all,
            commands::list,
            commands::watch,
            commands::queues,
            commands::add_queue,
            commands::update_queue,
            commands::remove_queue,
            commands::move_to_queue,
            commands::allow_metered,
            commands::set_tray_text,
            commands::open_browser,
            commands::remote_info,
            commands::save_site,
            commands::remove_site,
            commands::list_folder,
            commands::grab,
            commands::stop_grab,
            commands::tools_status,
            commands::install_tools,
            commands::media_info,
            commands::inspect_torrent,
            commands::convert,
            commands::convert_presets,
            commands::set_seeding,
            commands::preview_url,
            commands::set_mode,
            commands::set_limit,
            commands::network_changed,
            commands::get_settings,
            commands::save_settings,
            commands::ui_ready,
            commands::open_file,
            commands::reveal_file,
            commands::quit,
        ])
        .build(tauri::generate_context!())
        .expect("KlyxDown failed to start")
        .run(|app, event| match event {
            // Save every download's progress before the process ends.
            tauri::RunEvent::Exit => {
                if let Some(state) = app.try_state::<AppState>() {
                    state.engine.shutdown();
                }
            }
            // macOS hands over opened .torrent files this way.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Opened { urls } => {
                let args: Vec<String> = urls
                    .iter()
                    .map(|u| {
                        u.to_file_path()
                            .map_or_else(|_| u.to_string(), |p| p.to_string_lossy().into_owned())
                    })
                    .collect();
                bridge::capture_from_args(app, &args);
            }
            _ => {}
        });
}

/// State changes go to the UI as events; the tray tooltip refreshes every second.
fn forward_engine_events(app: AppHandle, engine: Engine) {
    let mut rx = engine.subscribe();
    let events_app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(Event::Changed(view)) => {
                    let _ = events_app.emit("download:changed", &view);
                }
                Ok(Event::Removed(id)) => {
                    let _ = events_app.emit("download:removed", id);
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            tray::refresh_status(&app);
        }
    });
}
