//! The Settings window: `config.toml` as a form, and back.

use uc_app::device::PROFILES;
use uc_app::settings::Sections;
use uc_config::Config;
use uc_win::compose::COMPOSE_KEYS;
use uc_win::dialog::{Control, Form, Value, Values};

/// How the app starts at login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Login {
    Off,
    Normal,
    Elevated,
}

impl Login {
    const ALL: [Login; 3] = [Login::Off, Login::Normal, Login::Elevated];
}

/// What the form shows beyond the settings themselves.
pub struct Context<'a> {
    /// Sections command-line flags override for this run.
    pub overridden: Sections,
    /// Run from an MSIX package, where Windows manages starting at login.
    pub packaged: bool,
    pub login: Login,
    /// A line on the keyboard's state, such as "Connected: Mathpad (1209:2211)".
    pub keyboard_status: &'a str,
    pub rules_file: &'a str,
}

pub const EDIT_RULES: &str = "edit_rules";
pub const RELOAD_RULES: &str = "reload_rules";

pub fn form(config: &Config, context: &Context) -> Form {
    let Sections { compose: compose_fixed, keyboard: keyboard_fixed, workarounds: workarounds_fixed } =
        context.overridden;
    let compose = &config.compose;
    let key_index = COMPOSE_KEYS.iter().position(|k| k.name() == compose.key).unwrap_or(0);
    let check = |id, label: &str, checked, fixed: bool| Control::Check {
        id,
        label: label.into(),
        checked,
        enabled: !fixed,
    };
    let mut controls = vec![Control::Group {
        label: "Compose key".into(),
        children: vec![
            check("compose.enabled", "Use a Compose key", compose.enabled, compose_fixed),
            Control::Choice {
                id: "compose.key",
                label: "Key".into(),
                options: COMPOSE_KEYS.iter().map(|k| k.label().to_owned()).collect(),
                selected: key_index,
                enabled: !compose_fixed,
            },
            check(
                "compose.wincompose_rules",
                "Include WinCompose's emoji and extra sequences",
                compose.wincompose_rules,
                compose_fixed,
            ),
            check(
                "compose.hex_entry",
                "Type any character with u, its hex code and Enter",
                compose.hex_entry,
                compose_fixed,
            ),
            check(
                "compose.type_invalid",
                "When a sequence matches nothing, type the keys pressed",
                !compose.discard_invalid,
                compose_fixed,
            ),
            Control::Label(format!("Your own sequences: {}", context.rules_file)),
            Control::Buttons(vec![(EDIT_RULES, "Edit my sequences".into()), (RELOAD_RULES, "Reload".into())]),
        ],
    }];

    let keyboard = &config.keyboard;
    let profile_index = PROFILES.iter().position(|p| p.name == keyboard.profile).unwrap_or(0);
    let text = |id, label: &str, value: &Option<String>| Control::Text {
        id,
        label: label.into(),
        value: value.clone().unwrap_or_default(),
        enabled: !keyboard_fixed,
    };
    controls.push(Control::Group {
        label: "Keyboard that sends Unicode over Raw HID".into(),
        children: vec![
            Control::Choice {
                id: "keyboard.profile",
                label: "Profile".into(),
                options: PROFILES.iter().map(|p| p.name.to_owned()).collect(),
                selected: profile_index,
                enabled: !keyboard_fixed,
            },
            text("keyboard.vid", "Vendor ID (hex, optional)", &keyboard.vid),
            text("keyboard.pid", "Product ID (hex, optional)", &keyboard.pid),
            text("keyboard.usage_page", "Usage page (hex, optional)", &keyboard.usage_page),
            text("keyboard.usage", "Usage (hex, optional)", &keyboard.usage),
            Control::Label(context.keyboard_status.to_owned()),
        ],
    });

    let workarounds = &config.workarounds;
    controls.push(Control::Group {
        label: "Workarounds for some applications".into(),
        children: vec![
            check(
                "workarounds.gtk_astral",
                "GIMP, Inkscape and other GTK apps: type \u{1D538} and the like safely",
                workarounds.gtk_astral,
                workarounds_fixed,
            ),
            check(
                "workarounds.office_font",
                "Word and Outlook: keep the font after a symbol",
                workarounds.office_font,
                workarounds_fixed,
            ),
        ],
    });

    let login = if context.packaged {
        Control::Label("Windows manages this: Settings > Apps > Startup.".into())
    } else {
        Control::Radio {
            id: "login",
            options: vec![
                "Don't start at login".into(),
                "Start at login".into(),
                "Start at login as administrator, to work in admin windows too".into(),
            ],
            selected: Login::ALL.iter().position(|&l| l == context.login).unwrap_or(0),
            enabled: true,
        }
    };
    controls.push(Control::Group { label: "Start at login".into(), children: vec![login] });

    let fixed: Vec<&str> =
        [(compose_fixed, "Compose key"), (keyboard_fixed, "keyboard"), (workarounds_fixed, "workarounds")]
            .into_iter()
            .filter_map(|(on, name)| on.then_some(name))
            .collect();
    if !fixed.is_empty() {
        controls.push(Control::Label(format!(
            "Command-line flags set the {} settings for this run; they are greyed out.",
            fixed.join(" and ")
        )));
    }
    Form { title: "unicompose settings".into(), controls }
}

