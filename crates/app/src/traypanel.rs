//! The notification-area icon's panel (#1365): a click on the icon shows, over it, what is
//! downloading (each download's name, its progress bar, how much has come of how much, its speed
//! and time left) with Pause/Resume and Cancel, and a button that opens the main window. A double
//! click on the icon opens the window straight away, as the click did before; the icon's menu
//! keeps "Show downloads on click", unticked for lists saved before the panel (the click keeps
//! opening the window there), ticked for new ones (`Settings::tray_panel`).
//!
//! A borderless window of ours, as the drop target is ([`crate::chrome::popup_viewport`]): it
//! takes the focus, and closes once it loses it (a click elsewhere), on Escape, or on a second
//! click on the icon. Drawn with the progress window's controls and colours, in every theme.

use std::time::Duration;

use egui::{Pos2, Rect, Vec2, ViewportId, pos2, vec2};

use crate::model::{self, Item, Status};
use crate::winui::{self, ButtonKind, r};

/// The window's title (never shown: no title bar; Windows styles it by this name).
const TITLE: &str = "mdm-tray-panel";

/// The panel's width, and the heights of its parts, in points (English layout).
const W: f32 = 380.0;
const HEAD: f32 = 32.0;
const ROW: f32 = 60.0;
/// The line saying there is nothing to show, or how many more there are.
const NOTE: f32 = 30.0;
const FOOT: f32 = 44.0;
/// The most downloads listed; the others are counted under them.
const MOST: usize = 5;
/// Between the icon and the panel.
const GAP: f32 = 8.0;
/// Pause/Resume and Cancel: their width in English (wider when a translation needs it).
const BUTTON_W: f32 = 70.0;

/// What the panel asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Pause(u64),
    Resume(u64),
    /// Stop the download (it stays in the list), as the progress window's Cancel does.
    Cancel(u64),
    /// The main window, and the panel closed.
    OpenMain,
    /// Closed: it lost the focus, or Escape.
    Close,
}

/// The panel while it shows.
pub struct Panel {
    /// Where the icon was clicked (screen, points).
    at: Pos2,
    /// Downloads paused from the panel: still listed, with Resume, while it shows.
    paused: Vec<u64>,
    /// Its window had the focus: losing it from then on closes the panel.
    focused: bool,
}

/// One download as the panel lists it.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub id: u64,
    pub name: String,
    /// How far, 0 to 1; None without a size.
    pub fraction: Option<f32>,
    /// "12.000 MB of 200.000 MB · 1.500 MB/sec · 2 min 6 sec" (as far as known).
    pub line: String,
    /// Running (Pause), or paused from the panel (Resume).
    pub running: bool,
}

impl Panel {
    /// A panel for a click on the icon at `at` (screen, points).
    pub fn new(at: Pos2) -> Panel {
        Panel {
            at,
            paused: Vec::new(),
            focused: false,
        }
    }

    /// Paused from the panel: kept in its list, with Resume.
    pub fn paused(&mut self, id: u64) {
        if !self.paused.contains(&id) {
            self.paused.push(id);
        }
    }

    /// Resumed or cancelled: listed again only while it runs.
    pub fn forget(&mut self, id: u64) {
        self.paused.retain(|p| *p != id);
    }

    /// What it lists: every running download, then those paused from it (not finished since).
    pub fn rows(&self, items: &[Item]) -> Vec<Row> {
        let running = items.iter().filter(|i| i.live.is_some());
        let paused = self.paused.iter().filter_map(|id| {
            items
                .iter()
                .find(|i| i.id == *id && i.live.is_none() && i.status != Status::Complete)
        });
        running.chain(paused).map(row).collect()
    }

    /// Show the panel; what was asked in it this frame.
    pub fn show(&mut self, ctx: &egui::Context, items: &[Item]) -> Vec<Event> {
        let rows = self.rows(items);
        let size = vec2(W, height(rows.len()));
        let at = place(self.at, size, crate::scale::work_area(ctx, self.at));
        let builder = crate::chrome::popup_viewport(TITLE)
            .with_position(at)
            .with_inner_size(size);
        let vid = ViewportId::from_hash_of(TITLE);
        let mut events = Vec::new();
        let mut styled = false;
        ctx.show_viewport_immediate(vid, builder, |ui, _| {
            styled = crate::chrome::style_popup(TITLE, crate::chrome::PopupLook::Floating);
            crate::dialogs::keyboard_to_click(ui.ctx());
            let o = ui.max_rect().min;
            let _surface = winui::Surface::enter(o.x, o.x + W);
            let p = ui.painter().clone();
            let whole = Rect::from_min_size(o, size);
            p.rect_filled(whole, 0.0, winui::BG());
            p.rect_stroke(
                whole,
                0.0,
                egui::Stroke::new(1.0, crate::theme::colors().drop_edge),
                egui::StrokeKind::Inside,
            );
            draw(ui, &p, o, &rows, &mut events);
            // Escape, or the system asked it to close (Alt+F4, a test closing its windows).
            if winui::escape_pressed(ui) || ui.ctx().input(|i| i.viewport().close_requested()) {
                events.push(Event::Close);
            }
        });
        if styled {
            crate::chrome::show_popup(ctx, vid);
            ctx.send_viewport_cmd_to(vid, egui::ViewportCommand::Focus);
        }
        // Closed once it loses the focus it had (a click elsewhere, the taskbar's included). Not
        // in an off-screen test copy, whose windows are kept from the foreground (win32.rs).
        match ctx
            .input_for(vid, |i| i.viewport().focused)
            .filter(|_| !crate::offscreen())
        {
            Some(true) => self.focused = true,
            Some(false) if self.focused => events.push(Event::Close),
            _ => {}
        }
        // Its figures move: drawn again twice a second while it shows.
        // (It is drawn in the main window's pass, which a hidden or minimized main window runs
        // only when asked: `App::tray_panel` asks.)
        ctx.request_repaint_after(Duration::from_millis(500));
        events
    }
}

