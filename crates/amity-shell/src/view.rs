//! What each shell surface shows.

use amity_ui::{
    kit,
    tokens::{self, typography},
};
use iced::{
    widget::{
        button, center, column, container, image, mouse_area, opaque, row, space, stack, text,
        text::Wrapping, text_input,
    },
    Alignment, ContentFit, Element, Length,
};

use crate::shell::{
    application_name, ellipsize, initials, page_start, task_order, Message, Shell, LAUNCHER_ROWS, QUERY_ID,
};

/// Width of the launcher and switcher panels in logical pixels.
const PANEL_WIDTH: f32 = 672.0;
/// Side of the tile that stands in for an application icon.
const TILE: f32 = 34.0;

fn label<'a>(
    content: impl text::IntoFragment<'a>,
    style: tokens::TypeStyle,
    color: tokens::Color,
) -> text::Text<'a> {
    text(content)
        .size(style.size)
        .line_height(text::LineHeight::Absolute(style.line_height.into()))
        .font(kit::font(style))
        .color(kit::color(color))
        .wrapping(Wrapping::None)
}

fn tile<'a>(name: &str, active: bool) -> Element<'a, Message> {
    container(label(initials(name), typography::CAPTION, tokens::text::TEXT))
        .center(TILE)
        .style(move |_| kit::monogram(active))
        .into()
}

fn key_cap<'a>(name: &'a str) -> Element<'a, Message> {
    container(label(name, typography::CAPTION, tokens::text::BODY))
        .padding([3.0, 8.0])
        .style(|_| kit::key_cap())
        .into()
}

/// The desktop background: the wallpaper, or the ground colour without one.
pub fn background(shell: &Shell) -> Element<'_, Message> {
    match &shell.wallpaper {
        Some(wallpaper) => image(wallpaper.clone())
            .content_fit(ContentFit::Cover)
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
        None => container(space())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::background(kit::color(tokens::surface::GROUND)))
            .into(),
    }
}

