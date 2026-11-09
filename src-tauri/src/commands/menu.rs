//! The macOS menu bar. The WebView owns the menu structure (from its actions table) and sends it
//! with the labels in the current language; choosing an item emits [`MENU_ACTION_EVENT`] with
//! the action id. Other platforms draw their menus in the WebView, so the command only validates.

use serde::Deserialize;
use tauri::{AppHandle, Emitter};

use crate::error::CommandError;

/// Event carrying the action id of a chosen menu item.
pub const MENU_ACTION_EVENT: &str = "menu-action";
/// Prefix of menu item ids that name an action.
const ACTION_PREFIX: &str = "action:";

const MAX_MENUS: usize = 12;
const MAX_ITEMS: usize = 50;

/// One top-level menu.
#[derive(Debug, Deserialize)]
pub struct MenuSpec {
    /// Title shown in the menu bar.
    pub label: String,
    /// Items in order.
    pub items: Vec<MenuItemSpec>,
}

/// An item of a menu.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MenuItemSpec {
    /// Runs the action `id` in the WebView.
    Action {
        /// Action id from the WebView's actions table.
        id: String,
        /// Item text.
        label: String,
    },
    /// A separator line.
    Separator,
}

/// Texts of the system-provided items, in the WebView's language.
#[derive(Debug, Deserialize)]
pub struct NativeLabels {
    pub about: String,
    pub settings: String,
    pub hide: String,
    pub hide_others: String,
    pub show_all: String,
    pub quit: String,
    pub edit: String,
    pub undo: String,
    pub redo: String,
    pub cut: String,
    pub copy: String,
    pub paste: String,
    pub select_all: String,
    pub window: String,
    pub minimize: String,
    pub zoom: String,
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new("input.invalid", message)
}

fn check_label(label: &str) -> Result<(), CommandError> {
    let count = label.chars().count();
    if count == 0 || count > 80 || label.chars().any(char::is_control) {
        return Err(invalid("menu labels must be 1-80 printable characters"));
    }
    Ok(())
}

fn check_id(id: &str) -> Result<(), CommandError> {
    let valid = !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.');
    if valid {
        Ok(())
    } else {
        Err(invalid(
            "menu action ids must be 1-64 letters, digits or dots",
        ))
    }
}

/// Checks the structure sent by the WebView before it reaches the system menu.
fn validate(menus: &[MenuSpec], labels: &NativeLabels) -> Result<(), CommandError> {
    if menus.len() > MAX_MENUS {
        return Err(invalid("too many menus"));
    }
    for menu in menus {
        check_label(&menu.label)?;
        if menu.items.len() > MAX_ITEMS {
            return Err(invalid("too many menu items"));
        }
        for item in &menu.items {
            if let MenuItemSpec::Action { id, label } = item {
                check_id(id)?;
                check_label(label)?;
            }
        }
    }
    for label in [
        &labels.about,
        &labels.settings,
        &labels.hide,
        &labels.hide_others,
        &labels.show_all,
        &labels.quit,
        &labels.edit,
        &labels.undo,
        &labels.redo,
        &labels.cut,
        &labels.copy,
        &labels.paste,
        &labels.select_all,
        &labels.window,
        &labels.minimize,
        &labels.zoom,
    ] {
        check_label(label)?;
    }
    Ok(())
}

/// Installs the menu bar (macOS). Items carry no shortcuts: the WebView's actions table handles
/// keys, so custom bindings never disagree with the menu. The Edit menu uses the system items so
/// text fields keep native copy and paste.
#[tauri::command]
pub fn menu_set(
    app: AppHandle,
    menus: Vec<MenuSpec>,
    labels: NativeLabels,
) -> Result<(), CommandError> {
    validate(&menus, &labels)?;
    #[cfg(target_os = "macos")]
    install(&app, &menus, &labels)
        .map_err(|error| CommandError::new("app.menu", error.to_string()))?;
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    Ok(())
}

