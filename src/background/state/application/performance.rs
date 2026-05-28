use std::sync::LazyLock;

use arc_swap::ArcSwap;
use libs_core::{handlers::FuncEvent, state::PerformanceMode};
use tauri::{Emitter, Listener};

use crate::{
    app::get_app_handle,
    error::{ErrorMap, ResultLogExt},
    hook::HookManager,
    state::application::FULL_STATE,
    windows_api::window::{event::WinEvent, Window},
};

pub static PERFORMANCE_MODE: LazyLock<ArcSwap<PerformanceMode>> = LazyLock::new(|| {
    start_listeners();
    let perf_mode = get_perf_mode();
    log::info!("Performance mode: {perf_mode:?}");
    ArcSwap::from_pointee(perf_mode)
});

fn start_listeners() {
    let handle = get_app_handle();
    handle.listen(FuncEvent::StateSettingsChanged, |_| check_for_changes());

    HookManager::subscribe(|(event, _origin)| {
        if matches!(
            event,
            WinEvent::SystemForeground
                | WinEvent::SyntheticFullscreenStart
                | WinEvent::SyntheticFullscreenEnd
        ) {
            check_for_changes();
        }
    });
}

fn get_perf_mode() -> PerformanceMode {
    let foreground = Window::get_foregrounded();
    if foreground.is_fullscreen() && !foreground.is_bar_overlay() {
        return PerformanceMode::Extreme;
    }

    let guard = FULL_STATE.load();
    let config = &guard.settings.performance_mode;

    config.default
}

fn check_for_changes() {
    let stored = PERFORMANCE_MODE.load_full();
    let current = get_perf_mode();
    if current != *stored {
        log::trace!("UI performance mode changed to {current:?}");
        PERFORMANCE_MODE.store(std::sync::Arc::new(current));
        get_app_handle()
            .emit(FuncEvent::StatePerformanceModeChanged, current)
            .wrap_error()
            .log_error();
    }
}
