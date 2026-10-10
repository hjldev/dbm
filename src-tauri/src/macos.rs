//! Tao 0.37 does not implement applicationShouldTerminate:, so macOS Quit
//! bypasses Tauri's ExitRequested event. Supply only that missing delegate
//! method; every normal Quit (including Dock Quit) then uses our window guard.
use objc2::{
    class, ffi, msg_send,
    runtime::{AnyClass, AnyObject, Imp, Sel},
    sel,
};
use std::sync::OnceLock;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    AppHandle, Emitter, Manager,
};

static APP: OnceLock<AppHandle> = OnceLock::new();

unsafe extern "C-unwind" fn should_terminate(
    _delegate: *mut AnyObject,
    _selector: Sel,
    _application: *mut AnyObject,
) -> usize {
    if let Some(window) = APP.get().and_then(|app| app.get_webview_window("main")) {
        let _ = window.close();
        0 // NSTerminateCancel: JS will destroy the window after approval.
    } else {
        1 // NSTerminateNow: no workspace remains to protect.
    }
}

pub fn install_quit_guard(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    APP.set(app.clone())
        .map_err(|_| "Quit guard already installed")?;
    // Called on the main thread during Tauri setup. NSApplication owns its
    // delegate for the entire run loop; no retained references are transferred.
    unsafe {
        let application: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let delegate: *mut AnyObject = msg_send![application, delegate];
        let delegate = delegate.as_ref().ok_or("Missing application delegate")?;
        let method = std::mem::transmute::<
            unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize,
            Imp,
        >(should_terminate);
        // NSUInteger return, self, selector, NSApplication argument on 64-bit macOS.
        let installed = ffi::class_addMethod(
            delegate.class() as *const AnyClass as *mut AnyClass,
            sel!(applicationShouldTerminate:),
            method,
            c"Q@:@".as_ptr(),
        );
        if !installed.as_bool() {
            return Err("Cannot install macOS quit guard".into());
        }
    }
    install_menu(app)?;
    Ok(())
}

fn install_menu(app: &AppHandle) -> tauri::Result<()> {
    let menu = Menu::with_items(
        app,
        &[
            &Submenu::with_items(
                app,
                "DBM",
                true,
                &[
                    &PredefinedMenuItem::about(app, Some("关于 DBM"), None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::services(app, Some("服务"))?,
                    &PredefinedMenuItem::hide(app, Some("隐藏 DBM"))?,
                    &PredefinedMenuItem::hide_others(app, Some("隐藏其他应用"))?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::quit(app, Some("退出 DBM"))?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "文件",
                true,
                &[&MenuItem::with_id(
                    app,
                    "close-tab",
                    "关闭标签",
                    true,
                    Some("CmdOrCtrl+W"),
                )?],
            )?,
            &Submenu::with_items(
                app,
                "编辑",
                true,
                &[
                    &PredefinedMenuItem::undo(app, Some("撤销"))?,
                    &PredefinedMenuItem::redo(app, Some("重做"))?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::cut(app, Some("剪切"))?,
                    &PredefinedMenuItem::copy(app, Some("复制"))?,
                    &PredefinedMenuItem::paste(app, Some("粘贴"))?,
                    &PredefinedMenuItem::select_all(app, Some("全选"))?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "窗口",
                true,
                &[
                    &PredefinedMenuItem::minimize(app, Some("最小化"))?,
                    &PredefinedMenuItem::fullscreen(app, Some("切换全屏"))?,
                ],
            )?,
        ],
    )?;
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        if event.id().as_ref() == "close-tab" {
            let _ = app.emit("dbm-close-tab", ());
        }
    });
    Ok(())
}
