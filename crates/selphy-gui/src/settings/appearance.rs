//! The Appearance pane: System, Light or Dark, applied at once and saved to
//! `gui.toml`.

use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage};
use gpui_kit::{Entity, SharedString};

use crate::state::{Prefs, ThemeChoice};

/// The Appearance page of the Settings window. A theme that cannot be saved
/// still applies; the file keeps the old one.
pub fn page(prefs: &Entity<Prefs>) -> SettingPage {
    let options = ThemeChoice::ALL
        .map(|theme| {
            let label = SharedString::from(theme.label());
            (label.clone(), label)
        })
        .to_vec();
    let read = prefs.clone();
    let write = prefs.clone();
    let field = SettingField::dropdown(
        options,
        move |cx| read.read(cx).settings().theme.label().into(),
        move |label: SharedString, cx| {
            let Some(theme) = ThemeChoice::ALL.into_iter().find(|t| label == t.label()) else {
                return;
            };
            write.update(cx, |prefs, cx| {
                // The theme is applied whether or not the file is written.
                prefs.change(|settings| settings.theme = theme).ok();
                cx.notify();
            });
            theme.apply(cx);
        },
    )
    .default_value(SharedString::from(ThemeChoice::default().label()));
    SettingPage::new("Appearance").group(
        SettingGroup::new().item(
            SettingItem::new("Theme", field)
                .description("System follows the appearance chosen in macOS."),
        ),
    )
}