/// The panel's height with `rows` downloads.
fn height(rows: usize) -> f32 {
    let listed = rows.min(MOST);
    let note = if rows == 0 || rows > MOST { NOTE } else { 0.0 };
    HEAD + listed as f32 * ROW + note + FOOT
}

/// Where the panel goes for a click at `at`: centred over it and above it, inside the screen's
/// work area (`area`: the screen without the taskbar), so it sits beside the taskbar wherever
/// that is (at the bottom, the top or a side). Where the work area isn't known (Linux, macOS),
/// below the click when there is no room above it (macOS's menu bar, a panel at the top).
/// Native Wayland places windows itself: there it goes where the compositor puts it.
pub fn place(at: Pos2, size: Vec2, area: Option<Rect>) -> Pos2 {
    let wanted = pos2(at.x - size.x / 2.0, at.y - size.y - GAP);
    match area {
        Some(a) => crate::scale::keep_inside(wanted, size, a),
        None => pos2(
            wanted.x.max(0.0),
            if wanted.y < 0.0 { at.y + GAP } else { wanted.y },
        ),
    }
    .round()
}

/// `item` as a row: its progress as the progress window gives it.
fn row(item: &Item) -> Row {
    let done = model::size3(item.downloaded);
    let mut parts = vec![match item.size.filter(|s| *s > 0) {
        Some(size) => crate::i18n::fmt(
            crate::i18n::t("{done} of {size}"),
            &[("done", &done), ("size", &model::size3(size))],
        ),
        None => done,
    }];
    let running = item.live.is_some();
    if running {
        parts.push(model::rate(item.rate()));
        let left = item.time_left();
        if !left.is_empty() {
            parts.push(left);
        }
    }
    Row {
        id: item.id,
        name: item.file_name.clone(),
        fraction: item.fraction().map(|f| f as f32),
        line: parts.join(" · "),
        running,
    }
}

/// The panel's contents at `o`: its title, each download, and the button to the main window.
fn draw(ui: &mut egui::Ui, p: &egui::Painter, o: Pos2, rows: &[Row], events: &mut Vec<Event>) {
    // i18n: the notification-area icon's panel (#1365): its title, over the downloads now running (also the tray menu's submenu)
    let title = crate::i18n::t("Downloads in progress");
    winui::text_font(
        p,
        winui::tahoma_bold(),
        o.x + 12.0,
        o.y + 20.0,
        title,
        winui::TEXT(),
    );
    let mut y = HEAD;
    winui::etched_h(p, o.x + 8.0, o.y + y - 2.0, W - 16.0);
    for row in rows.iter().take(MOST) {
        draw_row(ui, p, o, y, row, events);
        y += ROW;
        winui::etched_h(p, o.x + 8.0, o.y + y - 2.0, W - 16.0);
    }
    let note = if rows.is_empty() {
        // i18n: the notification-area icon's panel (#1365) when no download is running
        Some(crate::i18n::t("No downloads are running").to_owned())
    } else if rows.len() > MOST {
        Some(crate::i18n::fmt(
            // i18n: the notification-area icon's panel (#1365), under the downloads it lists: how many others are running; {count} a number
            crate::i18n::t("And {count} more"),
            &[("count", &(rows.len() - MOST).to_string())],
        ))
    } else {
        None
    };
    if let Some(note) = note {
        winui::fit(
            ui,
            p,
            o.x + 12.0,
            o.y + y + 19.0,
            &note,
            winui::MUTED(),
            o.x + W - 12.0,
            // Its own id for each of the two notes (nothing running, or how many more).
            ("tray-panel-note", rows.is_empty()),
        );
        y += NOTE;
    }
    // i18n: the notification-area icon's panel (#1365): its button that opens the main window (also the tray menu's first item)
    let open = crate::i18n::mark("Open Mister Download Manager");
    let rc = winui::button_rect_for(
        p,
        r(o, W - 12.0 - 200.0, y + 11.0, 200.0, 23.0),
        open,
        W - 24.0,
    );
    if winui::button(ui, rc, open, ButtonKind::Default, false, None).clicked() {
        events.push(Event::OpenMain);
    }
}

