mod app;
mod config;
mod fs;
mod keychain;
mod ops;
mod ui;
mod util;

rust_i18n::i18n!("locales", fallback = "en");

fn main() -> eframe::Result {
    let config = config::Config::load();
    let locale = config
        .language
        .clone()
        .or_else(sys_locale::get_locale)
        .unwrap_or_else(|| "en".to_string());
    // Map system locales like "zh-Hant-TW" / "zh-TW" onto our zh-TW bundle.
    if locale.starts_with("zh") {
        rust_i18n::set_locale("zh-TW");
    } else {
        rust_i18n::set_locale("en");
    }

    let (w, h) = config.window_size.unwrap_or((1150.0, 720.0));
    let viewport = egui::ViewportBuilder::default()
        .with_title("Your Commander")
        .with_inner_size([w.max(800.0), h.max(500.0)])
        .with_min_inner_size([800.0, 500.0]);
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "your-commander",
        options,
        Box::new(|cc| Ok(Box::new(app::YcApp::new(cc, config)))),
    )
}
