use std::sync::{Mutex, atomic::Ordering};

use serde::Deserialize;
use tauri::{
    AppHandle, Manager, Runtime,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use crate::AppState;

const TRAY_ID: &str = "main";

/// Tray menu and tooltip text in the interface language; the UI sends it (it has the
/// translations). `busy` contains `{n}` and `{speed}`.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayText {
    show: String,
    pause_all: String,
    resume_all: String,
    quit: String,
    idle: String,
    busy: String,
    offline: String,
}

impl Default for TrayText {
    fn default() -> Self {
        Self {
            show: "Show KlyxDown".into(),
            pause_all: "Pause all".into(),
            resume_all: "Resume all".into(),
            quit: "Quit KlyxDown".into(),
            idle: "KlyxDown: nothing downloading".into(),
            busy: "KlyxDown: {n} downloading at {speed} MB/s".into(),
            offline: "KlyxDown: offline, downloads resume when you're back".into(),
        }
    }
}

pub type TrayTextState = Mutex<TrayText>;

fn menu<R: Runtime>(app: &AppHandle<R>, text: &TrayText) -> tauri::Result<Menu<R>> {
    let show = MenuItem::with_id(app, "show", &text.show, true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause_all", &text.pause_all, true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume_all", &text.resume_all, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", &text.quit, true, None::<&str>)?;
    let sep = || PredefinedMenuItem::separator(app);
    Menu::with_items(app, &[&show, &sep()?, &pause, &resume, &sep()?, &quit])
}

/// Rebuilds the tray menu in a new language.
pub fn set_text<R: Runtime>(app: &AppHandle<R>, text: TrayText) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(menu(app, &text)?))?;
    }
    *app.state::<TrayTextState>()
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = text;
    refresh_status(app);
    Ok(())
}

pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    app.manage(TrayTextState::default());
    let menu = menu(app, &TrayText::default())?;

    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("KlyxDown")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "pause_all" => app.state::<AppState>().engine.pause_all(),
            "resume_all" => app.state::<AppState>().engine.resume_all(),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Brings the window back from the tray, a minimized state, or behind other windows.
/// Before the UI has loaded it only remembers the request; the UI shows the window
/// when it is ready (see `commands::ui_ready`), so an unpainted page never appears.
pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(state) = app.try_state::<AppState>()
        && !state.ui_ready.load(Ordering::SeqCst)
    {
        state.show_requested.store(true, Ordering::SeqCst);
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Keeps the tray tooltip in sync with what the engine is doing.
pub fn refresh_status<R: Runtime>(app: &AppHandle<R>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let state = app.state::<AppState>();
    let words = app
        .state::<TrayTextState>()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let text = if !state.online.load(Ordering::Relaxed) {
        words.offline
    } else {
        let running = state
            .engine
            .list()
            .iter()
            .filter(|d| d.state == klyxdown_engine::State::Downloading)
            .count();
        match running {
            0 => words.idle,
            n => words.busy.replace("{n}", &n.to_string()).replace(
                "{speed}",
                &format!("{:.1}", state.engine.total_speed() as f64 / 1e6),
            ),
        }
    };
    let _ = tray.set_tooltip(Some(text));
}
