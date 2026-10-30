use super::*;

/// A template tab pushed straight onto the strip, without opening a buffer.
///
/// The reorder path only reads the item's identity — the file — and its
/// title and dirty flag, so no editor entity is needed to prove where a
/// drop lands and what follows it there.
fn template(file: &str, dirty: bool) -> PaneItem {
    PaneItem::Template {
        file: PathBuf::from(file),
        title: SharedString::from(file),
        dirty,
    }
}

/// The drop bar follows the pointer, and a drop lands where it last pointed.
///
/// The whole gesture at workspace level: `drag_over_tab` arms the bar while
/// the pointer is over the strip, and `drop_tab` takes it and moves the item
/// into the gap the strip reports — the visible tab following along, the
/// dirty dot traveling with its item, and no close prompt anywhere, because
/// a move is not a close.
#[gpui::test]
fn work_tabs_reorder_to_the_drop_bar_and_keep_the_visible_tab(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
    });
    silence_the_update_check();
    let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));

    window
        .update(cx, |workspace, _window, cx| {
            workspace.pane.push(template("/cfg/a.java", false));
            workspace.pane.push(template("/cfg/b.java", true));
            workspace.pane.push(PaneItem::Preview {
                title: SharedString::from("a.java"),
            });
            assert_eq!(workspace.pane.items().len(), 4);
            // Pushing activates the pushed tab, so put the Generate tab
            // back on top: it is the visible one the drag starts from.
            assert!(workspace.pane.activate(0));

            let own = DraggedTab {
                source: WORK_STRIP_TAG,
                index: 0,
            };
            // The bar arms while the pointer names a gap...
            workspace.drag_over_tab(WORK_STRIP_TAG, own, Some(4), cx);
            assert_eq!(workspace.tab_drop.indicator(WORK_STRIP_TAG), Some(4));
            // ...and moves with it, gap by gap.
            workspace.drag_over_tab(WORK_STRIP_TAG, own, Some(2), cx);
            assert_eq!(workspace.tab_drop.indicator(WORK_STRIP_TAG), Some(2));
            // A foreign drag never raises this strip's bar...
            workspace.drag_over_tab(
                WORK_STRIP_TAG,
                DraggedTab {
                    source: 7,
                    index: 0,
                },
                Some(2),
                cx,
            );
            assert_eq!(workspace.tab_drop.indicator(WORK_STRIP_TAG), Some(2));
            // ...and leaving the strip on another tag's report wipes
            // nothing, while leaving this strip takes the bar down.
            workspace.drag_over_tab(
                9,
                DraggedTab {
                    source: WORK_STRIP_TAG,
                    index: 0,
                },
                None,
                cx,
            );
            assert_eq!(workspace.tab_drop.indicator(WORK_STRIP_TAG), Some(2));
            workspace.drag_over_tab(WORK_STRIP_TAG, own, None, cx);
            assert_eq!(workspace.tab_drop.current(), None);

            // Re-arm and drop: the Generate tab itself rides to gap 2,
            // the dirty template keeps its dot, and the tab on top is
            // still the Generate tab — at its new index.
            workspace.drag_over_tab(WORK_STRIP_TAG, own, Some(2), cx);
            assert!(workspace.drop_tab(own, 2, cx));
            assert_eq!(workspace.tab_drop.current(), None);
            assert_eq!(
                workspace.pane.items().to_vec(),
                vec![
                    template("/cfg/a.java", false),
                    PaneItem::Generate,
                    template("/cfg/b.java", true),
                    PaneItem::Preview {
                        title: SharedString::from("a.java"),
                    },
                ]
            );
            assert_eq!(workspace.pane.active(), Some(&PaneItem::Generate));
            assert_eq!(workspace.pane.active_index(), 1);
            assert_eq!(workspace.closing, None);

            // The preview rides too: gap 0 puts it first, and the
            // Generate tab follows to index 2.
            assert!(workspace.pane.relocate(3, 0));
            assert!(matches!(
                workspace.pane.items()[0],
                PaneItem::Preview { .. }
            ));
            assert_eq!(workspace.pane.active(), Some(&PaneItem::Generate));
            assert_eq!(workspace.pane.active_index(), 2);
            assert_eq!(workspace.closing, None);
        })
        .expect("the window is open");
}

