//! A small modal form built from a description: groups of checkboxes, drop-down lists,
//! text fields, radio buttons and buttons, laid out top to bottom, with OK, Cancel and
//! Apply. Native controls, so screen readers and keyboard navigation work as in any
//! Windows dialog: Tab moves between controls, Enter is OK, Escape is Cancel.

use std::collections::BTreeMap;
use std::io;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{CreateFontIndirectW, DeleteObject, COLOR_BTNFACE, HFONT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, SystemParametersInfoForDpi};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, IsDialogMessageW, LoadCursorW,
    RegisterClassW, SendMessageW, SetForegroundWindow, SetWindowLongPtrW, ShowWindow, TranslateMessage,
    BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_AUTORADIOBUTTON, BS_DEFPUSHBUTTON, BS_GROUPBOX,
    BS_PUSHBUTTON, CBS_DROPDOWNLIST, CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CREATESTRUCTW, CW_USEDEFAULT,
    ES_AUTOHSCROLL, GWLP_USERDATA, HMENU, IDCANCEL, IDC_ARROW, IDOK, MSG, NONCLIENTMETRICSW, SM_CXSCREEN,
    SM_CYSCREEN, SPI_GETNONCLIENTMETRICS, SW_SHOW, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_SETFONT, WNDCLASSW,
    WS_BORDER, WS_CAPTION, WS_CHILD, WS_DISABLED, WS_EX_CONTROLPARENT, WS_EX_DLGMODALFRAME, WS_GROUP,
    WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