/// The taskbar: launcher button, one entry per window, launcher hint, clock.
pub fn taskbar(shell: &Shell) -> Element<'_, Message> {
    let launcher = button(center(label("A", typography::WINDOW_TITLE, tokens::text::TEXT)))
        .width(40.0)
        .height(40.0)
        .padding(0)
        .on_press(Message::ToggleLauncher)
        .style(|_, status| kit::primary_button(status));

    let separator = container(space())
        .width(1.0)
        .height(28.0)
        .style(|_| container::background(kit::hairline()));

    let mut tasks = row![].spacing(4).align_y(Alignment::Center);
    for window in task_order(&shell.windows) {
        let active = shell.focused == Some(window.id) && !window.minimized;
        let name = application_name(&window.app_id, &shell.catalogue);
        let text_color = if window.minimized {
            tokens::text::MUTED
        } else {
            tokens::text::TEXT
        };
        let entry = column![
            row![
                tile(&name, active),
                label(ellipsize(&name, 18), typography::CONTROL, text_color),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            container(space())
                .width(if active { 18.0 } else { 6.0 })
                .height(3.0)
                .style(move |_| kit::indicator(active)),
        ]
        .spacing(3)
        .align_x(Alignment::Center);
        tasks = tasks.push(
            button(entry)
                .padding([5.0, 8.0])
                .on_press(Message::Task(window.id))
                .style(move |_, status| kit::taskbar_button(status, active)),
        );
    }

    let hint = button(
        row![
            label("Open an app", typography::BODY, tokens::text::MUTED),
            key_cap("Super"),
            key_cap("Space"),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding([8.0, 12.0])
    .on_press(Message::ToggleLauncher)
    .style(|_, status| kit::taskbar_button(status, false));

    let clock = column![
        label(
            shell.now.format("%H:%M").to_string(),
            typography::CONTROL,
            tokens::text::TEXT
        ),
        label(
            shell.now.format("%a %-d %b").to_string(),
            typography::CAPTION,
            tokens::text::MUTED
        ),
    ]
    .align_x(Alignment::End);

    container(
        row![
            launcher,
            separator,
            tasks,
            space().width(Length::Fill),
            hint,
            space().width(4.0),
            clock,
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding([0.0, 14.0])
    .width(Length::Fill)
    .height(Length::Fill)
    .align_y(Alignment::Center)
    .clip(true)
    .style(|_| kit::taskbar())
    .into()
}

/// The launcher: a search field over the installed applications.
pub fn launcher(shell: &Shell) -> Element<'_, Message> {
    let search = text_input("Search apps", &shell.query)
        .id(QUERY_ID)
        .on_input(Message::Query)
        .on_submit(Message::Submit)
        .padding([14.0, 18.0])
        .size(18.0)
        .font(kit::font(typography::CONTROL))
        .style(|_, status| kit::search_field(status));

    let mut results = column![].spacing(4);
    if shell.results.is_empty() {
        results = results.push(
            container(label(
                "No matching applications",
                typography::BODY,
                tokens::text::MUTED,
            ))
            .padding([14.0, 12.0]),
        );
    }
    let first = page_start(shell.selected, LAUNCHER_ROWS);
    for row_index in first..shell.results.len().min(first + LAUNCHER_ROWS) {
        let Some(entry) = shell.result(row_index) else {
            continue;
        };
        let selected = row_index == shell.selected;
        let mut names = column![label(
            ellipsize(&entry.name, 60),
            typography::CONTROL,
            tokens::text::TEXT
        )];
        if let Some(description) = entry.generic_name.as_ref().or(entry.comment.as_ref()) {
            names = names.push(label(
                ellipsize(description, 80),
                typography::CAPTION,
                tokens::text::MUTED,
            ));
        }
        results = results.push(
            button(
                row![tile(&entry.name, selected), names]
                    .spacing(12)
                    .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([8.0, 10.0])
            .on_press(Message::Activate(row_index))
            .style(move |_, status| kit::list_row(status, selected)),
        );
    }

    let mut footer = row![
        key_cap("Enter"),
        label("Open", typography::CAPTION, tokens::text::MUTED),
        space().width(8.0),
        key_cap("Esc"),
        label("Close", typography::CAPTION, tokens::text::MUTED),
        space().width(Length::Fill),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if shell.results.len() > LAUNCHER_ROWS {
        let last = shell.results.len().min(first + LAUNCHER_ROWS);
        footer = footer.push(label(
            format!("{}–{} of {}", first + 1, last, shell.results.len()),
            typography::CAPTION,
            tokens::text::MUTED,
        ));
    }

    let mut content = column![search, results].spacing(16);
    if let Some(error) = &shell.launch_error {
        content = content.push(label(error.clone(), typography::BODY, tokens::accent::ORCHID));
    }
    content = content.push(footer);

    let panel = container(content)
        .width(PANEL_WIDTH)
        .padding(24)
        .style(|_| kit::overlay_panel());

    let scrim = mouse_area(
        container(space())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| kit::scrim()),
    )
    .on_press(Message::CloseLauncher);

    stack![
        scrim,
        container(opaque(panel))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .padding(iced::Padding::default().top(120.0)),
    ]
    .into()
}

/// The window switcher overlay shown while Alt is held.
pub fn switcher(shell: &Shell) -> Element<'_, Message> {
    let Some(state) = &shell.switcher else {
        return space().into();
    };
    let mut rows = column![].spacing(4);
    for (index, id) in state.candidates.iter().enumerate() {
        let Some(window) = shell.windows.iter().find(|window| window.id == *id) else {
            continue;
        };
        let selected = index == state.selected;
        let name = application_name(&window.app_id, &shell.catalogue);
        let title = if window.title.is_empty() {
            name.clone()
        } else {
            window.title.clone()
        };
        let status = if window.minimized { "Minimized" } else { "Open" };
        rows = rows.push(
            container(
                row![
                    tile(&name, selected),
                    column![
                        label(ellipsize(&title, 56), typography::CONTROL, tokens::text::TEXT),
                        label(name, typography::CAPTION, tokens::text::MUTED),
                    ],
                    space().width(Length::Fill),
                    label(status, typography::CAPTION, tokens::text::MUTED),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([8.0, 10.0])
            .style(move |_| {
                let row = kit::list_row(button::Status::Active, selected);
                container::Style {
                    background: row.background,
                    border: row.border,
                    ..container::Style::default()
                }
            }),
        );
    }
    let hint = row![
        key_cap("Tab"),
        label("Next", typography::CAPTION, tokens::text::MUTED),
        space().width(8.0),
        key_cap("Esc"),
        label("Cancel", typography::CAPTION, tokens::text::MUTED),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    center(
        container(column![rows, hint].spacing(16))
            .width(PANEL_WIDTH)
            .padding(24)
            .style(|_| kit::overlay_panel()),
    )
    .into()
}
