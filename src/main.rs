//! S3 Browser: a native file manager for Amazon S3 and S3-compatible storage
//! (Cloudflare R2, MinIO, Backblaze B2, Wasabi, DigitalOcean Spaces, GCS …),
//! built with GPUI Kit. (Keychain access survives rebuilds: scripts/sign.sh.)

mod app;
mod assets;
mod cache;
mod connection_form;
mod providers;
mod s3;
mod settings;
mod store;
mod theme;
mod transfers;
mod ui;
mod util;

use gpui_kit::component::TitleBar;
use gpui_kit::prelude::*;
use gpui_kit::{Bounds, WindowBounds, WindowOptions, px, size};

/// Panics inside AppKit callbacks abort without a message; keep one on disk.
fn install_panic_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let dir = store::config_dir();
        let _ = std::fs::create_dir_all(&dir);
        let line = format!("[{}] {info}\n{}\n", chrono::Local::now().to_rfc3339(), std::backtrace::Backtrace::force_capture());
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("panic.log"))
            .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
        default(info);
    }));
}

fn main() {
    install_panic_log();
    // Diagnostics: can this build read its saved keys without a prompt? (Should print "ok".)
    #[cfg(target_os = "macos")]
    if std::env::args().any(|a| a == "--check-keychain") {
        match store::check_keychain() {
            Ok(n) => println!("keychain ok: {n} saved key(s)"),
            Err(e) => println!("keychain error: {e}"),
        }
        return;
    }
    gpui_kit::application().with_assets(assets::AppAssets).run(|cx| {
        gpui_kit::init(cx);
        // Our own palette over gpui-component's theme (follows the system
        // unless the user picked light or dark).
        theme::apply(settings::Settings::load().dark, None, cx);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1360.), px(860.)), cx))),
            window_min_size: Some(size(px(980.), px(600.))),
            ..TitleBar::window_options()
        };
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| app::S3App::new(window, cx)))
            .expect("failed to open the main window");

        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}
