//! Illustrative Settings samples. These never read or mutate a terminal or session.
use super::{theme::*, widgets::glyph, workspace::Workspace};
use rex_linux::settings::{NoticePosition, PaneDensity, PaneHeader};
use rune::{El, col, el, row, text};

fn frame(key: &str, label: &str) -> El {
    col()
        .key(key.to_owned())
        .role("img")
        .label(label.to_owned())
        .w_full()
        .min_w(0.)
        .padding(12.)
        .gap(8.)
        .radius(10.)
        .fill(rgb(ground()))
        .border(1., white(0.1))
        .clip_rounded(10.)
}

fn tab(label: &str, selected: bool) -> El {
    row()
        .min_w(0.)
        .gap(6.)
        .px(8.)
        .h(25.)
        .center_y()
        .radius(7.)
        .fill(white(if selected { 0.12 } else { 0.02 }))
        .child(glyph(&TERMINAL, 13., rgb(muted())))
        .child(
            text(label.to_owned())
                .font(medium(11.))
                .color(rgb(if selected { ink() } else { muted() }))
                .truncate(),
        )
}

pub fn appearance() -> El {
    frame(
        "preview-appearance",
        "Appearance preview: window, tabs and two terminal panes",
    )
    .child(row().gap(5.).children(
        [0xff655b, 0xffc84a, 0x64d76b].map(|c| el().size(6., 6.).radius(3.).fill(rgb(c))),
    ))
    .child(
        row()
            .gap(6.)
            .child(tab("Feature work", true))
            .child(tab("Code review", false)),
    )
    .child(row().gap(6.).h(44.).children((0..2).map(|_| {
        el().grow(1.)
            .h_full()
            .radius(5.)
            .fill(white(0.04))
            .border(1., white(0.07))
    })))
}

pub fn tabs(s: &Workspace) -> El {
    let tabs = if s.settings.vertical_tabs {
        col()
            .w(145.)
            .h_full()
            .gap(4.)
            .child(tab("Feature work", true))
            .child(tab("Code review", false))
    } else {
        row()
            .w_full()
            .gap(6.)
            .child(tab("Feature work", true))
            .child(tab("Code review", false))
    };
    let content = el()
        .grow(1.)
        .h_full()
        .min_h(30.)
        .radius(5.)
        .fill(white(0.03));
    let layout = if s.settings.vertical_tabs {
        row().h(95.).gap(8.).child(tabs).child(content)
    } else {
        col().h(95.).gap(8.).child(tabs).child(content)
    };
    frame(
        "preview-tabs",
        if s.settings.vertical_tabs {
            "Tab preview: selected and inactive tabs in the sidebar"
        } else {
            "Tab preview: selected and inactive tabs above the terminal"
        },
    )
    .child(layout)
}

pub fn panes(s: &Workspace) -> El {
    let compact = s.settings.pane_density == PaneDensity::Compact;
    let header = s.settings.pane_header;
    let mut panes = row().h(115.).gap(if compact { 1. } else { 7. });
    for i in 0..2 {
        let show_header = header == PaneHeader::Always
            || header == PaneHeader::OnHover && i == 0
            || header == PaneHeader::Auto && !compact;
        let mut pane = col()
            .grow(1.)
            .min_w(0.)
            .h_full()
            .radius(if compact { 0. } else { 6. })
            .fill(white(0.025))
            .border(
                if compact { 0. } else { 1. },
                if i == 0 {
                    rgb(accent()).alpha(0.5)
                } else {
                    white(0.08)
                },
            );
        if show_header {
            let heading = row()
                .w_full()
                .h(22.)
                .px(6.)
                .gap(4.)
                .center_y()
                .child(
                    text(if i == 0 { "shell" } else { "editor" })
                        .font(ui(10.))
                        .color(rgb(muted()))
                        .grow(1.)
                        .truncate(),
                )
                .child(glyph(&FLOAT_PANE, 11., rgb(muted())))
                .child(glyph(&ZOOM, 11., rgb(muted())))
                .child(glyph(&CLOSE, 11., rgb(muted())));
            pane = pane.child(if header == PaneHeader::OnHover {
                heading.absolute(0., 0.).fill(rgb(popover()))
            } else {
                heading
            });
        }
        let padding = s.settings.pane_padding;
        pane = pane.child(
            col()
                .padding(padding)
                .gap(5.)
                .child(el().w(46.).h(3.).radius(1.).fill(rgb(ink()).alpha(0.6)))
                .child(el().w(70.).h(3.).radius(1.).fill(rgb(muted()).alpha(0.4)))
                .child(el().w(28.).h(3.).radius(1.).fill(rgb(accent()).alpha(0.6))),
        );
        panes = panes.child(pane);
    }
    frame(
        "preview-panes",
        &format!(
            "Pane preview: {}, headers {:?}, padding {} pt",
            if compact { "compact" } else { "comfortable" },
            header,
            s.settings.pane_padding
        ),
    )
    .child(panes)
}

