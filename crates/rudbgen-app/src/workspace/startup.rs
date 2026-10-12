//! Background work initiated once the workspace is constructed.
use super::*;

impl Workspace {
    pub(super) fn start_background_work(cx: &mut Context<Self>) {
        // The start-up update check, off the UI thread: it is an HTTPS request
        // to GitHub, and nothing on screen waits for it (architecture document,
        // §4.1). The tag the user may have ignored is read here, on the UI
        // thread, because the settings global is only reachable from it.
        //
        // The answer opens a dialog, so it deliberately does *not* go through
        // `open_about`'s `close_overlays` route: this is the one dialog nobody
        // asked for, arriving at a moment nobody chose, and it must never take
        // the screen from something the user opened themselves. If anything is
        // already up, the check simply says nothing and tries again next
        // launch.
        //
        // The guard is here, in this crate, rather than inside the check:
        // `cfg!(test)` compiled into a dependency is that dependency's build,
        // so `rugpui_shell::update::check` cannot tell a test build of *this*
        // crate from a release one, and every render test that opens a window
        // would make a real request to GitHub.
        //
        // `rugpui_shell::update::set_startup_check_enabled(false)` says the same
        // thing to the shell and is the other half of the guard; the tests
        // install it once, so that a path reaching the check some other way is
        // still silent. See [`tests::silence_the_update_check`].
        if !cfg!(test) {
            let ignored = update::ignored_release(cx);
            cx.spawn(async move |this, cx| {
                let found = cx
                    .background_executor()
                    .spawn(async move { update::check(ignored.as_deref()) })
                    .await;
                let Some(release) = found else {
                    return;
                };
                this.update(cx, |workspace, cx| {
                    if workspace.dialog_open(cx) {
                        log::debug!("update {} announced while a dialog is open", release.tag);
                        return;
                    }
                    workspace.update.update(cx, |dialog, cx| {
                        dialog.open(release, cx);
                    });
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }
}
