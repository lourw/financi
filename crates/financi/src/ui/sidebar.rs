//! The window sidebar: nav items and the DB-path footer (3.62).

use super::*;
use crate::ui::widgets::*;

/// The left inset (from the sidebar's content edge) shared by the nav items
/// and the section labels (3.13). The gpui-kit nav row brings its own 8px
/// left padding, which sat the items right of the single column the app
/// name, the section labels and the footer share
/// (`docs/ui-conventions.md`, *Sidebar*); both now use this one inset.
/// A collapsed row keeps the component's own padding (see
/// [`NavItem::collapsed`]), which centers the icon either way.
pub(crate) const NAV_ROW_INSET: f32 = 0.;

/// A sidebar entry: a clickable item or a section label (the gpui-kit
/// [`Sidebar`] takes one item type, and the layout mixes items with the
/// *Organize* / *Data* section headers).
#[derive(Clone)]
pub(super) enum NavItem {
    Item(Box<SidebarMenuItem>),
    Section(SharedString),
}

impl Collapsible for NavItem {
    fn collapsed(self, collapsed: bool) -> Self {
        match self {
            // The expanded row pins its left inset (3.13); a collapsed row
            // goes back to the component's own padding so the icon centers
            // in the rail exactly as the component lays it out.
            Self::Item(item) => {
                let item = item.collapsed(collapsed);
                let item = if collapsed {
                    item.pl_2()
                } else {
                    item.pl(px(NAV_ROW_INSET))
                };
                Self::Item(Box::new(item))
            }
            section => section,
        }
    }

    fn is_collapsed(&self) -> bool {
        match self {
            Self::Item(item) => item.is_collapsed(),
            Self::Section(_) => false,
        }
    }
}

impl SidebarItem for NavItem {
    fn render(
        self,
        id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        match self {
            Self::Item(item) => item.render(id, window, cx).into_any_element(),
            Self::Section(label) => h_flex()
                .flex_shrink_0()
                .h_8()
                .items_center()
                .px(px(NAV_ROW_INSET))
                .text_xs()
                .text_color(cx.theme().sidebar_foreground.opacity(0.7))
                .child(label.to_uppercase())
                .into_any_element(),
        }
    }
}

// ---------------------------------------------------------------------------
// Transaction table

impl Shell {
    fn nav_item(
        &self,
        label: &'static str,
        icon: IconName,
        page: Page,
        cx: &mut Context<Self>,
    ) -> SidebarMenuItem {
        SidebarMenuItem::new(label)
            .icon(icon)
            // Drop the component's own left padding (3.13): the icon sits in
            // the same column as the section labels, the header and footer.
            .pl(px(NAV_ROW_INSET))
            .active(self.page == page)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.go_page(page, cx);
            }))
    }

    /// The sidebar: header, grouped items with count pills, footer
    /// (ui-conventions, *Sidebar*).
    pub(super) fn sidebar(&mut self, cx: &mut Context<Self>) -> Sidebar<NavItem> {
        let theme = cx.theme();
        let (muted, pill_bg, pill_fg) = (
            theme.muted_foreground,
            theme.primary,
            theme.primary_foreground,
        );
        let dashboard = self.nav_item("Dashboard", IconName::LayoutDashboard, Page::Dashboard, cx);
        let to_categorize = self.nav_item(
            "To categorize",
            IconName::ListChecks,
            Page::ToCategorize,
            cx,
        );
        let to_categorize = if self.uncategorized_visible > 0 {
            let count = self.uncategorized_visible;
            to_categorize.suffix(move |_, _| {
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(20.))
                    .px_1()
                    .rounded(px(8.))
                    .text_xs()
                    .bg(pill_bg)
                    .text_color(pill_fg)
                    .child(flow::count(count))
            })
        } else {
            to_categorize
        };
        let categories = self.nav_item("Categories", IconName::Tags, Page::Categories, cx);
        // 3.28: the rules live on their own page now, after Categories.
        let rules_nav = self.nav_item("Rules", IconName::Wrench, Page::Rules, cx);
        // Transfers pill: the number of open reviews (4.2), hidden at 0.
        let transfers = self.nav_item("Transfers", IconName::ArrowLeftRight, Page::Transfers, cx);
        let transfers = if !self.transfer_reviews.is_empty() {
            let count = self.transfer_reviews.len() as i64;
            transfers.suffix(move |_, _| {
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(20.))
                    .px_1()
                    .rounded(px(8.))
                    .text_xs()
                    .bg(pill_bg)
                    .text_color(pill_fg)
                    .child(flow::count(count))
            })
        } else {
            transfers
        };
        let imports = self.nav_item("Imports", IconName::Download, Page::Imports, cx);

        Sidebar::new("nav")
            .w(px(SIDEBAR_W))
            .collapsible(false)
            .header(div().font_weight(FontWeight::SEMIBOLD).child("financi"))
            .child(NavItem::Item(Box::new(dashboard)))
            .child(NavItem::Section("Organize".into()))
            .child(NavItem::Item(Box::new(to_categorize)))
            .child(NavItem::Item(Box::new(categories)))
            .child(NavItem::Item(Box::new(rules_nav)))
            .child(NavItem::Item(Box::new(transfers)))
            .child(NavItem::Section("Data".into()))
            .child(NavItem::Item(Box::new(imports)))
            .footer(
                v_flex()
                    .w_full()
                    .gap_1()
                    .text_xs()
                    .text_color(muted)
                    .child(format!("{} transactions", flow::count(self.total_visible)))
                    .child(
                        // 3.38: the path ellipsizes at the sidebar's width;
                        // hovering shows the full path prefixed "Database: "
                        // (ui-conventions, *Sidebar*).
                        div()
                            .id("sidebar:db-path")
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .debug_selector({
                                let path = self.db_path.display().to_string();
                                move || format!("sidebar:db-path:{path}")
                            })
                            .tooltip({
                                let full = format!("Database: {}", self.db_path.display());
                                move |window, cx| {
                                    let tip = full.clone();
                                    Tooltip::element(move |_, _| {
                                        div()
                                            .debug_selector({
                                                let s = tip.clone();
                                                move || format!("sidebar:db-path-tooltip:{s}")
                                            })
                                            .child(tip.clone())
                                    })
                                    .build(window, cx)
                                }
                            })
                            .child(self.db_path.display().to_string()),
                    )
                    .child({
                        // 3.39: the build version as the footer's last line,
                        // e.g. "v0.17.0-3-gabc1234-dirty" (plan.md → *Show
                        // the version subtly in the UI*). Ellipsized with a
                        // hover tooltip, like the path above (3.38).
                        let label = crate::version::version_label();
                        div()
                            .id("sidebar:version")
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .debug_selector({
                                let s = label.clone();
                                move || format!("sidebar:version-text:{s}")
                            })
                            .tooltip({
                                let label = label.clone();
                                move |window, cx| {
                                    let tip = label.clone();
                                    Tooltip::element(move |_, _| {
                                        div()
                                            .debug_selector({
                                                let s = tip.clone();
                                                move || format!("sidebar:version-tooltip:{s}")
                                            })
                                            .child(tip.clone())
                                    })
                                    .build(window, cx)
                                }
                            })
                            .child(label)
                    }),
            )
    }
}