/// A canceled drag leaves no bar behind, a drop with no bar still lands in
/// the gap the strip reports, and a foreign tab is refused.
///
/// The root's own release is what clears a canceled gesture — the drop
/// handler never runs for it — while `drop_tab` reads nothing back from the
/// bar: the strip hands it the gap, and the keyboard reorder has no bar.
#[gpui::test]
fn a_canceled_tab_drag_leaves_no_bar_and_a_bare_drop_lands_in_the_reported_gap(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
    });
    silence_the_update_check();
    let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));

    window
        .update(cx, |workspace, _window, cx| {
            let own = DraggedTab {
                source: WORK_STRIP_TAG,
                index: 0,
            };
            // Clearing nothing is a no-op...
            workspace.clear_tab_drop(cx);
            assert_eq!(workspace.tab_drop.current(), None);
            // ...arming then releasing takes the bar down...
            workspace.drag_over_tab(WORK_STRIP_TAG, own, Some(2), cx);
            assert_eq!(workspace.tab_drop.indicator(WORK_STRIP_TAG), Some(2));
            workspace.clear_tab_drop(cx);
            assert_eq!(workspace.tab_drop.current(), None);
            // ...a drop with no bar behind it lands in the gap reported...
            workspace.pane.push(template("/cfg/a.java", false));
            assert!(workspace.drop_tab(own, 2, cx));
            assert_eq!(
                workspace.pane.items().to_vec(),
                vec![template("/cfg/a.java", false), PaneItem::Generate]
            );
            // ...and a foreign tab moves nothing but still takes the bar.
            workspace.drag_over_tab(WORK_STRIP_TAG, own, Some(0), cx);
            let foreign = DraggedTab {
                source: 7,
                index: 0,
            };
            assert!(!workspace.drop_tab(foreign, 0, cx));
            assert_eq!(workspace.tab_drop.current(), None);
            assert_eq!(
                workspace.pane.items().to_vec(),
                vec![template("/cfg/a.java", false), PaneItem::Generate]
            );
        })
        .expect("the window is open");
}

/// A render coming back relabels the preview tab where it stands.
///
/// The tab keeps the gap it was dragged to and the strip keeps the tab on
/// top: closing it and pushing it again would send it to the end and to the
/// front — mid-drag, if the render came back during one.
#[gpui::test]
fn the_preview_tab_is_retitled_in_place(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
    });
    silence_the_update_check();
    let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));

    window
        .update(cx, |workspace, _window, cx| {
            workspace.pane.push(PaneItem::Preview {
                title: SharedString::from("stale.java"),
            });
            workspace.pane.push(template("/cfg/a.java", false));
            assert!(workspace.pane.relocate(1, 0));
            // Something other than the preview on top: the Generate tab.
            assert!(workspace.pane.activate(1));

            workspace.retitle_preview(cx);
            let shown = workspace.preview.read(cx).title();
            assert_eq!(
                workspace.pane.items().to_vec(),
                vec![
                    PaneItem::Preview { title: shown },
                    PaneItem::Generate,
                    template("/cfg/a.java", false),
                ]
            );
            assert_eq!(workspace.pane.active(), Some(&PaneItem::Generate));
        })
        .expect("the window is open");
}