/// One download's row, from `y` down: its name, its bar with Pause (or Resume) and Cancel beside
/// it, and how far it is under them; read to screen readers as one line.
fn draw_row(
    ui: &mut egui::Ui,
    p: &egui::Painter,
    o: Pos2,
    y: f32,
    row: &Row,
    events: &mut Vec<Event>,
) {
    let buttons_x = W - 12.0 - 2.0 * BUTTON_W - 6.0;
    winui::fit(
        ui,
        p,
        o.x + 12.0,
        o.y + y + 16.0,
        &crate::i18n::isolate(&row.name),
        winui::TEXT(),
        o.x + W - 12.0,
        ("tray-panel-name", row.id),
    );
    let bar = r(o, 12.0, y + 24.0, buttons_x - 12.0 - 10.0, 14.0);
    winui::meter(p, bar, row.fraction.unwrap_or(0.0));
    let (toggle, event) = if row.running {
        (crate::i18n::mark("Pause"), Event::Pause(row.id))
    } else {
        (crate::i18n::mark("Resume"), Event::Resume(row.id))
    };
    let cancel = crate::i18n::mark("Cancel");
    ui.push_id(("tray-panel-row", row.id), |ui| {
        let at = r(o, buttons_x, y + 20.0, BUTTON_W, 21.0);
        if winui::button(ui, at, toggle, ButtonKind::Normal, false, None).clicked() {
            events.push(event);
        }
        let at = r(o, buttons_x + BUTTON_W + 6.0, y + 20.0, BUTTON_W, 21.0);
        if winui::button(ui, at, cancel, ButtonKind::Normal, false, None).clicked() {
            events.push(Event::Cancel(row.id));
        }
    });
    winui::fit(
        ui,
        p,
        o.x + 12.0,
        o.y + y + 54.0,
        &row.line,
        winui::MUTED(),
        o.x + W - 12.0,
        ("tray-panel-line", row.id),
    );
    // The row for screen readers: its name, how far, and the line under the bar.
    let whole = r(o, 0.0, y, W, ROW);
    let resp = ui.interact(
        whole,
        ui.scope_id().with(("tray-panel-read", row.id)),
        egui::Sense::hover(),
    );
    let percent = row
        .fraction
        .map(|f| format!(", {}%", (f * 100.0).floor() as u32))
        .unwrap_or_default();
    winui::expose(
        &resp,
        egui::Role::ListItem,
        &format!("{}{percent}, {}", row.name, row.line),
        true,
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Over the icon, inside the work area: above a taskbar at the bottom, under one at the top,
    /// beside one at a side; the screen's edge never cuts it.
    #[test]
    fn the_panel_sits_beside_the_taskbar_wherever_it_is() {
        let size = vec2(380.0, 200.0);
        // A 1920 × 1080 screen, the taskbar at the bottom (40 tall): the icon near the right.
        let bottom = Rect::from_min_max(pos2(0.0, 0.0), pos2(1920.0, 1040.0));
        let p = place(pos2(1800.0, 1060.0), size, Some(bottom));
        assert_eq!(p, pos2(1920.0 - 380.0, 1040.0 - 200.0));
        // In the middle of the taskbar: centred over the click, above the taskbar.
        let p = place(pos2(960.0, 1060.0), size, Some(bottom));
        assert_eq!(p, pos2(770.0, 840.0));
        // The taskbar at the top: under it.
        let top = Rect::from_min_max(pos2(0.0, 40.0), pos2(1920.0, 1080.0));
        assert_eq!(place(pos2(1800.0, 20.0), size, Some(top)).y, 40.0);
        // At the left: beside it.
        let left = Rect::from_min_max(pos2(60.0, 0.0), pos2(1920.0, 1080.0));
        assert_eq!(place(pos2(30.0, 1000.0), size, Some(left)).x, 60.0);
        // No work area (macOS's menu bar): under the click when there is no room above.
        assert_eq!(place(pos2(1500.0, 12.0), size, None), pos2(1310.0, 20.0));
        assert_eq!(place(pos2(100.0, 900.0), size, None), pos2(0.0, 692.0));
    }

    /// Room for the downloads listed (five at most), and a line when none runs or more do.
    #[test]
    fn the_panel_grows_with_its_downloads_up_to_five() {
        assert_eq!(height(0), HEAD + NOTE + FOOT);
        assert_eq!(height(1), HEAD + ROW + FOOT);
        assert_eq!(height(5), HEAD + 5.0 * ROW + FOOT);
        assert_eq!(height(9), HEAD + 5.0 * ROW + NOTE + FOOT);
    }
}
