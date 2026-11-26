#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod background;
mod cleanup;
mod cleanup_ui;
mod command;
mod docker;
mod docker_logs;
mod docker_resources;
mod docker_terminal;
mod environment;
mod environment_catalog;
mod environment_icons;
mod environment_origin;
mod environment_settings;
mod environment_variables;
mod gpu;
mod logs;
mod logs_access;
mod logs_list;
mod logs_ui;
mod overview;
mod ports;
mod process_control;
mod processes;
#[cfg(not(windows))]
mod profile;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod profile_metadata;
mod services;
mod services_ui;
mod storage;
mod ui;
mod update;
mod volumes;
mod worker;

fn main() -> eframe::Result {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "--vijideck-apply-update")
    {
        update::run_helper(&args[1..]);
        return Ok(());
    }
    let mut update_error = None;
    if let [command, id, error] = args.as_slice()
        && command == "--vijideck-update-result"
        && let Some(id) = id.to_str()
    {
        let id = id.to_owned();
        std::thread::spawn(move || {
            if let Err(error) = update::finish(&id) {
                eprintln!("{error}");
            }
        });
        update_error = error
            .to_str()
            .filter(|error| !error.is_empty())
            .map(str::to_owned);
    }
    let icon = image::load_from_memory(include_bytes!(
        "../packaging/linux/icons/hicolor/64x64/apps/vijideck.png"
    ))
    .expect("embedded application icon is a valid PNG")
    .to_rgba8();
    let (width, height) = icon.dimensions();
    eframe::run_native(
        "VijiDeck",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_transparent(true)
                .with_icon(std::sync::Arc::new(eframe::egui::IconData {
                    rgba: icon.into_raw(),
                    width,
                    height,
                }))
                .with_inner_size(initial_window_size())
                .with_min_inner_size([900.0, 580.0]),
            ..Default::default()
        },
        Box::new(move |cc| Ok(Box::new(app::VijiDeck::new(cc, update_error)))),
    )
}

fn initial_window_size() -> [f32; 2] {
    #[cfg(feature = "ui-preview")]
    if std::env::var_os("VIJIDECK_SCREENSHOT_TO").is_some()
        && std::env::var_os("VIJIDECK_PREVIEW_COMPACT").is_some()
    {
        return [900.0, 580.0];
    }
    [1320.0, 820.0]
}
