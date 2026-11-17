//! The native macOS menu bar and keyboard shortcuts.
//!
//! App-wide actions (Quit, Hide) are handled here; everything that needs the
//! window (New Connection, Upload, views, navigation…) is handled by
//! `S3App` through `on_action` listeners in `ui/mod.rs`.

use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::{App, KeyBinding, Menu, MenuItem, SystemMenuType};

gpui_kit::actions!(
    s3_browser,
    [
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        About,
        OpenSettings,
        NewConnection,
        UploadFiles,
        UploadFolder,
        NewFolder,
        CloseWindow,
        Refresh,
        GoBack,
        GoForward,
        GoUp,
        ViewGrid,
        ViewList,
        ViewMedia,
        ZoomIn,
        ZoomOut,
        ToggleSidebar,
        ToggleInspector,
        ShowTransfers,
        ToggleTheme,
        Minimize,
        ZoomWindow,
    ]
);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-n", NewConnection, None),
        KeyBinding::new("cmd-u", UploadFiles, None),
        KeyBinding::new("shift-cmd-u", UploadFolder, None),
        KeyBinding::new("shift-cmd-n", NewFolder, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-r", Refresh, None),
        KeyBinding::new("cmd-[", GoBack, None),
        KeyBinding::new("cmd-]", GoForward, None),
        KeyBinding::new("cmd-up", GoUp, None),
        KeyBinding::new("cmd-1", ViewGrid, None),
        KeyBinding::new("cmd-2", ViewList, None),
        KeyBinding::new("cmd-3", ViewMedia, None),
        KeyBinding::new("cmd-=", ZoomIn, None),
        KeyBinding::new("cmd--", ZoomOut, None),
        KeyBinding::new("alt-cmd-s", ToggleSidebar, None),
        KeyBinding::new("alt-cmd-i", ToggleInspector, None),
        KeyBinding::new("alt-cmd-t", ShowTransfers, None),
        KeyBinding::new("cmd-m", Minimize, None),
    ]);

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());

    cx.set_menus([
        Menu {
            name: "S3 Browser".into(),
            items: vec![
                MenuItem::action("About S3 Browser", About),
                MenuItem::separator(),
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide S3 Browser", Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit S3 Browser", Quit),
            ],
            disabled: false,
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Connection…", NewConnection),
                MenuItem::separator(),
                MenuItem::action("Upload Files…", UploadFiles),
                MenuItem::action("Upload Folder…", UploadFolder),
                MenuItem::action("New Folder", NewFolder),
                MenuItem::separator(),
                MenuItem::action("Close Window", CloseWindow),
            ],
            disabled: false,
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::action("Undo", Undo),
                MenuItem::action("Redo", Redo),
                MenuItem::separator(),
                MenuItem::action("Cut", Cut),
                MenuItem::action("Copy", Copy),
                MenuItem::action("Paste", Paste),
                MenuItem::action("Select All", SelectAll),
            ],
            disabled: false,
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("as Grid", ViewGrid),
                MenuItem::action("as List", ViewList),
                MenuItem::action("as Media", ViewMedia),
                MenuItem::separator(),
                MenuItem::action("Bigger Thumbnails", ZoomIn),
                MenuItem::action("Smaller Thumbnails", ZoomOut),
                MenuItem::separator(),
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
                MenuItem::action("Toggle Inspector", ToggleInspector),
                MenuItem::action("Show Transfers", ShowTransfers),
                MenuItem::separator(),
                MenuItem::action("Toggle Light / Dark", ToggleTheme),
            ],
            disabled: false,
        },
        Menu {
            name: "Go".into(),
            items: vec![
                MenuItem::action("Back", GoBack),
                MenuItem::action("Forward", GoForward),
                MenuItem::action("Enclosing Folder", GoUp),
                MenuItem::separator(),
                MenuItem::action("Refresh", Refresh),
            ],
            disabled: false,
        },
        Menu {
            name: "Window".into(),
            items: vec![MenuItem::action("Minimize", Minimize), MenuItem::action("Zoom", ZoomWindow)],
            disabled: false,
        },
    ]);
}