/// A window with two template tabs open over real files, and the Generate
/// tab in front of them.
fn two_templates(
    cx: &mut gpui::TestAppContext,
) -> (tempfile::TempDir, gpui::WindowHandle<Workspace>) {
    cx.update(|cx| {
        app_settings::init(cx);
        rugpui::init(cx);
        rugpui_editor::init(cx);
    });
    silence_the_update_check();
    let dir = tempfile::tempdir().expect("tempdir");
    let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));
    window
        .update(cx, |workspace, _window, cx| {
            for name in ["a.java", "b.java"] {
                let file = dir.path().join(name);
                std::fs::write(&file, "class A {}\n").expect("the template is written");
                workspace.open_template(file, cx);
            }
            assert!(workspace.pane.activate(0));
        })
        .expect("the window is open");
    (dir, window)
}

/// `Ctrl+PageDown` and `Ctrl+PageUp` walk the strip and wrap at its ends; the
/// numbered chords go straight to a tab.
#[gpui::test]
fn the_tab_keys_walk_the_strip(cx: &mut gpui::TestAppContext) {
    let (_dir, window) = two_templates(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.next_tab_action(&NextTab, window, cx);
            assert_eq!(workspace.pane.active_index(), 1);
            workspace.next_tab_action(&NextTab, window, cx);
            assert_eq!(workspace.pane.active_index(), 2);
            workspace.next_tab_action(&NextTab, window, cx);
            assert_eq!(workspace.pane.active_index(), 0, "the end did not wrap");
            workspace.prev_tab_action(&PrevTab, window, cx);
            assert_eq!(workspace.pane.active_index(), 2, "the start did not wrap");
            workspace.select_tab_action(&SelectTab(1), window, cx);
            assert_eq!(workspace.pane.active_index(), 1);
            // A digit past the strip is nothing to go to.
            workspace.select_tab_action(&SelectTab(8), window, cx);
            assert_eq!(workspace.pane.active_index(), 1);
        })
        .expect("the window is open");
}

/// `Ctrl+W` closes the tab on top through the same door as its close button:
/// a template with edits asks, and the Generate tab is never taken.
#[gpui::test]
fn close_tab_asks_and_spares_the_generate_tab(cx: &mut gpui::TestAppContext) {
    let (_dir, window) = two_templates(cx);
    window
        .update(cx, |workspace, window, cx| {
            workspace.close_tab_action(&CloseTab, window, cx);
            assert_eq!(workspace.pane.items().len(), 3, "the Generate tab closed");

            workspace.select_tab_action(&SelectTab(1), window, cx);
            let at = workspace.active_template(cx).expect("a template is on top");
            workspace.templates[at].update(cx, |pane, cx| pane.insert("// edit\n", window, cx));
            workspace.close_tab_action(&CloseTab, window, cx);
            assert!(workspace.closing.is_some(), "the edit was not asked about");
            workspace.cancel_close(window, cx);
            assert_eq!(workspace.pane.items().len(), 3);
        })
        .expect("the window is open");
}

/// The tab menu's *Close other tabs* closes the clean ones at once and asks
/// about the one with edits, carrying on once that is answered — and leaves
/// the Generate tab standing.
#[gpui::test]
fn close_others_asks_about_each_edited_tab(cx: &mut gpui::TestAppContext) {
    let (_dir, window) = two_templates(cx);
    window
        .update(cx, |workspace, window, cx| {
            let Some(PaneItem::Template { file, .. }) = workspace.pane.get(2).cloned() else {
                panic!("the second template is not where it was opened");
            };
            let at = workspace
                .template_index(&file, cx)
                .expect("the second template is open");
            workspace.templates[at].update(cx, |pane, cx| pane.insert("// edit\n", window, cx));
            workspace.close_tabs(&[0, 1, 2], window, cx);
            assert!(
                workspace.closing.is_some(),
                "the edited tab was not asked about"
            );
            assert_eq!(
                workspace.pane.items().len(),
                2,
                "the clean tab is still open"
            );
            workspace.answer_close(false, window, cx);
            assert_eq!(workspace.pane.items().len(), 1);
            assert!(matches!(workspace.pane.get(0), Some(PaneItem::Generate)));
        })
        .expect("the window is open");
}
