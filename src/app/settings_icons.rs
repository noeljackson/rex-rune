//! Small category pictograms, separate from monochrome action icons.
use super::{panel::SettingsSection, theme::*, widgets::glyph};
use rune::{Color, El, el};

pub fn category(section: SettingsSection) -> El {
    let (top, bottom, icon) = match section.root() {
        SettingsSection::Interface => (0x62656c, 0x272a30, &TABS_TOP),
        SettingsSection::Terminal => (0x426b40, 0x18361e, &TERMINAL),
        SettingsSection::Shortcuts => (0x8b8d92, 0x515359, &COMMAND),
        SettingsSection::Integrations => (0xe67aaf, 0x8459d9, &SETTINGS_PUZZLE),
        SettingsSection::Notifications => (0xff8964, 0xf14026, &SETTINGS_BELL),
        SettingsSection::Hosts => (0xd36af3, 0x9230e6, &SETTINGS_GLOBE),
        SettingsSection::Updates => (0x67afe4, 0x286cd6, &SETTINGS_UPDATE),
        SettingsSection::About => (0x8fa7b9, 0x536a7f, &SETTINGS_INFO),
        SettingsSection::Tabs | SettingsSection::Panes => unreachable!(),
    };
    let mut badge = el()
        .key(format!("settings-icon:{}", section.root().key()))
        // The pictogram is fixed artwork; only the navigation row changes emphasis.
        .motion(rune::Spring::NONE)
        .size(20., 20.)
        .radius(5.)
        .continuous_corners()
        .linear_gradient(1.8, [(0., rgb(top)), (1., rgb(bottom))])
        .border(1., white(0.16))
        .center()
        .child(glyph(icon, 15., Color::WHITE));
    if section.root() == SettingsSection::Interface {
        // Window traffic lights are part of the category pictogram, not controls.
        for (x, color) in [(4., 0xff655b), (9., 0xffc84a), (14., 0x64d76b)] {
            badge = badge.child(
                el().absolute(x, 3.)
                    .size(3., 3.)
                    .radius(2.)
                    .fill(rgb(color)),
            );
        }
    }
    badge
}