#[cfg(target_os = "macos")]
fn install(app: &AppHandle, menus: &[MenuSpec], labels: &NativeLabels) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};

    let action = |id: &str, label: &str| {
        MenuItemBuilder::with_id(format!("{ACTION_PREFIX}{id}"), label).build(app)
    };
    let app_menu = SubmenuBuilder::new(app, "RoveTerm")
        .item(&action("help.about", &labels.about)?)
        .separator()
        .item(&action("settings.open", &labels.settings)?)
        .separator()
        .item(&PredefinedMenuItem::hide(app, Some(&labels.hide))?)
        .item(&PredefinedMenuItem::hide_others(
            app,
            Some(&labels.hide_others),
        )?)
        .item(&PredefinedMenuItem::show_all(app, Some(&labels.show_all))?)
        .separator()
        .item(&PredefinedMenuItem::quit(app, Some(&labels.quit))?)
        .build()?;
    let edit = SubmenuBuilder::new(app, &labels.edit)
        .item(&PredefinedMenuItem::undo(app, Some(&labels.undo))?)
        .item(&PredefinedMenuItem::redo(app, Some(&labels.redo))?)
        .separator()
        .item(&PredefinedMenuItem::cut(app, Some(&labels.cut))?)
        .item(&PredefinedMenuItem::copy(app, Some(&labels.copy))?)
        .item(&PredefinedMenuItem::paste(app, Some(&labels.paste))?)
        .item(&PredefinedMenuItem::select_all(
            app,
            Some(&labels.select_all),
        )?)
        .build()?;
    let mut bar = MenuBuilder::new(app).item(&app_menu).item(&edit);
    for menu in menus {
        let mut submenu = SubmenuBuilder::new(app, &menu.label);
        for item in &menu.items {
            submenu = match item {
                MenuItemSpec::Action { id, label } => submenu.item(&action(id, label)?),
                MenuItemSpec::Separator => submenu.separator(),
            };
        }
        bar = bar.item(&submenu.build()?);
    }
    let window = SubmenuBuilder::new(app, &labels.window)
        .item(&PredefinedMenuItem::minimize(app, Some(&labels.minimize))?)
        .item(&PredefinedMenuItem::maximize(app, Some(&labels.zoom))?)
        .build()?;
    app.set_menu(bar.item(&window).build()?)?;
    Ok(())
}

/// Forwards a chosen action item to the WebView.
pub fn on_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    if let Some(id) = event.id().0.strip_prefix(ACTION_PREFIX)
        && let Err(error) = app.emit(MENU_ACTION_EVENT, id)
    {
        tracing::error!(%error, "forwarding menu action failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> NativeLabels {
        let text = || "x".to_owned();
        NativeLabels {
            about: text(),
            settings: text(),
            hide: text(),
            hide_others: text(),
            show_all: text(),
            quit: text(),
            edit: text(),
            undo: text(),
            redo: text(),
            cut: text(),
            copy: text(),
            paste: text(),
            select_all: text(),
            window: text(),
            minimize: text(),
            zoom: text(),
        }
    }

    fn menu(id: &str, label: &str) -> MenuSpec {
        MenuSpec {
            label: "File".into(),
            items: vec![
                MenuItemSpec::Action {
                    id: id.into(),
                    label: label.into(),
                },
                MenuItemSpec::Separator,
            ],
        }
    }

    #[test]
    fn accepts_action_menus_and_rejects_bad_input() {
        assert!(validate(&[menu("tab.newLocal", "New terminal")], &labels()).is_ok());
        assert!(validate(&[menu("", "Empty id")], &labels()).is_err());
        assert!(validate(&[menu("evil:id", "Colon")], &labels()).is_err());
        assert!(validate(&[menu("tab.close", "")], &labels()).is_err());
        assert!(validate(&[menu("tab.close", "line\nbreak")], &labels()).is_err());
        let many: Vec<MenuSpec> = (0..13).map(|_| menu("a", "b")).collect();
        assert!(validate(&many, &labels()).is_err());
        let mut bad = labels();
        bad.quit = String::new();
        assert!(validate(&[], &bad).is_err());
    }

    #[test]
    fn parses_the_webview_shape() {
        let spec: MenuSpec = serde_json::from_str(
            r#"{"label":"View","items":[{"kind":"action","id":"panel.right","label":"Right"},{"kind":"separator"}]}"#,
        )
        .unwrap();
        assert!(matches!(&spec.items[0], MenuItemSpec::Action { id, .. } if id == "panel.right"));
        assert!(matches!(spec.items[1], MenuItemSpec::Separator));
    }
}