/// One control. `id`s name the values `show` hands back, and the buttons clicked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    /// A labelled frame around other controls.
    Group {
        label: String,
        children: Vec<Control>,
    },
    /// Text, one line per `\n`.
    Label(String),
    Check {
        id: &'static str,
        label: String,
        checked: bool,
        enabled: bool,
    },
    /// A drop-down list, with a label on its left.
    Choice {
        id: &'static str,
        label: String,
        options: Vec<String>,
        selected: usize,
        enabled: bool,
    },
    /// A one-line text field, with a label on its left.
    Text {
        id: &'static str,
        label: String,
        value: String,
        enabled: bool,
    },
    /// Radio buttons, one per line.
    Radio {
        id: &'static str,
        options: Vec<String>,
        selected: usize,
        enabled: bool,
    },
    /// Push buttons side by side. Clicking one calls the button handler with its id.
    Buttons(Vec<(&'static str, String)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Index(usize),
    Text(String),
}

pub type Values = BTreeMap<&'static str, Value>;

pub struct Form {
    pub title: String,
    pub controls: Vec<Control>,
}

/// What the form calls while it is open.
pub trait Handler {
    /// OK or Apply: take the values. An error is shown and the form stays open.
    fn apply(&mut self, values: &Values) -> Result<(), String>;
    /// A button from `Control::Buttons`.
    fn button(&mut self, id: &'static str);
}

// ---------------------------------------------------------------------------------------
// Layout, in pixels at 96 DPI. Pure, so it is tested without windows.

const WIDTH: i32 = 560;
const MARGIN: i32 = 12;
const GAP: i32 = 8;
const LINE: i32 = 22;
const LABEL_LINE: i32 = 16;
const FIELD: i32 = 24;
const LABEL_COLUMN: i32 = 170;
const GROUP_TOP: i32 = 22;
const GROUP_PAD: i32 = 10;
const BUTTON_W: i32 = 88;
const BUTTON_H: i32 = 26;
/// Drop-down lists open this much taller than their field.
const LIST_DROP: i32 = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Kind<'a> {
    GroupBox(&'a str),
    Static(&'a str),
    Check(&'static str, &'a str, bool, bool),
    ChoiceBox(&'static str, &'a [String], usize, bool),
    Edit(&'static str, &'a str, bool),
    /// A radio button: its group's id, the option index, its text, selected, enabled.
    RadioButton(&'static str, usize, &'a str, bool, bool),
    Button(&'static str, &'a str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placed<'a> {
    pub kind: Kind<'a>,
    pub rect: Rect,
}

/// Where every control goes, and the height of the whole form above the OK row.
pub(crate) fn layout(controls: &[Control]) -> (Vec<Placed<'_>>, i32) {
    let mut placed = Vec::new();
    let bottom = stack(controls, MARGIN, WIDTH - 2 * MARGIN, MARGIN, &mut placed);
    (placed, bottom)
}

/// Lays `controls` out from `y` down, in a column at `x` of `width`; returns the next `y`.
fn stack<'a>(controls: &'a [Control], x: i32, width: i32, mut y: i32, out: &mut Vec<Placed<'a>>) -> i32 {
    for (i, control) in controls.iter().enumerate() {
        if i > 0 {
            y += GAP;
        }
        let at = |x, y, w, h| Rect { x, y, w, h };
        match control {
            Control::Group { label, children } => {
                let index = out.len();
                out.push(Placed { kind: Kind::GroupBox(label), rect: at(x, y, width, 0) });
                let inner = stack(children, x + GROUP_PAD, width - 2 * GROUP_PAD, y + GROUP_TOP, out);
                let height = inner - y + GROUP_PAD;
                out[index].rect.h = height;
                y += height;
            }
            Control::Label(text) => {
                let lines = text.lines().count().max(1) as i32;
                out.push(Placed { kind: Kind::Static(text), rect: at(x, y, width, lines * LABEL_LINE) });
                y += lines * LABEL_LINE;
            }
            Control::Check { id, label, checked, enabled } => {
                out.push(Placed {
                    kind: Kind::Check(id, label, *checked, *enabled),
                    rect: at(x, y, width, LINE),
                });
                y += LINE;
            }
            Control::Choice { id, label, options, selected, enabled } => {
                out.push(Placed { kind: Kind::Static(label), rect: at(x, y + 4, LABEL_COLUMN, LABEL_LINE) });
                let rect = at(x + LABEL_COLUMN, y, width - LABEL_COLUMN, FIELD);
                out.push(Placed { kind: Kind::ChoiceBox(id, options, *selected, *enabled), rect });
                y += FIELD;
            }
            Control::Text { id, label, value, enabled } => {
                out.push(Placed { kind: Kind::Static(label), rect: at(x, y + 4, LABEL_COLUMN, LABEL_LINE) });
                let rect = at(x + LABEL_COLUMN, y, width - LABEL_COLUMN, FIELD);
                out.push(Placed { kind: Kind::Edit(id, value, *enabled), rect });
                y += FIELD;
            }
            Control::Radio { id, options, selected, enabled } => {
                for (index, option) in options.iter().enumerate() {
                    let kind = Kind::RadioButton(id, index, option, index == *selected, *enabled);
                    out.push(Placed { kind, rect: at(x, y, width, LINE) });
                    y += LINE;
                }
            }
            Control::Buttons(buttons) => {
                let mut bx = x;
                for (id, label) in buttons {
                    let w = button_width(label);
                    out.push(Placed { kind: Kind::Button(id, label), rect: at(bx, y, w, BUTTON_H) });
                    bx += w + GAP;
                }
                y += BUTTON_H;
            }
        }
    }
    y
}

fn button_width(label: &str) -> i32 {
    (label.chars().count() as i32 * 7 + 24).max(BUTTON_W)
}

/// OK, Cancel and Apply, right-aligned under the form.
fn footer(top: i32) -> [(i32, &'static str, Rect); 3] {
    let y = top + GAP * 2;
    let right = WIDTH - MARGIN;
    let at = |i: i32| Rect { x: right - (3 - i) * BUTTON_W - (2 - i) * GAP, y, w: BUTTON_W, h: BUTTON_H };
    [(IDOK, "OK", at(0)), (IDCANCEL, "Cancel", at(1)), (ID_APPLY, "Apply", at(2))]
}

const ID_APPLY: i32 = 3;
/// Child control ids start here; lower ones are OK, Cancel and Apply.
const FIRST_ID: i32 = 100;

// ---------------------------------------------------------------------------------------
// Windows.

/// What a click asked for, read by the modal loop after the window procedure returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Ok,
    Cancel,
    Apply,
    Button(usize),
}

struct State {
    command: Option<Command>,
}

/// A created control and how to read it back.
enum Field {
    Check(&'static str, HWND),
    Choice(&'static str, HWND),
    Edit(&'static str, HWND),
    Radio(&'static str, usize, HWND),
    Button(&'static str),
    Other,
}

/// Shows the form and runs it until OK or Cancel. Returns true if OK applied the values.
/// The calling thread's other windows keep working meanwhile.
pub fn show(form: &Form, handler: &mut dyn Handler) -> io::Result<bool> {
    let (placed, bottom) = layout(&form.controls);
    let buttons = footer(bottom);
    let client_height = buttons[0].2.y + BUTTON_H + MARGIN;
    // SAFETY: GetDpiForSystem has no preconditions.
    let dpi = unsafe { GetDpiForSystem() } as i32;
    let scale = |v: i32| v * dpi / 96;

    let class = wide("unicompose-form");
    let title = wide(&form.title);
    let mut state = State { command: None };
    // SAFETY: the class and window use this module's instance; `state` outlives the window,
    // which is destroyed before this function returns.
    let window = unsafe {
        let instance = GetModuleHandleW(null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(form_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_BTNFACE + 1) as usize as _,
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc); // Fails harmlessly once registered.
        let style = WS_CAPTION | WS_SYSMENU;
        let ex_style = WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT;
        let mut rect = RECT { left: 0, top: 0, right: scale(WIDTH), bottom: scale(client_height) };
        AdjustWindowRectEx(&mut rect, style, 0, ex_style);
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        let (x, y) = centred(w, h);
        CreateWindowExW(
            ex_style,
            class.as_ptr(),
            title.as_ptr(),
            style,
            x,
            y,
            w,
            h,
            null_mut(),
            null_mut(),
            instance,
            (&mut state as *mut State).cast(),
        )
    };
    if window.is_null() {
        return Err(io::Error::last_os_error());
    }
    let font = message_font(dpi);
    let mut fields = Vec::new();
    for (index, item) in placed.iter().enumerate() {
        let id = FIRST_ID + index as i32;
        let r = Rect {
            x: scale(item.rect.x),
            y: scale(item.rect.y),
            w: scale(item.rect.w),
            h: scale(item.rect.h),
        };
        fields.push(create_control(window, id, &item.kind, r, font, scale(LIST_DROP)));
    }
    for (id, label, rect) in buttons {
        let r = Rect { x: scale(rect.x), y: scale(rect.y), w: scale(rect.w), h: scale(rect.h) };
        let style = if id == IDOK { BS_DEFPUSHBUTTON } else { BS_PUSHBUTTON };
        child(window, id, "BUTTON", label, (style as u32) | WS_TABSTOP, r, font);
    }
    // SAFETY: `window` is ours and alive.
    unsafe {
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
        if let Some(first) = fields.iter().find_map(|f| match f {
            Field::Check(_, h) | Field::Choice(_, h) | Field::Edit(_, h) | Field::Radio(_, _, h) => Some(*h),
            _ => None,
        }) {
            SetFocus(first);
        }
    }

    let mut applied = false;
    // SAFETY: MSG is plain data.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    let result = loop {
        // SAFETY: `msg` is a valid MSG to write into; 0 means WM_QUIT and -1 an error.
        let got = unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) };
        if got <= 0 {
            break Ok(applied);
        }
        // SAFETY: `msg` was just filled in; IsDialogMessage handles Tab, Enter and Esc.
        unsafe {
            if IsDialogMessageW(window, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let Some(command) = state.command.take() else { continue };
        match command {
            Command::Cancel => break Ok(applied),
            Command::Ok | Command::Apply => match handler.apply(&read_values(&fields)) {
                Ok(()) if command == Command::Ok => break Ok(true),
                Ok(()) => applied = true,
                Err(e) => crate::app::message_box(&form.title, &e, true),
            },
            Command::Button(index) => {
                if let Some(Field::Button(id)) = fields.get(index) {
                    handler.button(id);
                }
            }
        }
    };
    // SAFETY: the window and font are ours and released once.
    unsafe {
        DestroyWindow(window);
        DeleteObject(font as _);
    }
    result
}

fn read_values(fields: &[Field]) -> Values {
    let mut values = Values::new();
    for field in fields {
        // SAFETY: each handle is a live child control of the form, of the class its
        // message expects.
        unsafe {
            match *field {
                Field::Check(id, h) => {
                    values.insert(id, Value::Bool(SendMessageW(h, BM_GETCHECK, 0, 0) == 1));
                }
                Field::Choice(id, h) => {
                    let index = SendMessageW(h, CB_GETCURSEL, 0, 0);
                    values.insert(id, Value::Index(usize::try_from(index).unwrap_or(0)));
                }
                Field::Edit(id, h) => {
                    let len = GetWindowTextLengthW(h).max(0) as usize;
                    let mut buf = vec![0u16; len + 1];
                    let n = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
                    values.insert(id, Value::Text(String::from_utf16_lossy(&buf[..n])));
                }
                Field::Radio(id, index, h) => {
                    if SendMessageW(h, BM_GETCHECK, 0, 0) == 1 {
                        values.insert(id, Value::Index(index));
                    }
                }
                Field::Button(_) | Field::Other => {}
            }
        }
    }
    values
}

fn create_control(window: HWND, id: i32, kind: &Kind, r: Rect, font: HFONT, drop: i32) -> Field {
    let tab = WS_TABSTOP;
    let disabled = |enabled: bool| if enabled { 0 } else { WS_DISABLED };
    match *kind {
        Kind::GroupBox(label) => {
            child(window, id, "BUTTON", label, BS_GROUPBOX as u32, r, font);
            Field::Other
        }
        Kind::Static(text) => {
            child(window, id, "STATIC", text, 0, r, font);
            Field::Other
        }
        Kind::Check(field, label, checked, enabled) => {
            let h =
                child(window, id, "BUTTON", label, BS_AUTOCHECKBOX as u32 | tab | disabled(enabled), r, font);
            // SAFETY: `h` is a checkbox we just created.
            unsafe { SendMessageW(h, BM_SETCHECK, usize::from(checked), 0) };
            Field::Check(field, h)
        }
        Kind::ChoiceBox(field, options, selected, enabled) => {
            let r = Rect { h: r.h + drop, ..r };
            let style = CBS_DROPDOWNLIST as u32 | WS_VSCROLL | tab | disabled(enabled);
            let h = child(window, id, "COMBOBOX", "", style, r, font);
            for option in options {
                let text = wide(option);
                // SAFETY: `h` is a combo box; the string outlives the call.
                unsafe { SendMessageW(h, CB_ADDSTRING, 0, text.as_ptr() as LPARAM) };
            }
            // SAFETY: as above.
            unsafe { SendMessageW(h, CB_SETCURSEL, selected, 0) };
            Field::Choice(field, h)
        }
        Kind::Edit(field, value, enabled) => {
            let style = ES_AUTOHSCROLL as u32 | WS_BORDER | tab | disabled(enabled);
            Field::Edit(field, child(window, id, "EDIT", value, style, r, font))
        }
        Kind::RadioButton(field, index, label, selected, enabled) => {
            // The first option starts a group, so arrow keys move within it.
            let group = if index == 0 { WS_GROUP | tab } else { 0 };
            let style = BS_AUTORADIOBUTTON as u32 | group | disabled(enabled);
            let h = child(window, id, "BUTTON", label, style, r, font);
            // SAFETY: `h` is a radio button we just created.
            unsafe { SendMessageW(h, BM_SETCHECK, usize::from(selected), 0) };
            Field::Radio(field, index, h)
        }
        Kind::Button(field, label) => {
            child(window, id, "BUTTON", label, BS_PUSHBUTTON as u32 | tab, r, font);
            Field::Button(field)
        }
    }
}

fn child(window: HWND, id: i32, class: &str, text: &str, style: u32, r: Rect, font: HFONT) -> HWND {
    let class = wide(class);
    let text = wide(text);
    // SAFETY: `window` is live; the strings outlive the call; the id is passed as the
    // menu handle, as child windows take it.
    unsafe {
        let h = CreateWindowExW(
            0,
            class.as_ptr(),
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            r.x,
            r.y,
            r.w,
            r.h,
            window,
            id as isize as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        SendMessageW(h, WM_SETFONT, font as WPARAM, 1);
        if style & WS_DISABLED != 0 {
            EnableWindow(h, 0);
        }
        h
    }
}

/// The font Windows uses for message boxes, at `dpi`.
fn message_font(dpi: i32) -> HFONT {
    // SAFETY: NONCLIENTMETRICSW is plain data with its size set, as the call requires.
    unsafe {
        let mut metrics: NONCLIENTMETRICSW = std::mem::zeroed();
        metrics.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
        SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&mut metrics as *mut NONCLIENTMETRICSW).cast(),
            0,
            dpi as u32,
        );
        CreateFontIndirectW(&metrics.lfMessageFont)
    }
}

fn centred(w: i32, h: i32) -> (i32, i32) {
    // SAFETY: GetSystemMetrics has no preconditions.
    let (sw, sh) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    if sw <= 0 {
        return (CW_USEDEFAULT, CW_USEDEFAULT);
    }
    ((sw - w).max(0) / 2, (sh - h).max(0) / 2)
}

unsafe extern "system" fn form_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: Windows calls this with valid arguments for `msg`; GWLP_USERDATA holds the
    // `State` passed at creation, which outlives the window.
    unsafe {
        match msg {
            WM_CREATE => {
                let create = &*(lparam as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
                0
            }
            WM_COMMAND | WM_CLOSE => {
                let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
                if let Some(state) = state.as_mut() {
                    let id = (wparam & 0xFFFF) as i32;
                    state.command = match (msg, id) {
                        (WM_CLOSE, _) | (_, IDCANCEL) => Some(Command::Cancel),
                        (_, IDOK) => Some(Command::Ok),
                        (_, ID_APPLY) => Some(Command::Apply),
                        // Only buttons send a plain click with a zero notification code;
                        // the loop ignores ids that are not buttons.
                        (_, id) if id >= FIRST_ID && (wparam >> 16) == 0 => {
                            Some(Command::Button((id - FIRST_ID) as usize))
                        }
                        _ => state.command,
                    };
                }
                if msg == WM_CLOSE {
                    0
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Control> {
        vec![
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
                    Control::Buttons(vec![
                        ("edit", "Edit my rules".into()),
                        ("reload", "Reload rules".into()),
                    ]),
                ],
            },
            Control::Label("Two\nlines".into()),
            Control::Radio {
                id: "login",
                options: vec!["Off".into(), "On".into()],
                selected: 0,
                enabled: true,
            },
            Control::Text { id: "vid", label: "Vendor ID".into(), value: "1209".into(), enabled: false },
        ]
    }

    fn inside(inner: Rect, outer: Rect) -> bool {
        inner.x >= outer.x
            && inner.y >= outer.y
            && inner.x + inner.w <= outer.x + outer.w
            && inner.y + inner.h <= outer.y + outer.h
    }

    #[test]
    fn everything_fits_the_width_in_order_without_overlap() {
        let controls = sample();
        let (placed, bottom) = layout(&controls);
        let form = Rect { x: 0, y: 0, w: WIDTH, h: bottom + MARGIN };
        for item in &placed {
            assert!(inside(item.rect, form), "{item:?}");
            assert!(item.rect.h > 0 && item.rect.w > 0, "{item:?}");
        }
        let group = placed[0].rect;
        for item in &placed[1..6] {
            assert!(inside(item.rect, group), "group child {item:?} outside {group:?}");
        }
        assert!(placed[6].rect.y >= group.y + group.h, "the next control starts below the group");
        // No two controls overlap, apart from a group and what it holds.
        let controls: Vec<&Placed> = placed.iter().filter(|p| !matches!(p.kind, Kind::GroupBox(_))).collect();
        for (i, a) in controls.iter().enumerate() {
            for b in &controls[i + 1..] {
                let apart = a.rect.x + a.rect.w <= b.rect.x
                    || b.rect.x + b.rect.w <= a.rect.x
                    || a.rect.y + a.rect.h <= b.rect.y
                    || b.rect.y + b.rect.h <= a.rect.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn labels_take_a_line_each_and_radios_one_row_each() {
        let controls = [
            Control::Label("a\nb\nc".into()),
            Control::Radio { id: "r", options: vec!["x".into(), "y".into()], selected: 1, enabled: true },
        ];
        let (placed, _) = layout(&controls);
        assert_eq!(placed[0].rect.h, 3 * LABEL_LINE);
        assert!(matches!(placed[2].kind, Kind::RadioButton("r", 1, "y", true, true)));
        assert_eq!(placed[2].rect.y - placed[1].rect.y, LINE);
    }

    #[test]
    fn footer_buttons_are_right_aligned_below_the_form() {
        let buttons = footer(200);
        assert_eq!(buttons[2].2.x + BUTTON_W, WIDTH - MARGIN);
        assert!(buttons.iter().all(|(_, _, r)| r.y > 200));
    }
}