pub fn typography(s: &Workspace) -> El {
    frame(
        "preview-typography",
        "Typography sample using the currently rendered terminal font and selected size",
    )
    .child(
        text("~/project ❯ cargo build\nBuild complete!  0123456789")
            .font(mono(s.settings.font_size))
            .color(rgb(ink()))
            .wrap(),
    )
}

pub fn terminal_theme(s: &Workspace) -> El {
    let theme = s.term_theme.as_ref();
    let bg = theme.and_then(|t| t.background).unwrap_or(ground());
    let fg = theme.and_then(|t| t.foreground).unwrap_or(ink());
    let mut colors = row().gap(4.);
    if let Some(theme) = theme {
        for (_, color) in theme.palette.iter().filter(|(index, _)| *index < 8).take(8) {
            colors = colors.child(
                el().size(15., 15.)
                    .radius(8.)
                    .fill(rgb(*color))
                    .border(1., white(0.15)),
            );
        }
    }
    frame(
        "preview-terminal-theme",
        if theme.is_some() {
            "Selected terminal theme sample; actual panes remain server-owned"
        } else {
            "Interface color sample; Follow server keeps actual terminal colors on the server"
        },
    )
    .child(
        col()
            .w_full()
            .min_w(0.)
            .padding(10.)
            .gap(8.)
            .radius(7.)
            .fill(rgb(bg))
            .child(
                row()
                    .gap(8.)
                    .center_y()
                    .child(glyph(&SETTINGS_PALETTE, 18., rgb(fg)))
                    .child(text("Aa   ~/project   ❯ _").font(mono(16.)).color(rgb(fg))),
            )
            .child(colors),
    )
}

pub fn notifications(s: &Workspace) -> El {
    let bottom = s.settings.notice_position == NoticePosition::BottomRight;
    frame(
        "preview-notification",
        if bottom {
            "Notification preview: bottom right"
        } else {
            "Notification preview: top right"
        },
    )
    .child(
        el().w_full()
            .h(85.)
            .radius(6.)
            .border(1., white(0.09))
            .child(
                row()
                    .absolute(10., 10.)
                    .gap(5.)
                    .children((0..3).map(|_| el().size(24., 4.).radius(2.).fill(white(0.1)))),
            )
            .child(
                row()
                    .w(110.)
                    .h(24.)
                    .radius(6.)
                    .fill(rgb(accent()).alpha(0.2))
                    .border(1., rgb(accent()).alpha(0.6))
                    .pin(
                        None,
                        if bottom { None } else { Some(7.) },
                        Some(7.),
                        if bottom { Some(7.) } else { None },
                    )
                    .px(7.)
                    .gap(5.)
                    .center_y()
                    .child(glyph(&SETTINGS_BELL, 12., rgb(accent())))
                    .child(text("Notification").font(ui(10.)).color(rgb(ink()))),
            ),
    )
}