/// The settings the form's values describe, starting from `saved` so anything the form
/// does not show is kept. Fails with a message naming a field that is not valid.
pub fn read(values: &Values, saved: &Config) -> Result<(Config, Option<Login>), String> {
    let mut config = saved.clone();
    let flag = |id: &str| match values.get(id) {
        Some(Value::Bool(b)) => Some(*b),
        _ => None,
    };
    let index = |id: &str| match values.get(id) {
        Some(Value::Index(i)) => Some(*i),
        _ => None,
    };
    let text = |id: &str| match values.get(id) {
        Some(Value::Text(t)) => Some(t.trim().to_owned()),
        _ => None,
    };
    let compose = &mut config.compose;
    compose.enabled = flag("compose.enabled").unwrap_or(compose.enabled);
    if let Some(key) = index("compose.key").and_then(|i| COMPOSE_KEYS.get(i)) {
        compose.key = key.name().to_owned();
    }
    compose.wincompose_rules = flag("compose.wincompose_rules").unwrap_or(compose.wincompose_rules);
    compose.hex_entry = flag("compose.hex_entry").unwrap_or(compose.hex_entry);
    compose.discard_invalid = flag("compose.type_invalid").map_or(compose.discard_invalid, |t| !t);

    let keyboard = &mut config.keyboard;
    if let Some(profile) = index("keyboard.profile").and_then(|i| PROFILES.get(i)) {
        keyboard.profile = profile.name.to_owned();
    }
    for (id, field) in [
        ("keyboard.vid", &mut keyboard.vid),
        ("keyboard.pid", &mut keyboard.pid),
        ("keyboard.usage_page", &mut keyboard.usage_page),
        ("keyboard.usage", &mut keyboard.usage),
    ] {
        if let Some(value) = text(id) {
            *field = (!value.is_empty()).then_some(value);
        }
    }
    uc_app::device::resolve(keyboard)?;

    let workarounds = &mut config.workarounds;
    workarounds.gtk_astral = flag("workarounds.gtk_astral").unwrap_or(workarounds.gtk_astral);
    workarounds.office_font = flag("workarounds.office_font").unwrap_or(workarounds.office_font);

    let login = index("login").and_then(|i| Login::ALL.get(i).copied());
    Ok((config, login))
}

/// What `Edit my sequences` creates when the file does not exist yet.
pub const RULES_TEMPLATE: &str = "\
# Your own Compose sequences. unicompose reads this file after its built-in ones, so a
# sequence here replaces a built-in one with the same keys. The format is XCompose:
#
#   <Multi_key> <key> <key> : \"text\"
#
# Keys are single characters such as <a> <!> <1>, or names such as <space>, <minus>,
# <quotedbl>, <Up>. After saving, use Reload in the settings or the tray menu.

# <Multi_key> <h> <w> : \"Hello, world!\"
";

#[cfg(test)]
mod tests {
    use super::*;
    use uc_win::dialog::Control;

    fn context() -> Context<'static> {
        Context {
            overridden: Sections::default(),
            packaged: false,
            login: Login::Normal,
            keyboard_status: "Waiting for mathpad (1209:2211)",
            rules_file: "C:\\Users\\me\\.XCompose",
        }
    }

    /// The values the form shows, as the dialog would hand them back unchanged.
    fn values_of(form: &Form) -> Values {
        fn walk(controls: &[Control], values: &mut Values) {
            for control in controls {
                match control {
                    Control::Group { children, .. } => walk(children, values),
                    Control::Check { id, checked, .. } => {
                        values.insert(id, Value::Bool(*checked));
                    }
                    Control::Choice { id, selected, .. } | Control::Radio { id, selected, .. } => {
                        values.insert(id, Value::Index(*selected));
                    }
                    Control::Text { id, value, .. } => {
                        values.insert(id, Value::Text(value.clone()));
                    }
                    Control::Label(_) | Control::Buttons(_) => {}
                }
            }
        }
        let mut values = Values::new();
        walk(&form.controls, &mut values);
        values
    }

    #[test]
    fn unchanged_form_gives_back_the_same_settings() {
        let mut config = Config::default();
        config.compose.key = "capslock".into();
        config.compose.discard_invalid = true;
        config.keyboard.vid = Some("1d50".into());
        config.workarounds.office_font = true;
        let form = form(&config, &context());
        let (read_back, login) = read(&values_of(&form), &config).unwrap();
        assert_eq!(read_back, config);
        assert_eq!(login, Some(Login::Normal));
    }

    #[test]
    fn edits_are_read_and_bad_ids_are_refused() {
        let config = Config::default();
        let mut values = values_of(&form(&config, &context()));
        values.insert("compose.key", Value::Index(7)); // Caps Lock
        values.insert("keyboard.pid", Value::Text(" ".into()));
        values.insert("login", Value::Index(2));
        let (changed, login) = read(&values, &config).unwrap();
        assert_eq!(changed.compose.key, COMPOSE_KEYS[7].name());
        assert_eq!(changed.keyboard.pid, None, "blank means the profile's own");
        assert_eq!(login, Some(Login::Elevated));

        values.insert("keyboard.vid", Value::Text("xyz".into()));
        assert!(read(&values, &config).unwrap_err().contains("vid"));
    }

    #[test]
    fn overridden_sections_are_greyed_and_packaged_apps_have_no_login_choice() {
        let context = Context {
            overridden: Sections { compose: true, ..Sections::default() },
            packaged: true,
            ..context()
        };
        let form = form(&Config::default(), &context);
        let Control::Group { children, .. } = &form.controls[0] else { panic!("compose group first") };
        assert!(matches!(children[0], Control::Check { enabled: false, .. }));
        let values = values_of(&form);
        assert!(!values.contains_key("login"));
        assert!(matches!(form.controls.last(), Some(Control::Label(note)) if note.contains("Compose key")));
    }
}
