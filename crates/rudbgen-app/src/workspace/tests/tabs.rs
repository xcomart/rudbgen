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
/// the pointer is over the strip, `resolve_tab_drop` takes it on drop, and
/// `relocate` moves the item — the visible tab following along, the dirty
/// dot travelling with its item, and no close prompt anywhere, because a
/// move is not a close.
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
            assert_eq!(workspace.tab_drop, Some((WORK_STRIP_TAG, 4)));
            // ...and moves with it, gap by gap.
            workspace.drag_over_tab(WORK_STRIP_TAG, own, Some(2), cx);
            assert_eq!(workspace.tab_drop, Some((WORK_STRIP_TAG, 2)));
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
            assert_eq!(workspace.tab_drop, Some((WORK_STRIP_TAG, 2)));
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
            assert_eq!(workspace.tab_drop, Some((WORK_STRIP_TAG, 2)));
            workspace.drag_over_tab(WORK_STRIP_TAG, own, None, cx);
            assert_eq!(workspace.tab_drop, None);

            // Re-arm and drop: the Generate tab itself rides to gap 2,
            // the dirty template keeps its dot, and the tab on top is
            // still the Generate tab — at its new index.
            workspace.drag_over_tab(WORK_STRIP_TAG, own, Some(2), cx);
            let at = workspace.resolve_tab_drop(WORK_STRIP_TAG, 0);
            assert_eq!(at, 2);
            assert_eq!(workspace.tab_drop, None);
            assert!(workspace.pane.relocate(own.index, at));
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

/// A cancelled drag leaves no bar behind, and a drop with no bar lands on
/// the static position.
///
/// The root's own release is what clears a cancelled gesture — the drop
/// handler never runs for it — and `resolve_tab_drop` falls back when the
/// gesture left nothing to take.
#[gpui::test]
fn a_cancelled_tab_drag_leaves_no_bar_and_a_bare_drop_falls_back(cx: &mut gpui::TestAppContext) {
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
            assert_eq!(workspace.tab_drop, None);
            // ...arming then releasing takes the bar down...
            workspace.drag_over_tab(WORK_STRIP_TAG, own, Some(2), cx);
            assert_eq!(workspace.tab_drop, Some((WORK_STRIP_TAG, 2)));
            workspace.clear_tab_drop(cx);
            assert_eq!(workspace.tab_drop, None);
            // ...and a drop with no bar behind it lands on the static
            // position the bar would have reported.
            assert_eq!(workspace.resolve_tab_drop(WORK_STRIP_TAG, 2), 2);
            assert_eq!(workspace.tab_drop, None);
        })
        .expect("the window is open");
}
