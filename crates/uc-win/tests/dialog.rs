//! Opens a real form and presses its buttons by message, so it needs no keyboard focus.
#![cfg(windows)]

use std::time::{Duration, Instant};

use uc_win::dialog::{show, Control, Form, Handler, Value, Values};
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, IDCANCEL, IDOK, WM_COMMAND};

struct Recorder {
    applied: Vec<Values>,
    buttons: Vec<&'static str>,
}

impl Handler for Recorder {
    fn apply(&mut self, values: &Values) -> Result<(), String> {
        self.applied.push(values.clone());
        Ok(())
    }

    fn button(&mut self, id: &'static str) {
        self.buttons.push(id);
    }
}

/// Posts WM_COMMAND `ids` to the form titled `title` once it exists, one at a time.
fn click_later(title: &'static str, ids: Vec<i32>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let class: Vec<u16> = "unicompose-form".encode_utf16().chain([0]).collect();
        let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
        let deadline = Instant::now() + Duration::from_secs(5);
        let window = loop {
            // SAFETY: both strings are live and NUL-terminated.
            let window = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
            if !window.is_null() || Instant::now() > deadline {
                break window;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(!window.is_null(), "the form never appeared");
        for id in ids {
            std::thread::sleep(Duration::from_millis(100));
            // SAFETY: posting a plain command message to a live window.
            unsafe { PostMessageW(window, WM_COMMAND, id as usize, 0) };
        }
    })
}

fn form(title: &str) -> Form {
    Form {
        title: title.into(),
        controls: vec![
            Control::Group {
                label: "Compose key".into(),
                children: vec![
                    Control::Check {
                        id: "on",
                        label: "Use a Compose key".into(),
                        checked: true,
                        enabled: true,
                    },
                    Control::Choice {
                        id: "key",
                        label: "Key".into(),
                        options: vec!["Right Alt".into(), "Menu".into()],
                        selected: 1,
                        enabled: true,
                    },
                    Control::Buttons(vec![("edit", "Edit my rules".into())]),
                ],
            },
            Control::Radio {
                id: "login",
                options: vec!["Off".into(), "On".into()],
                selected: 1,
                enabled: true,
            },
            Control::Text { id: "vid", label: "Vendor ID".into(), value: "1209".into(), enabled: true },
        ],
    }
}

fn expected() -> Values {
    let mut values = Values::new();
    values.insert("on", Value::Bool(true));
    values.insert("key", Value::Index(1));
    values.insert("login", Value::Index(1));
    values.insert("vid", Value::Text("1209".into()));
    values
}

#[test]
fn ok_hands_back_the_values_as_shown() {
    let title = "unicompose dialog test: ok";
    let clicker = click_later(title, vec![IDOK]);
    let mut handler = Recorder { applied: Vec::new(), buttons: Vec::new() };
    assert!(show(&form(title), &mut handler).unwrap());
    clicker.join().unwrap();
    assert_eq!(handler.applied, [expected()]);
}

#[test]
fn buttons_report_their_id_and_leave_the_form_open() {
    let title = "unicompose dialog test: button";
    // Control ids start at 100 in layout order: group, check, label, choice, then the button.
    let edit_button = 104;
    let clicker = click_later(title, vec![edit_button, IDCANCEL]);
    let mut handler = Recorder { applied: Vec::new(), buttons: Vec::new() };
    assert!(!show(&form(title), &mut handler).unwrap(), "Cancel applies nothing");
    clicker.join().unwrap();
    assert_eq!(handler.buttons, ["edit"]);
    assert!(handler.applied.is_empty());
}
