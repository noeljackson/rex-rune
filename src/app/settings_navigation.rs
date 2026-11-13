//! Window-local Settings search and page history. None of this changes preferences.
use super::{panel::SettingsSection, workspace::Workspace};
use rune::{TextEdit, Tx};

#[derive(Clone)]
struct Visit {
    section: SettingsSection,
    query: String,
    scroll: f32,
    focus: String,
}

pub struct Navigation {
    pub search: TextEdit,
    pub offset: rune::Live<f32>,
    back: Vec<Visit>,
    forward: Vec<Visit>,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            search: TextEdit::new(""),
            offset: rune::Live::new(0.),
            back: Vec::new(),
            forward: Vec::new(),
        }
    }
}

impl Navigation {
    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }
    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }
    pub fn scroll_key(&self, section: SettingsSection) -> String {
        format!(
            "settings-scroll:{}",
            if self.search.text.trim().is_empty() {
                section.key()
            } else {
                "search"
            }
        )
    }
    fn snapshot(&self, section: SettingsSection, focus: String) -> Visit {
        Visit {
            section,
            query: self.search.text.clone(),
            scroll: self.offset.get(),
            focus,
        }
    }
    pub fn leave(&mut self, section: SettingsSection, next: SettingsSection) {
        if section != next || !self.search.text.is_empty() {
            self.back.push(self.snapshot(
                section,
                format!("settings-section:{}", section.root().key()),
            ));
            self.forward.clear();
            self.search = TextEdit::new("");
            self.offset = rune::Live::new(0.);
        }
    }
}

impl Workspace {
    pub fn settings_open(&mut self, section: SettingsSection, origin: &str, tx: &mut Tx) {
        let before = self.settings_navigation.back.len();
        self.open_section(section);
        if self.settings_navigation.back.len() > before {
            self.settings_navigation.back.last_mut().unwrap().focus = origin.to_owned();
        }
        tx.focus("settings-page-title", false);
        tx.scroll_to_y(self.settings_navigation.scroll_key(section), 0.);
    }

    pub fn settings_history(&mut self, forward: bool, tx: &mut Tx) {
        let nav = &mut self.settings_navigation;
        let Some(visit) = (if forward {
            &mut nav.forward
        } else {
            &mut nav.back
        })
        .pop() else {
            return;
        };
        let current = nav.snapshot(self.settings_section, "settings-page-title".into());
        (if forward {
            &mut nav.back
        } else {
            &mut nav.forward
        })
        .push(current);
        // open_section performs page-specific refreshes; preserve the history while it does.
        let saved = std::mem::take(nav);
        self.open_section(visit.section);
        self.settings_navigation = saved;
        let nav = &mut self.settings_navigation;
        nav.search = TextEdit::new(&visit.query);
        nav.offset = rune::Live::new(visit.scroll);
        tx.scroll_to_y(nav.scroll_key(visit.section), visit.scroll);
        let ring = visit.focus != "settings-page-title";
        tx.focus(visit.focus, ring);
    }
}

pub struct Setting {
    pub page: SettingsSection,
    pub title: &'static str,
}

/// Search describes implemented choices, including the subpage that owns each choice.
pub fn results(query: &str) -> Vec<Setting> {
    use SettingsSection::*;
    let query = query.to_lowercase();
    let entries = [
        (
            Interface,
            "Appearance and interface zoom",
            "theme chrome palette colors scale size",
        ),
        (
            Tabs,
            "Tab position and sidebar",
            "top sidebar hosts sessions tabs toolbar",
        ),
        (
            Interface,
            "Pane density and headers",
            "comfortable compact split layout hover hidden always",
        ),
        (Panes, "Pane padding", "terminal content space margin inset"),
        (
            Terminal,
            "Terminal typography",
            "font family size fallback glyph text",
        ),
        (
            Terminal,
            "Terminal theme",
            "colors colours ghostty palette follow server chrome",
        ),
        (
            Terminal,
            "Default shell",
            "host command new sessions tabs splits",
        ),
        (
            Notifications,
            "In-app notification position",
            "notices top bottom right corner placement",
        ),
        (
            Notifications,
            "Program activity and notifications",
            "activity tabs desktop error blocked done messages sounds",
        ),
        (
            Integrations,
            "Coding agent integrations",
            "claude codex opencode pi skill hooks",
        ),
        (
            Hosts,
            "Remote hosts and workspace control",
            "server endpoint ssh tailnet consent label",
        ),
        (
            Shortcuts,
            "Keyboard shortcuts",
            "bindings keys keymap overrides commands",
        ),
        (
            Updates,
            "Updates",
            "version release upgrade automatic check",
        ),
        (
            About,
            "About Rex Linux",
            "version build debug license diagnostics",
        ),
    ];
    entries
        .into_iter()
        .filter_map(|(page, title, words)| {
            let haystack = format!("{} {title} {words}", page.title()).to_lowercase();
            query
                .split_whitespace()
                .all(|word| haystack.contains(word))
                .then_some(Setting { page, title })
        })
        .collect()
}

/// Settings-only shortcuts are local controls, with the same behavior as its header buttons.
pub fn on_key(s: &mut Workspace, ev: &rune::KeyEvent, tx: &mut Tx) -> bool {
    if s.panel
        .as_ref()
        .is_some_and(|p| matches!(p.kind, super::panel::PanelKind::Settings))
        && ev.mods.alt
        && !ev.mods.ctrl
        && !ev.mods.cmd
        && matches!(ev.key, rune::Key::Left | rune::Key::Right)
    {
        s.settings_history(ev.key == rune::Key::Right, tx);
        true
    } else {
        false
    }
}
