mod commands;
mod db;
mod error;
mod models;
mod mt5;
mod services;
mod state;
mod stats;
mod util;

use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let state = tauri::async_runtime::block_on(AppState::new(app.handle()))?;
            app.manage(state.clone());

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = state.start_server(handle).await {
                    log::error!("No se pudo iniciar el servidor MT5: {error}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap,
            commands::list_accounts,
            commands::create_account,
            commands::update_account,
            commands::confirm_account,
            commands::archive_account,
            commands::set_active_account,
            commands::list_strategies,
            commands::get_strategy,
            commands::create_strategy,
            commands::update_strategy,
            commands::archive_strategy,
            commands::duplicate_strategy,
            commands::list_asset_groups,
            commands::get_asset_group,
            commands::create_asset_group,
            commands::update_asset_group,
            commands::delete_asset_group,
            commands::add_group_to_day,
            commands::get_day,
            commands::create_day_asset,
            commands::delete_day_asset,
            commands::select_asset_strategy,
            commands::toggle_plan_item,
            commands::add_plan_item,
            commands::delete_plan_item,
            commands::save_day_journal,
            commands::save_asset_journal,
            commands::save_image,
            commands::list_images,
            commands::delete_image,
            commands::list_trades,
            commands::get_trade,
            commands::update_trade_notes,
            commands::assign_trade_strategy,
            commands::assign_trade_to_asset,
            commands::get_trade_series,
            commands::request_trade_series,
            commands::request_sync,
            commands::get_stats,
            commands::get_settings,
            commands::update_settings,
            commands::regenerate_api_key,
            commands::get_mt5_status,
            commands::restart_mt5_server,
            commands::open_data_dir,
            commands::create_backup,
        ])
        .run(tauri::generate_context!())
        .expect("error while building tauri application");
}
