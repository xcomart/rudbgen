//! The variable palette: what the inspector's column becomes while a template
//! tab is on top (architecture document, §4.5).
//!
//! Eight groups of entries — the table's fields, a column's, a foreign key's,
//! an index's, the statements, the decorators, the conditions and the
//! variables the Generate tab defines — over a filter box. Clicking one writes
//! it at the caret of the template that is open; hovering one says what it
//! does, and shows what it renders to for the table the live preview is using.
//!
//! What it holds is [`crate::palette`]'s answer and nothing else: the panel has
//! no rule of its own, so what is offered and in what order is decided in a
//! module with no gpui in it and tested there.

use gpui::{
    App, Context, DragMoveEvent, Entity, EventEmitter, FocusHandle, Focusable, ScrollHandle,
    SharedString, Subscription, Window, div, prelude::*, px,
};
use rudbgen_ui::{
    DraggedThumb, Scrollbar, ScrollbarAxis, ScrollbarState, TextInput, hide_later, hide_now,
    scroll_to, scrolled, theme, tooltip_label,
};

use crate::app_settings;
use crate::i18n::ts;
use crate::palette::{PaletteItem, Section};

/// Element id of the panel's scrolling box.
const PANEL_SCROLL: &str = "palette-scroll";

/// Element id of the panel's overlay scroll indicator.
const PANEL_SCROLLBAR: &str = "palette-scrollbar";

/// What the panel asks the shell for.
pub enum PaletteEvent {
    /// Write this at the caret of whatever template tab is on top.
    Insert(SharedString),
}

/// The right-hand panel while a template is being edited.
pub struct VariablePalette {
    focus_handle: FocusHandle,
    /// The filter box over the list.
    search: Entity<TextInput>,
    /// Everything on offer, before the filter.
    items: Vec<PaletteItem>,
    /// Vertical scroll of the list.
    scroll: ScrollHandle,
    /// Whether the overlay scroll indicator is on screen.
    scrollbar: ScrollbarState,
    /// Keeps the filter box's subscription alive.
    _search_events: Subscription,
}

impl VariablePalette {
    /// An empty palette. [`VariablePalette::set_items`] fills it.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new(cx).placeholder(ts!("palette.search")));
        // Every keystroke: this is a narrowing, not a search.
        let search_events = cx.observe(&search, |_palette, _input, cx| cx.notify());

        Self {
            focus_handle: cx.focus_handle(),
            search,
            items: Vec::new(),
            scroll: ScrollHandle::new(),
            scrollbar: ScrollbarState::new(),
            _search_events: search_events,
        }
    }

    /// Replaces what is on offer.
    ///
    /// The filter is deliberately left alone: the entries change whenever the
    /// preview's table changes or a variable is typed into the Generate tab,
    /// and a filter that emptied itself every time would be unusable.
    pub fn set_items(&mut self, items: Vec<PaletteItem>, cx: &mut Context<Self>) {
        self.items = items;
        cx.notify();
    }

    /// How many entries are on offer, filter and all.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// What the filter box holds.
    fn query(&self, cx: &App) -> String {
        self.search.read(cx).content().trim().to_ascii_lowercase()
    }

    /// The entries of `section` that pass the filter.
    ///
    /// The filter matches the name *and* the description, so looking for
    /// "pascal" finds `.pascal` and looking for "primary key" finds `keys`
    /// without anyone having to know what it is called.
    fn shown<'a>(&'a self, section: Section, query: &str) -> Vec<&'a PaletteItem> {
        self.items
            .iter()
            .filter(|item| item.section == section)
            .filter(|item| {
                query.is_empty()
                    || item.name.to_ascii_lowercase().contains(query)
                    || item.description.to_ascii_lowercase().contains(query)
            })
            .collect()
    }

    // --- the scroll bar ---------------------------------------------------

    /// The panel's overlay scroll indicator, as it now stands.
    fn bar(&self) -> Scrollbar {
        Scrollbar::for_handle(PANEL_SCROLLBAR, ScrollbarAxis::Vertical, &self.scroll)
            .fade(self.scrollbar.fade())
    }

    /// Puts the bar up whenever the list has moved.
    fn watch_scroll(&mut self, cx: &mut Context<Self>) {
        let scrolled = scrolled(&self.scroll, ScrollbarAxis::Vertical);
        if let Some(epoch) = self.scrollbar.moved(scrolled) {
            hide_later(epoch, cx, move |panel: &mut Self| {
                Some(&mut panel.scrollbar)
            });
        }
    }

    /// Scrolls the list when its thumb is dragged.
    pub fn drag_scrollbar(&mut self, event: &DragMoveEvent<DraggedThumb>, cx: &mut Context<Self>) {
        let Some(progress) = self.bar().dragged(event, cx) else {
            return;
        };
        self.scrollbar.hold();
        scroll_to(&self.scroll, ScrollbarAxis::Vertical, progress);
        cx.notify();
    }

    /// Lets go of the thumb, and starts its clock again.
    pub fn release_scrollbar(&mut self, cx: &mut Context<Self>) {
        if let Some(epoch) = self.scrollbar.release() {
            hide_later(epoch, cx, move |panel: &mut Self| {
                Some(&mut panel.scrollbar)
            });
            cx.notify();
        }
    }

    /// Puts the bar up while the pointer rests on the edge it rides.
    fn hover_scrollbar(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            if self.scrollbar.hover_enter() {
                cx.notify();
            }
            return;
        }
        let Some(epoch) = self.scrollbar.hover_leave() else {
            return;
        };
        hide_now(self, epoch, cx, move |panel: &mut Self| {
            Some(&mut panel.scrollbar)
        });
    }
}

