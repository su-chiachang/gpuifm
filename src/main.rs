use gpui::{
    App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds,
    WindowOptions, px, size,
};
use nanofm::ui::actions::Quit;
use nanofm::fm_core::places::home_dir;
use nanofm::fm_core::state::{StateStore, state_dir};
use nanofm::ui::browser::{Browser, BrowserOptions};
use nanofm::ui::opener::SystemOpener;
use std::rc::Rc;

fn main() {
    // Optional first argument: directory to open instead of $HOME (used for smoke tests).
    let start = std::env::args().nth(1).map(std::path::PathBuf::from).map(|p| p.canonicalize().unwrap_or(p));
    // State lives in ~/Library/Application Support/nanofm-rs (never the Swift app's `nanofm`);
    // NANOFM_STATE_DIR overrides it (used for smoke tests).
    let state = state_dir(std::env::var_os("NANOFM_STATE_DIR").as_deref(), &home_dir());
    Application::new().run(|cx: &mut App| {
        nanofm::ui::keymap::bind(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        nanofm::ui::menus::rebuild(cx, None, None);

        let bounds = Bounds::centered(None, size(px(1000.), px(640.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("nanofm".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |window, cx| {
                let opts = BrowserOptions {
                    start,
                    store: Some(StateStore::new(state)),
                    index_roots: None,
                    opener: Rc::new(SystemOpener::default()),
                    watch: true,
                };
                cx.new(|cx| Browser::with_options(opts, window, cx))
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
