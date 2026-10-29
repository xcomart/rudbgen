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