impl EventEmitter<PaletteEvent> for VariablePalette {}

impl Focusable for VariablePalette {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for VariablePalette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        self.watch_scroll(cx);
        let mono = app_settings::editor_font(cx);
        let query = self.query(cx);

        let mut groups = Vec::new();
        for section in Section::ALL {
            let shown = self.shown(section, &query);
            if shown.is_empty() {
                continue;
            }
            let rows: Vec<_> = shown
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let insert = item.click.clone();
                    let this = cx.entity();
                    div()
                        .id((section.id(), index))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .px(px(6.))
                        .py(px(2.))
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.surface_hover))
                        .tooltip(tooltip_label(item.description.clone()))
                        .on_click(move |_, _window, cx| {
                            this.update(cx, |_, cx| {
                                cx.emit(PaletteEvent::Insert(insert.clone()));
                            });
                        })
                        .child(
                            div()
                                .flex_none()
                                .font_family(mono.clone())
                                .text_size(px(11.))
                                .text_color(theme.text)
                                .child(item.name.clone()),
                        )
                        .children(item.example.clone().map(|example| {
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(format!("\u{2192} {example}")))
                        }))
                })
                .collect();

            groups.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(
                        div()
                            .px(px(6.))
                            .pt(px(6.))
                            .text_size(px(10.))
                            .text_color(theme.text_muted)
                            .child(section.title()),
                    )
                    .children(rows),
            );
        }

        let empty = groups.is_empty().then(|| {
            div()
                .p(px(10.))
                .text_size(px(11.))
                .text_color(theme.text_muted)
                .child(ts!("palette.no_match"))
        });

        let bar = self
            .bar()
            .on_hover(cx.listener(|panel, hovered: &bool, _window, cx| {
                panel.hover_scrollbar(*hovered, cx);
            }));

        div()
            .key_context("VariablePalette")
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .bg(theme.surface)
            .border_l_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(px(6.))
                    .p(px(8.))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.text)
                            .child(ts!("palette.title")),
                    )
                    .child(self.search.clone()),
            )
            .child(
                // The bar hangs off this wrapper rather than the panel root:
                // the thumb is measured against the scrolling box, so the box
                // the strip spans has to be that one — hung off the root it
                // would ride over the title and the filter above, and stop
                // short of the bottom by their height.
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_grow_1()
                    .min_h_0()
                    .child(
                        div()
                            .id(PANEL_SCROLL)
                            .track_scroll(&self.scroll)
                            .flex()
                            .flex_col()
                            .flex_grow_1()
                            .min_h_0()
                            .pb(px(8.))
                            .overflow_y_scroll()
                            .restrict_scroll_to_axis()
                            .children(groups)
                            .children(empty),
                    )
                    .children(bar.render(&theme)),
            )
    }
}
