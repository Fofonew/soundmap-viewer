//! SoundMap — application desktop (Tauri 2).
//!
//! - Mode Exploration : tuiles WMTS/TMS servies par `soundmap://` (proxy Rust
//!   avec cache disque, quotas et UA générique) ; aucun appel LiDAR HD.
//! - Zone de travail : estimation → téléchargement du pack → vue 3D hors ligne.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod protocol;
mod sim;
mod state;

use tauri::Manager;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tao=warn,wry=warn".into()),
        )
        .init();

    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let st = state::AppState::new(&data_dir)?;
            tracing::info!(zones = %st.zones_dir.display(), cache = %st.cache.root().display(), offline = st.offline, "SoundMap prêt");
            app.manage(st);
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .register_asynchronous_uri_scheme_protocol("soundmap", protocol::handle)
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::geocode,
            commands::estimate_zone,
            commands::download_zone,
            commands::resume_zone,
            commands::cancel_download,
            commands::list_zones,
            commands::open_zone,
            commands::delete_zone,
            commands::log_metric,
            sim::catalog,
            sim::load_scene,
            sim::compute_preview,
            sim::result_layer,
            sim::section_at,
            sim::run_lowfreq,
            sim::cancel_job,
            sim::compute_impact,
            sim::compute_sizing,
            sim::calibrate_measurements,
            sim::fetch_vegetation,
            sim::fetch_ground_data,
            sim::find_spots,
            sim::auralize_point,
            sim::run_optimizer,
            sim::save_project,
            sim::load_project,
            sim::autosave_project,
            sim::autoload_project,
            sim::export_result,
        ])
        .run(tauri::generate_context!())?;
    Ok(())
}
