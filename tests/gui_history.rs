//! Explicit desktop acceptance; excluded from routine automated checks.
#[allow(dead_code)]
#[path = "../src/ui.rs"]
mod ui;
use clevo_fan_control::{config::Settings, platform::SingleInstance, worker::Worker};
use fltk::{app, button::CheckButton, group::Group, prelude::*};
use std::{
    cell::Cell,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

fn find(group: &Group, label: &str) -> Option<fltk::widget::Widget> {
    for i in 0..group.children() {
        let widget = group.child(i)?;
        if widget.label() == label {
            return Some(widget);
        }
        if let Some(group) = widget.as_group()
            && let Some(widget) = find(&group, label)
        {
            return Some(widget);
        }
    }
    None
}
fn widget(label: &str) -> fltk::widget::Widget {
    for window in ui::acceptance::windows().into_iter().chain(
        app::windows()
            .unwrap_or_default()
            .into_iter()
            .map(|w| unsafe { w.into_widget::<fltk::window::Window>() }),
    ) {
        if let Some(widget) = find(&window.as_group().unwrap(), label) {
            return widget;
        }
    }
    panic!("Widget not found: {label}")
}
fn click(label: &str) {
    widget(label).do_callback();
}
fn has_text(needle: &str) -> bool {
    fn walk(group: &Group, needle: &str) -> bool {
        (0..group.children())
            .filter_map(|i| group.child(i))
            .any(|w| w.label().contains(needle) || w.as_group().is_some_and(|g| walk(&g, needle)))
    }
    ui::acceptance::windows()
        .iter()
        .any(|w| walk(&w.as_group().unwrap(), needle))
}
fn recording(enabled: bool) {
    check("后台历史记录（独立保存，重启保持）", enabled);
}
fn check(label: &str, enabled: bool) {
    let mut button = unsafe { CheckButton::from_widget_ptr(widget(label).as_widget_ptr()) };
    button.set_value(enabled);
    button.do_callback();
}

#[test]
#[ignore = "requires an isolated desktop session and the controller lock"]
fn desktop_history_workflow() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        std::process::exit(101);
    }));
    let _instance = SingleInstance::acquire().unwrap();
    let root = PathBuf::from(std::env::var("CFC_GUI_ROOT").expect("CFC_GUI_ROOT"));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("ClevoFanControl.x64.json");
    let case = std::env::var("CFC_GUI_CASE").unwrap_or_else(|_| "functional".into());
    let mut settings = Settings {
        close_to_tray: true,
        ..Settings::default()
    };
    settings.global.update_interval = std::env::var("CFC_GUI_INTERVAL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.2);
    settings.history_recording_enabled = std::env::var("CFC_GUI_RECORD").as_deref() == Ok("true");
    if case == "restart" {
        settings = Settings::load(&path).unwrap();
        assert!(settings.history_recording_enabled);
    }
    if case == "open-failure" {
        settings.history_recording_enabled = true;
        std::fs::write(root.join("history-demo"), "blocked history path").unwrap();
    }
    let load_error = if case == "invalid" {
        std::fs::write(&path, b"invalid original").unwrap();
        Some("invalid original".into())
    } else {
        settings.save(&path).unwrap();
        None
    };
    let initial = std::fs::read(&path).unwrap();
    let worker = Worker::start(settings.clone(), true);
    let legacy = root.join("legacy.json");
    let mut old = serde_json::to_value(&settings.global).unwrap();
    let object = old.as_object_mut().unwrap();
    object.insert("updateInterval".into(), 2.into());
    object.insert("forceCooling".into(), false.into());
    object.insert("autoRun".into(), false.into());
    object.insert("closeToTray".into(), true.into());
    object.insert("fontSize".into(), 10.into());
    std::fs::write(&legacy, serde_json::to_vec(&old).unwrap()).unwrap();
    ui::acceptance::set_import(legacy);
    let finished = Rc::new(Cell::new(false));
    let checked = finished.clone();
    let exit_at = Rc::new(Cell::new(None::<Instant>));
    let p = path.clone();
    let r = root.clone();
    if case == "functional" {
        app::add_timeout3(0.3, move |_| {
            click("设置");
            recording(true);
            click("保存并应用");
        });
        let p1 = p.clone();
        app::add_timeout3(0.9, move |_| {
            assert!(Settings::load(&p1).unwrap().history_recording_enabled);
            click("返回");
            check("自动匹配进程", true);
            recording(false);
        });
        let p2 = p.clone();
        app::add_timeout3(1.4, move |_| {
            let s = Settings::load(&p2).unwrap();
            assert!(!s.history_recording_enabled);
            assert!(!s.auto_switch);
            recording(true);
        });
        app::add_timeout3(1.9, move |_| {
            let mut window = ui::acceptance::windows()
                .into_iter()
                .find(|w| w.label().starts_with("ClevoFanControl ·"))
                .unwrap();
            window.do_callback();
        });
        let p3 = p.clone();
        app::add_timeout3(2.7, move |_| {
            assert!(Settings::load(&p3).unwrap().history_recording_enabled);
        });
        app::add_timeout3(3.0, move |_| {
            click("设置");
            click("返回");
            click("保存并应用");
        });
        let p4 = p.clone();
        app::add_timeout3(3.7, move |_| {
            let saved = Settings::load(&p4).unwrap();
            assert!(saved.history_recording_enabled);
            assert!(saved.auto_switch);
        });
        app::add_timeout3(3.8, move |_| click("重新加载"));
        app::add_timeout3(4.1, move |_| click("导入旧配置"));
        let p5 = p.clone();
        app::add_timeout3(4.5, move |_| {
            let saved = Settings::load(&p5).unwrap();
            assert!(saved.history_recording_enabled);
            assert_eq!(saved.global.update_interval, 0.2);
        });
        app::add_timeout3(4.8, move |_| {
            let mut window = ui::acceptance::windows()
                .into_iter()
                .find(|w| w.label().starts_with("ClevoFanControl ·"))
                .unwrap();
            window.show();
            click("退出");
            checked.set(true);
        });
        app::add_timeout3(5.0, move |_| click("不保存退出"));
    } else if case == "save-failure" {
        app::add_timeout3(0.3, move |_| {
            click("设置");
            recording(true);
        });
        let p1 = p.clone();
        app::add_timeout3(0.9, move |_| {
            assert!(Settings::load(&p1).unwrap().history_recording_enabled);
            std::fs::rename(&p1, p1.with_extension("backup")).unwrap();
            std::fs::create_dir(&p1).unwrap();
            recording(false);
        });
        let p2 = p.clone();
        app::add_timeout3(1.6, move |_| {
            assert!(p2.is_dir());
            assert!(has_text("下次启动仍可能开启"));
            std::fs::remove_dir(&p2).unwrap();
            std::fs::rename(p2.with_extension("backup"), &p2).unwrap();
            click("返回");
            click("退出");
            checked.set(true);
        });
    } else if case == "open-failure" {
        app::add_timeout3(1.0, move |_| {
            assert!(has_text("历史记录：错误"));
            click("退出");
            checked.set(true);
        });
    } else if case == "invalid" {
        app::add_timeout3(0.3, move |_| {
            click("设置");
            recording(true);
        });
        app::add_timeout3(0.9, move |_| {
            assert_eq!(std::fs::read(&p).unwrap(), initial);
            click("返回");
            click("退出");
            checked.set(true);
        });
    } else {
        let hidden = std::env::var("CFC_GUI_HIDDEN").as_deref() == Ok("true");
        let handle = Rc::new(Cell::new(0usize));
        let shown_handle = handle.clone();
        app::add_timeout3(0.4, move |_| {
            let mut window = ui::acceptance::windows()
                .into_iter()
                .find(|w| w.label().starts_with("ClevoFanControl ·"))
                .unwrap();
            if hidden {
                shown_handle.set(window.raw_handle() as usize);
                window.do_callback();
            } else {
                window.hide();
                window.show();
                shown_handle.set(window.raw_handle() as usize);
            }
        });
        let root_visibility = root.clone();
        app::add_timeout3(0.9, move |_| {
            let window = ui::acceptance::windows()
                .into_iter()
                .find(|w| w.label().starts_with("ClevoFanControl ·"))
                .unwrap();
            let native = unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible(handle.get() as _)
            } != 0;
            assert_eq!(window.shown(), !hidden);
            assert_eq!(native, !hidden);
            std::fs::write(
                root_visibility.join("visibility.json"),
                serde_json::to_vec(
                    &serde_json::json!({"fltk_shown":window.shown(),"native_visible":native}),
                )
                .unwrap(),
            )
            .unwrap();
        });
        let seconds = std::env::var("CFC_GUI_SECONDS")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(62.0);
        let exit_requested = exit_at.clone();
        app::add_timeout3(seconds, move |_| {
            exit_requested.set(Some(Instant::now()));
            click("退出");
            checked.set(true);
        });
    }
    app::add_timeout3(0.6, move |_| {
        for mut window in app::windows().unwrap_or_default() {
            if window.label().starts_with("设置") {
                window.show();
                app::check();
                if let Ok(image) = fltk::draw::capture_window(&mut window) {
                    let mut bytes =
                        format!("P6\n{} {}\n255\n", image.data_w(), image.data_h()).into_bytes();
                    bytes.extend_from_slice(&image.to_rgb_data());
                    std::fs::write(r.join("settings.ppm"), bytes).unwrap();
                }
                window.hide();
            }
        }
    });
    ui::run(settings, path, worker, load_error, true, false);
    assert!(finished.get());
    if let Some(at) = exit_at.get() {
        std::fs::write(
            root.join("shutdown.json"),
            serde_json::to_vec(
                &serde_json::json!({"request_to_return_ms":at.elapsed().as_millis()}),
            )
            .unwrap(),
        )
        .unwrap();
    }
    std::thread::sleep(Duration::from_millis(20));
}
