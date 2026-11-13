mod atas;
mod commands;
mod copy;
mod db;
mod error;
mod installers;
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
            atas::spawn(state.clone());

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
            commands::get_performance_calendar,
            commands::get_settings,
            commands::update_settings,
            commands::regenerate_api_key,
            commands::get_mt5_status,
            commands::get_nt8_status,
            commands::restart_mt5_server,
            commands::restart_bridge_server,
            commands::write_nt8_config,
            commands::open_nt8_folder,
            commands::get_atas_status,
            commands::write_atas_config,
            commands::open_atas_indicators_folder,
            commands::open_data_dir,
            commands::create_backup,
            installers::install_nt8_bridge,
            installers::install_atas_bridge,
            copy::commands::get_copy_overview,
            copy::commands::create_copy_group,
            copy::commands::update_copy_group,
            copy::commands::delete_copy_group,
            copy::commands::set_copy_enabled,
            copy::commands::set_copy_group_enabled,
            copy::commands::add_copy_follower,
            copy::commands::remove_copy_follower,
            copy::commands::update_copy_follower,
            copy::commands::set_copy_follower_enabled,
            copy::commands::resume_copy_follower,
            copy::commands::copy_panic,
            copy::commands::list_copy_log,
            copy::commands::preview_copy_size,
        ])
        .run(tauri::generate_context!())
        .expect("error while building tauri application");
}
