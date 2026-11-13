# Mac action compatibility

The baseline is Rex Beta 0.1.0 (1925), server 0.1.0-preview.14 / API 12, captured read-only on 2026-11-12. The [fixture](../../tests/fixtures/parity/mac-preview14.json) contains only action names, schemas and repeat metadata, with no client, session or terminal data. This is a versioned compatibility target, not a claim about later Mac releases.

The [machine-readable matrix](../../tests/fixtures/parity/compatibility.json) covers every baseline action. Tests compare registration and remote decisions against the actual registry and validate sample arguments derived from the captured schemas. Keep both matrix and tests honest when changing a declaration.

Supported means registered for local bindings and Commands; it does not mean remotely callable. Alias means the existing handler is reused. Partial means an argument or behavior differs. Unsupported means the name is recognized with an explicit diagnostic. Missing means it is not registered. Platform-specific entries need a Linux-specific design rather than a no-op alias. The key-forwarding binding is a special path, not a remotely callable UI action.

## Remote control

The saved host owns consent. Off remains the default; the original view-only option permits exactly eight canonical action names. Added aliases remain remote-denied even when their canonical action is allowed. Broader workspace consent and correlated mutation replies are tracked separately in issues [4](https://github.com/noeljackson/rex-rune/issues/4) and [5](https://github.com/noeljackson/rex-rune/issues/5). Clipboard and local system access are not granted by this contract.

## Arguments and server revisions

Existing Linux spellings and defaults remain valid. `pane.split` accepts both Mac `auto` and Linux `best_fit`; directional aliases use the same split handler. `client.tab.move` accepts backward/forward as well as left/right. `pane.focus` accepts an optional boolean `wrap`; omitting it preserves the server's default, which differs from the Mac default. `client.theme.change` currently opens the picker and rejects its Mac optional `name` argument.

All client connections negotiate supported server API revisions 10–13. This baseline was observed on revision 12. Registration alone does not prove a method works on earlier revisions: method refusals are surfaced and requests are not retried. `pane.move_to_new_tab` is explicitly unsupported because the tested server cannot create an empty tab around an existing block. Do not create a sacrificial shell to hide that gap.

## Reused upstream work

Imported upstream commits 99dc9db and c982398 (PR 57) provide local compatibility aliases, config reload, pane resize/move/balance, page scrolling and key modes, including the reviewed balancing correction. Attribution is retained in the cherry-picked commits. Upstream issue [44](https://github.com/shuv1337/rex-rune/issues/44) remains relevant; later prompt-navigation work in PR 60 has not been imported here. Cross-server Lua call relaying remains separate from client.perform, as tracked by upstream issue [20](https://github.com/shuv1337/rex-rune/issues/20).

## Baseline matrix

| Mac action | Local status | Handler / difference | Remote |
| --- | --- | --- | --- |
| `client.agents.setup` | missing | Not registered in this Linux baseline. | denied |
| `client.appearance.switch_mode` | missing | Not registered in this Linux baseline. | denied |
| `client.close` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.config.copy_path` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.config.reload` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.debug.copy` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.diagnostics` | missing | Not registered in this Linux baseline. | denied |
| `client.find.next` | alias | `client.search.next` | denied |
| `client.find.open` | alias | `client.search` | denied |
| `client.find.previous` | alias | `client.search.previous` | denied |
| `client.font.decrease` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.font.increase` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.font.reset` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.host.add` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.host.remove` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.host.switch` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.key.forward` | binding | Special keymap forwarding action; it is not a client.perform declaration. | denied |
| `client.mode.enter` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.mode.exit` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.mode.exit_all` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.notice.dismiss` | missing | Not registered in this Linux baseline. | denied |
| `client.onboarding` | missing | Not registered in this Linux baseline. | denied |
| `client.pair_device` | platform specific | Mac-specific integration or update workflow; no direct Linux contract yet. | denied |
| `client.palette.toggle` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.pane_header.reveal` | missing | Not registered in this Linux baseline. | denied |
| `client.permissions` | platform specific | Mac-specific integration or update workflow; no direct Linux contract yet. | denied |
| `client.prompt.next` | unsupported | OSC 133 prompt navigation has not been imported; upstream PR 60 implements it. | denied |
| `client.prompt.previous` | unsupported | OSC 133 prompt navigation has not been imported; upstream PR 60 implements it. | denied |
| `client.quick_look` | missing | Not registered in this Linux baseline. | denied |
| `client.scroll.page_down` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.scroll.page_up` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.server.restart` | missing | Not registered in this Linux baseline. | denied |
| `client.settings` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.shortcuts.edit` | missing | Not registered in this Linux baseline. | denied |
| `client.sidebar.activity.by_priority` | missing | Not registered in this Linux baseline. | denied |
| `client.sidebar.activity.by_session` | missing | Not registered in this Linux baseline. | denied |
| `client.sidebar.activity.toggle` | missing | Not registered in this Linux baseline. | denied |
| `client.sidebar.filter.focus` | missing | Not registered in this Linux baseline. | denied |
| `client.sidebar.toggle` | supported | Registered local action; remote availability is a separate permission. | view |
| `client.tab.close` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.tab.close_others` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.tab.goto` | supported | Registered local action; remote availability is a separate permission. | view |
| `client.tab.move` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.tab.move.backward` | alias | `client.tab.move` | denied |
| `client.tab.move.forward` | alias | `client.tab.move` | denied |
| `client.tab.move_to_new_window` | missing | Not registered in this Linux baseline. | denied |
| `client.tab.new` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.tab.next` | supported | Registered local action; remote availability is a separate permission. | view |
| `client.tab.previous` | supported | Registered local action; remote availability is a separate permission. | view |
| `client.tab.rename` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.tab_style.toggle` | missing | Not registered in this Linux baseline. | denied |
| `client.theme.change` | partial | Picker supported; the optional theme name argument is not supported. | denied |
| `client.theme.import` | missing | Not registered in this Linux baseline. | denied |
| `client.toolbar.reveal` | missing | Not registered in this Linux baseline. | denied |
| `client.update.accept` | platform specific | Mac-specific integration or update workflow; no direct Linux contract yet. | denied |
| `client.update.accept_when_idle` | platform specific | Mac-specific integration or update workflow; no direct Linux contract yet. | denied |
| `client.update.check` | missing | Not registered in this Linux baseline. | denied |
| `client.update.install` | platform specific | Mac-specific integration or update workflow; no direct Linux contract yet. | denied |
| `client.update.set_automation` | platform specific | Mac-specific integration or update workflow; no direct Linux contract yet. | denied |
| `client.update.set_channel` | platform specific | Mac-specific integration or update workflow; no direct Linux contract yet. | denied |
| `client.update.status` | missing | Not registered in this Linux baseline. | denied |
| `client.version.copy` | supported | Registered local action; remote availability is a separate permission. | denied |
| `client.visibility.toggle` | missing | Not registered in this Linux baseline. | denied |
| `client.window.close` | missing | Not registered in this Linux baseline. | denied |
| `client.window.fullscreen.toggle` | missing | Not registered in this Linux baseline. | denied |
| `client.window.new` | missing | Not registered in this Linux baseline. | denied |
| `pane.balance` | supported | Registered local action; remote availability is a separate permission. | denied |
| `pane.close` | supported | Registered local action; remote availability is a separate permission. | denied |
| `pane.close_others` | supported | Registered local action; remote availability is a separate permission. | denied |
| `pane.copy` | supported | Registered local action; remote availability is a separate permission. | denied |
| `pane.copy_id` | supported | Registered local action; remote availability is a separate permission. | denied |
| `pane.float` | missing | Not registered in this Linux baseline. | denied |
| `pane.focus` | partial | Explicit wrap is forwarded; omitted wrap preserves the server default (false on the tested API 12 server), while Mac defaults to true. | view |
| `pane.focus.down` | alias | `pane.focus` | denied |
| `pane.focus.left` | alias | `pane.focus` | denied |
| `pane.focus.right` | alias | `pane.focus` | denied |
| `pane.focus.up` | alias | `pane.focus` | denied |
| `pane.focus_next` | supported | Registered local action; remote availability is a separate permission. | denied |
| `pane.focus_previous` | supported | Registered local action; remote availability is a separate permission. | denied |
| `pane.go_to` | missing | Not registered in this Linux baseline. | denied |
| `pane.go_to_directory` | missing | Not registered in this Linux baseline. | denied |
| `pane.insert_path` | missing | Not registered in this Linux baseline. | denied |
| `pane.minimize_all_floating` | missing | Not registered in this Linux baseline. | denied |
| `pane.minimize_floating` | missing | Not registered in this Linux baseline. | denied |
| `pane.move` | supported | Registered local action; remote availability is a separate permission. | denied |
| `pane.move.down` | alias | `pane.move` | denied |
| `pane.move.left` | alias | `pane.move` | denied |
| `pane.move.right` | alias | `pane.move` | denied |
| `pane.move.up` | alias | `pane.move` | denied |
| `pane.move_to_new_tab` | unsupported | Server API 12 cannot make an empty tab around an existing block without starting another program. | denied |
| `pane.move_to_tab` | supported | Registered local action; remote availability is a separate permission. | denied |
| `pane.new_floating` | missing | Not registered in this Linux baseline. | denied |
| `pane.paste` | supported | Registered local action; remote availability is a separate permission. | denied |
| `pane.rename` | supported | Registered local action; remote availability is a separate permission. | denied |
| `pane.resize` | supported | Registered local action; remote availability is a separate permission. | denied |
| `pane.select_all` | supported | Registered local action; remote availability is a separate permission. | denied |
| `pane.send_key` | supported | Registered local action; remote availability is a separate permission. | denied |
| `pane.split` | supported | Registered local action; remote availability is a separate permission. | denied |
| `pane.split.auto` | alias | `pane.split` | denied |
| `pane.split.down` | alias | `pane.split` | denied |
| `pane.split.left` | alias | `pane.split` | denied |
| `pane.split.right` | alias | `pane.split` | denied |
| `pane.split.up` | alias | `pane.split` | denied |
| `pane.view_activity` | missing | Not registered in this Linux baseline. | denied |
| `pane.zoom` | supported | Registered local action; remote availability is a separate permission. | view |
| `session.close` | supported | Registered local action; remote availability is a separate permission. | denied |
| `session.collapse` | missing | Not registered in this Linux baseline. | denied |
| `session.collapse_others` | missing | Not registered in this Linux baseline. | denied |
| `session.expand` | missing | Not registered in this Linux baseline. | denied |
| `session.new` | supported | Registered local action; remote availability is a separate permission. | denied |
| `session.next` | supported | Registered local action; remote availability is a separate permission. | denied |
| `session.open_on_web` | platform specific | Mac-specific integration or update workflow; no direct Linux contract yet. | denied |
| `session.previous` | supported | Registered local action; remote availability is a separate permission. | denied |
| `session.refresh` | supported | Registered local action; remote availability is a separate permission. | denied |
| `session.rename` | supported | Registered local action; remote availability is a separate permission. | denied |
| `session.select` | supported | Registered local action; remote availability is a separate permission. | view |
| `session.switch` | supported | Registered local action; remote availability is a separate permission. | denied |
