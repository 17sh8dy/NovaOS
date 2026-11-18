use super::*;
use std::vec;

const W: usize = 800;
const H: usize = 600;
const RED: Color = rgb(200, 0, 0);
const GREEN: Color = rgb(0, 200, 0);

/// Two windows: 1 at (100, 100) 200x100 content, 2 at (250, 150) 200x100 content (on top).
fn desk() -> Desktop {
    let mut d = Desktop::new(W, H, "test");
    d.add_window(1, "One", (200, 100), (100, 100), RED);
    d.add_window(2, "Two", (200, 100), (250, 150), GREEN);
    d.set_menu(vec![(String::from("One"), 1), (String::from("Two"), 2)]);
    d.take_damage();
    d
}

fn click(d: &mut Desktop, x: i32, y: i32) -> Vec<Event> {
    d.pointer_moved(x, y);
    let mut ev = d.press();
    ev.extend(d.release());
    ev
}

fn center(r: Rect) -> (i32, i32) {
    (r.x + r.w / 2, r.y + r.h / 2)
}

#[test]
fn metrics_and_layout() {
    let d = desk();
    assert_eq!(d.m.u, 1);
    assert_eq!(d.taskbar(), Rect::new(0, H as i32 - d.m.taskbar_h, W as i32, d.m.taskbar_h));
    let c = d.content_rect(1).unwrap();
    assert_eq!((c.x, c.y, c.w, c.h), (101, 101 + d.m.title_h, 200, 100));
    assert_eq!(d.window(1).unwrap().rect.h, 100 + d.m.title_h + 2);
    assert_eq!(Metrics::for_screen(1600).u, 2);
    assert_eq!(d.task_buttons().len(), 2);
}

#[test]
fn newest_window_is_on_top_and_focused() {
    let d = desk();
    assert_eq!(d.stacking(), vec![1, 2]);
    assert_eq!(d.focused(), Some(2));
    // The overlap belongs to window 2.
    assert!(matches!(d.hit(260, 200), Hit::Content(2, ..)));
    assert!(matches!(d.hit(150, 200), Hit::Content(1, ..)));
}

#[test]
fn hit_testing() {
    let d = desk();
    let w = d.window(2).unwrap().rect;
    assert_eq!(d.hit(w.x + 20, w.y + 10), Hit::Title(2));
    let close = d.close_rect(d.window(2).unwrap());
    assert_eq!(d.hit(close.x + 2, close.y + 2), Hit::Close(2));
    let min = d.minimize_rect(d.window(2).unwrap());
    assert_eq!(d.hit(min.x + 2, min.y + 2), Hit::Minimize(2));
    let c = d.content_rect(2).unwrap();
    assert_eq!(d.hit(c.x + 5, c.y + 7), Hit::Content(2, 5, 7));
    assert_eq!(d.hit(w.x, w.y + 50), Hit::Frame(2));
    assert_eq!(d.hit(700, 50), Hit::Desktop);
    let l = d.launcher();
    assert_eq!(d.hit(l.x + 1, l.y + 1), Hit::Launcher);
    let (id, b) = d.task_buttons()[1];
    assert_eq!(d.hit(b.x + 3, b.y + 3), Hit::TaskButton(id));
    assert_eq!(d.hit(W as i32 - 300, H as i32 - 2), Hit::Taskbar);
}

#[test]
fn clicking_focuses_and_raises() {
    let mut d = desk();
    let ev = click(&mut d, 120, 200); // window 1 content, not covered
    assert_eq!(ev, vec![Event::Focused(1), Event::Click(1, 19, 200 - 101 - d.m.title_h)]);
    assert_eq!(d.stacking(), vec![2, 1]);
    assert_eq!(d.focused(), Some(1));
    // Clicking the focused window again: just a click.
    let ev = click(&mut d, 120, 200);
    assert!(matches!(ev.as_slice(), [Event::Click(1, ..)]));
}

#[test]
fn dragging_moves_a_window_and_damages_both_places() {
    let mut d = desk();
    let r = d.window(2).unwrap().rect;
    d.pointer_moved(r.x + 30, r.y + 10);
    assert_eq!(d.press(), vec![Event::DragStarted(2)]);
    d.take_damage();
    d.pointer_moved(r.x + 130, r.y + 60);
    let damage = d.take_damage();
    let total = damage.iter().fold(Rect::default(), |a, r| a.union(r));
    assert!(total.contains(r.x, r.y), "the old place is repainted");
    assert!(total.contains(r.x + 100 + r.w - 1, r.y + 50 + r.h - 1), "the new place is repainted");
    assert_eq!(d.release(), vec![Event::Moved(2, r.x + 100, r.y + 50)]);
    assert_eq!(d.dragging(), None);
    // A click on the title that doesn't move the window is not a move.
    let r2 = d.window(2).unwrap().rect;
    d.pointer_moved(r2.x + 30, r2.y + 10);
    d.press();
    assert!(d.release().is_empty());
    // Moving after release doesn't drag.
    d.pointer_moved(10, 10);
    assert_eq!(d.window(2).unwrap().rect.x, r.x + 100);
}

#[test]
fn a_window_cannot_be_lost() {
    let mut d = desk();
    let r = d.window(1).unwrap().rect;
    d.pointer_moved(r.x + 30, r.y + 10);
    d.press();
    d.pointer_moved(5000, 5000);
    d.release();
    let r = d.window(1).unwrap().rect;
    assert!(r.y + d.m.title_h <= d.taskbar().y, "title bar stays above the taskbar");
    assert!(r.x <= W as i32 - 60, "some of the title stays on screen");
    d.move_window(1, -5000, -5000);
    let r = d.window(1).unwrap().rect;
    assert!(r.right() >= 60 && r.y == 0);
}

#[test]
fn close_and_minimize() {
    let mut d = desk();
    let close = d.close_rect(d.window(2).unwrap());
    assert_eq!(click(&mut d, close.x + 2, close.y + 2), vec![Event::Closed(2)]);
    assert_eq!(d.state(2), Some(State::Closed));
    assert_eq!(d.focused(), Some(1), "focus falls to the next window");
    assert_eq!(d.task_buttons().len(), 1, "closed windows leave the taskbar");
    assert!(matches!(d.hit(260, 200), Hit::Content(1, ..)) || matches!(d.hit(260, 200), Hit::Desktop));

    let min = d.minimize_rect(d.window(1).unwrap());
    assert_eq!(click(&mut d, min.x + 2, min.y + 2), vec![Event::Minimized(1)]);
    assert_eq!(d.focused(), None);
    // Its taskbar button brings it back.
    let (_, b) = d.task_buttons()[0];
    let (bx, by) = center(b);
    assert_eq!(click(&mut d, bx, by), vec![Event::Restored(1)]);
    assert_eq!((d.state(1), d.focused()), (Some(State::Open), Some(1)));
    // Clicking the button of the focused top window minimises it.
    assert_eq!(click(&mut d, bx, by), vec![Event::Minimized(1)]);
    // show() reopens a closed window on top.
    d.show(2);
    assert_eq!((d.state(2), d.focused(), d.stacking().last().copied()), (Some(State::Open), Some(2), Some(2)));
}

#[test]
fn task_button_focuses_a_covered_window() {
    let mut d = desk();
    let (_, b) = d.task_buttons()[0]; // window 1, under window 2
    let (bx, by) = center(b);
    assert_eq!(click(&mut d, bx, by), vec![Event::Focused(1)]);
    assert_eq!(d.stacking(), vec![2, 1]);
}

#[test]
fn launcher_menu() {
    let mut d = desk();
    let (lx, ly) = center(d.launcher());
    assert_eq!(click(&mut d, lx, ly), vec![Event::MenuOpened]);
    assert!(d.menu_is_open());
    let (ix, iy) = center(d.menu_item(1));
    assert_eq!(d.hit(ix, iy), Hit::MenuItem(1));
    assert_eq!(click(&mut d, ix, iy), vec![Event::MenuChose(2)]);
    assert!(!d.menu_is_open());
    // Clicking elsewhere closes it.
    click(&mut d, lx, ly);
    assert_eq!(click(&mut d, 700, 50), vec![Event::MenuClosed]);
    // The launcher toggles it.
    click(&mut d, lx, ly);
    assert_eq!(click(&mut d, lx, ly), vec![Event::MenuClosed]);
}

#[test]
fn damage_is_merged_and_clipped() {
    let mut d = desk();
    d.add_damage(Rect::new(-10, -10, 20, 20));
    d.add_damage(Rect::new(5, 5, 20, 20));
    d.add_damage(Rect::new(700, 400, 10, 10));
    d.add_damage(Rect::new(5000, 5000, 10, 10));
    let r = d.take_damage();
    assert_eq!(r.len(), 2);
    assert!(r.contains(&Rect::new(0, 0, 25, 25)));
    assert!(d.take_damage().is_empty());
    // Content damage lands in screen coordinates, clipped to the content.
    d.damage_content(1, Rect::new(190, 90, 50, 50));
    let c = d.content_rect(1).unwrap();
    assert_eq!(d.take_damage(), vec![Rect::new(c.x + 190, c.y + 90, 10, 10)]);
    d.minimize(1);
    d.take_damage();
    d.damage_content(1, Rect::new(0, 0, 10, 10));
    assert!(d.take_damage().is_empty(), "hidden windows don't damage the screen");
}

#[test]
fn many_small_damages_collapse() {
    let mut d = desk();
    for i in 0..100 {
        d.add_damage(Rect::new((i * 37) % 700, (i * 53) % 500, 3, 3));
    }
    assert!(d.take_damage().len() <= 24);
}

fn composed(d: &Desktop) -> Canvas {
    let mut back = Canvas::new(W, H, 0);
    d.compose(&mut back, d.screen());
    back
}

#[test]
fn compose_draws_windows_in_order() {
    let mut d = desk();
    d.pointer_moved(790, 10);
    let back = composed(&d);
    let c1 = d.content_rect(1).unwrap();
    let c2 = d.content_rect(2).unwrap();
    assert_eq!(back.get(c1.x + 5, c1.y + 5), Some(RED));
    assert_eq!(back.get(c2.x + 5, c2.y + 5), Some(GREEN), "window 2 is on top where they overlap");
    // Focused title bar colour, unfocused one different.
    let t2 = d.window(2).unwrap().rect;
    let t1 = d.window(1).unwrap().rect;
    assert_eq!(back.get(t2.x + 5, t2.y + 3), Some(TITLE_FOCUS));
    assert_eq!(back.get(t1.x + 5, t1.y + 3), Some(TITLE));
    // Taskbar.
    assert_eq!(back.get(W as i32 - 2, H as i32 - 2), Some(BAR));
    // Pointer: black tip.
    assert_eq!(back.get(790, 10), Some(0));
    // Shadow: darker than the wallpaper right of window 2.
    let wall = d.wallpaper.get(t2.right() + 2, t2.y + 20).unwrap();
    let shaded = back.get(t2.right() + 2, t2.y + 20).unwrap();
    assert!(shaded & 0xFF < wall & 0xFF || wall & 0xFF == 0);
}

#[test]
fn hidden_windows_are_not_drawn() {
    let mut d = desk();
    let c2 = d.content_rect(2).unwrap();
    d.close(2);
    let back = composed(&d);
    // Where only window 2 was (right part): wallpaper now.
    let (x, y) = (c2.right() - 5, c2.bottom() - 5);
    assert_eq!(back.get(x, y), d.wallpaper.get(x, y));
}

#[test]
fn compose_only_touches_the_area() {
    let d = desk();
    let mut back = Canvas::new(W, H, 0x123456);
    let area = Rect::new(10, 10, 5, 5);
    d.compose(&mut back, area);
    assert_eq!(back.get(9, 9), Some(0x123456));
    assert_eq!(back.get(15, 15), Some(0x123456));
    assert_ne!(back.get(12, 12), Some(0x123456));
}

#[test]
fn partial_compose_matches_full_compose() {
    let mut d = desk();
    d.pointer_moved(300, 160);
    let full = composed(&d);
    let mut parts = Canvas::new(W, H, 0);
    for y in (0..H as i32).step_by(97) {
        for x in (0..W as i32).step_by(131) {
            d.compose(&mut parts, Rect::new(x, y, 131, 97));
        }
    }
    assert!(full.pixels() == parts.pixels(), "drawing in pieces gives the same picture");
}

#[test]
fn content_changes_show_up() {
    let mut d = desk();
    d.content_mut(1).unwrap().fill(Rect::new(0, 0, 10, 10), 0xABCDEF);
    d.damage_content(1, Rect::new(0, 0, 10, 10));
    let back = composed(&d);
    let c = d.content_rect(1).unwrap();
    assert_eq!(back.get(c.x + 1, c.y + 1), Some(0xABCDEF));
}

#[test]
fn clock_and_note_damage_the_taskbar() {
    let mut d = desk();
    d.set_clock("12:34");
    let r = d.take_damage();
    assert!(r.iter().all(|r| r.y >= d.taskbar().y));
    d.set_clock("12:34");
    assert!(d.take_damage().is_empty(), "same text, nothing to redraw");
    d.set_note("4 CPUs");
    assert!(!d.take_damage().is_empty());
}

// ---- Lock screen, hover, Alt+Tab ----

#[test]
fn lock_screen_swallows_clicks_and_covers_windows() {
    use crate::lock::{LockKey, Mode, Outcome};
    let mut d = desk();
    let mut before = Canvas::new(W, H, 0);
    d.compose(&mut before, d.screen());
    d.show_lock(Mode::Unlock);
    assert_eq!(d.lock_shown(), Some(Mode::Unlock));
    assert_eq!(d.take_damage(), vec![d.screen()], "locking repaints everything");
    // Nothing underneath can be clicked, dragged or cycled to.
    let title = d.chrome(1).unwrap().0;
    assert!(click(&mut d, title.x + 5, title.y + 5).is_empty());
    assert_eq!(d.hit(title.x + 5, title.y + 5), Hit::Desktop);
    assert_eq!(d.cycle(), None);
    assert_eq!(d.stacking(), vec![1, 2]);
    // The window's red content is not visible while locked.
    let mut locked = Canvas::new(W, H, 0);
    d.compose(&mut locked, d.screen());
    let c = d.content_rect(1).unwrap();
    assert_eq!(before.get(c.x + 2, c.y + 2), Some(RED));
    assert_ne!(locked.get(c.x + 2, c.y + 2), Some(RED));
    // Typing only touches the card.
    d.take_damage();
    d.lock_key(LockKey::Char('a'));
    let card = crate::lock::Lock::card(&d.m, d.screen());
    assert_eq!(d.take_damage(), vec![card]);
    assert_eq!(d.lock_key(LockKey::Enter), Outcome::Check(String::from("a")));
    assert_eq!(d.lock_checked(false), Outcome::None);
    assert!(d.lock_shown().is_some());
    d.lock_key(LockKey::Enter);
    assert_eq!(d.lock_checked(true), Outcome::Close);
    assert_eq!(d.lock_shown(), None);
    d.pointer_moved(W as i32 / 2, H as i32 / 2);
    let mut after = Canvas::new(W, H, 0);
    d.compose(&mut after, d.screen());
    assert_eq!(after.pixels(), before.pixels(), "unlocking gives the same desktop back");
}

#[test]
fn locked_screen_composes_the_same_in_pieces() {
    let mut d = desk();
    d.set_clock("12:34");
    d.set_date("Tuesday 17 November 2026");
    d.show_lock(crate::lock::Mode::Unlock);
    d.lock_key(crate::lock::LockKey::Char('x'));
    let mut whole = Canvas::new(W, H, 0);
    d.compose(&mut whole, d.screen());
    let mut pieces = Canvas::new(W, H, 0);
    for y in (0..H as i32).step_by(37) {
        for x in (0..W as i32).step_by(53) {
            d.compose(&mut pieces, Rect::new(x, y, 53, 37));
        }
    }
    assert_eq!(whole.pixels(), pieces.pixels());
}

#[test]
fn buttons_light_up_under_the_pointer() {
    let mut d = desk();
    let close = d.chrome(2).unwrap().1;
    let (cx, cy) = center(close);
    let mut before = Canvas::new(W, H, 0);
    d.compose(&mut before, d.screen());
    d.pointer_moved(cx, cy);
    assert_eq!(d.hovered(), Hit::Close(2));
    let dmg = d.take_damage();
    assert!(dmg.iter().any(|r| r.intersect(&close) == close), "the close button is repainted");
    let mut lit = Canvas::new(W, H, 0);
    d.compose(&mut lit, d.screen());
    assert_eq!(lit.get(close.x + 1, close.y + 1), Some(DANGER));
    assert_ne!(before.get(close.x + 1, close.y + 1), Some(DANGER));
    // Moving within the same button repaints only the pointer.
    d.pointer_moved(cx + 1, cy);
    let ptr = d.take_damage().iter().fold(Rect::default(), |a, r| a.union(r));
    assert!(ptr.w < close.w + 40, "no big repaint for a 1-pixel move");
    // Off the button: back to normal.
    d.pointer_moved(5, 5);
    assert_eq!(d.hovered(), Hit::Desktop);
    let mut off = Canvas::new(W, H, 0);
    d.compose(&mut off, d.screen());
    assert_eq!(off.get(close.x + 1, close.y + 1), before.get(close.x + 1, close.y + 1));
}

#[test]
fn alt_tab_visits_every_window() {
    let mut d = desk();
    d.add_window(3, "Three", (100, 80), (400, 300), RED);
    assert_eq!(d.stacking(), vec![1, 2, 3]);
    d.minimize(2);
    assert_eq!(d.cycle(), Some(Event::Focused(1)));
    assert_eq!(d.focused(), Some(1));
    assert_eq!(d.cycle(), Some(Event::Restored(2)), "minimised windows come back too");
    assert_eq!(d.state(2), Some(State::Open));
    assert_eq!(d.cycle(), Some(Event::Focused(3)));
    assert_eq!(d.cycle(), Some(Event::Focused(1)));
    // Closed windows are skipped; a lone window has nowhere to go.
    d.close(2);
    d.close(3);
    assert_eq!(d.cycle(), None);
}

// ---- Maximise and resize ----

#[test]
fn maximise_fills_the_work_area_and_restores() {
    let mut d = desk();
    assert_eq!(d.maximize_button(1), None, "not resizable: no maximise button");
    d.set_resizable(1, true);
    let before = d.window(1).unwrap().rect;
    let max = d.maximize_button(1).unwrap();
    let ev = click(&mut d, max.x + 2, max.y + 2);
    assert!(ev.contains(&Event::Resized(1)));
    assert_eq!(d.window(1).unwrap().rect, d.work_area());
    assert!(d.is_maximized(1));
    assert_eq!(d.stacking().last(), Some(&1), "raised");
    let c = d.content_rect(1).unwrap();
    assert_eq!((c.w, c.h), (d.window(1).unwrap().content.width() as i32, d.window(1).unwrap().content.height() as i32));
    // No resize edges while maximised.
    let r = d.window(1).unwrap().rect;
    assert!(!matches!(d.hit(r.right() - 2, r.bottom() - 2), Hit::Resize(..)));
    let max = d.maximize_button(1).unwrap();
    assert!(click(&mut d, max.x + 2, max.y + 2).contains(&Event::Resized(1)));
    assert_eq!(d.window(1).unwrap().rect, before);
    assert!(!d.is_maximized(1));
}

#[test]
fn dragging_the_corner_resizes_with_an_outline_then_applies() {
    let mut d = desk();
    d.set_resizable(2, true);
    let r = d.window(2).unwrap().rect;
    let (x, y) = (r.right() - 2, r.bottom() - 2);
    assert_eq!(d.hit(x, y), Hit::Resize(2, EDGE_RIGHT | EDGE_BOTTOM));
    assert_eq!(d.hit(r.right() - 2, r.y + d.m.title_h + 20), Hit::Resize(2, EDGE_RIGHT));
    d.pointer_moved(x, y);
    assert!(d.press().is_empty() || d.focused() == Some(2));
    d.pointer_moved(x + 60, y + 40);
    // Nothing applied until release.
    assert_eq!(d.window(2).unwrap().rect, r);
    assert_eq!(d.release(), vec![Event::Resized(2)]);
    let n = d.window(2).unwrap();
    assert_eq!((n.rect.w, n.rect.h), (r.w + 60, r.h + 40));
    assert_eq!(n.content.width() as i32, 260);
    // Too small is clamped to the minimum.
    let r = d.window(2).unwrap().rect;
    d.pointer_moved(r.right() - 2, r.bottom() - 2);
    d.press();
    d.pointer_moved(r.x + 5, r.y + 5);
    d.release();
    let n = d.window(2).unwrap();
    assert_eq!((n.content.width(), n.content.height()), (200, 100));
    // A click on the edge without moving changes nothing.
    let r = d.window(2).unwrap().rect;
    d.pointer_moved(r.right() - 2, r.bottom() - 2);
    d.press();
    assert!(d.release().is_empty());
}

#[test]
fn a_resized_window_composes_the_same_in_pieces() {
    let mut d = desk();
    d.set_resizable(2, true);
    d.toggle_maximize(2);
    let mut whole = Canvas::new(W, H, 0);
    d.compose(&mut whole, d.screen());
    let mut pieces = Canvas::new(W, H, 0);
    for y in (0..H as i32).step_by(41) {
        for x in (0..W as i32).step_by(59) {
            d.compose(&mut pieces, Rect::new(x, y, 59, 41));
        }
    }
    assert_eq!(whole.pixels(), pieces.pixels());
}

// ---- Desktop icons, marquee selection, double-click ----

/// `desk()` plus three icons, with the two windows moved out of the icon column.
fn icon_desk() -> Desktop {
    let mut d = desk();
    d.move_window(1, 300, 100);
    d.move_window(2, 400, 200);
    d.set_icons(vec![(10, String::from("Terminal")), (11, String::from("Files")), (12, String::from("Notes"))]);
    d.take_damage();
    d
}

fn press_at(d: &mut Desktop, x: i32, y: i32) -> Vec<Event> {
    d.pointer_moved(x, y);
    d.press()
}

#[test]
fn icons_lay_out_down_the_left_and_wrap_into_columns() {
    let d = icon_desk();
    let r: Vec<Rect> = (0..3).map(|i| d.icon_rect(i)).collect();
    assert_eq!((r[0].x, r[1].x, r[2].x), (r[0].x, r[0].x, r[0].x), "one column");
    assert!(r[0].y < r[1].y && r[1].y < r[2].y);
    assert!(r.iter().all(|c| !c.is_empty() && d.work_area().intersect(c) == *c));
    // A short screen pushes later icons into the next column.
    let mut small = Desktop::new(800, 200, "t");
    small.set_icons((0..5).map(|i| (i, String::from("x"))).collect());
    assert!(small.icon_rect(4).x > small.icon_rect(0).x);
    assert_eq!(small.icon_rect(0).y, small.icon_rect(4).y.min(small.icon_rect(0).y));
    // Hit testing finds them (and windows still win where they overlap).
    let (cx, cy) = center(r[1]);
    assert_eq!(d.hit(cx, cy), Hit::Icon(11));
    assert_eq!(d.hit(700, 20), Hit::Desktop);
}

#[test]
fn clicking_an_icon_selects_only_it_and_the_wallpaper_clears() {
    let mut d = icon_desk();
    let (x0, y0) = center(d.icon_rect(0));
    let (x1, y1) = center(d.icon_rect(1));
    assert_eq!(press_at(&mut d, x0, y0), vec![Event::Selected(vec![10])]);
    d.release();
    assert_eq!(press_at(&mut d, x1, y1), vec![Event::Selected(vec![11])]);
    d.release();
    assert_eq!(d.selected_icons(), vec![11]);
    // Clicking the same one again changes nothing worth reporting.
    d.set_time(5000);
    assert!(press_at(&mut d, x1, y1).is_empty());
    d.release();
    // The empty wallpaper clears the selection.
    assert_eq!(press_at(&mut d, 700, 20), vec![Event::Selected(Vec::new())]);
    d.release();
    assert!(d.selected_icons().is_empty());
}

#[test]
fn dragging_a_rubber_band_selects_what_it_touches_in_any_direction() {
    let mut d = icon_desk();
    let (r0, r1, r2) = (d.icon_rect(0), d.icon_rect(1), d.icon_rect(2));
    // Down-right across the first two icons only.
    press_at(&mut d, r0.x - 2, r0.y - 2);
    assert!(d.marquee_rect().is_some());
    d.pointer_moved(r1.x + 10, r1.y + 10);
    assert_eq!(d.selected_icons(), vec![10, 11], "selected while dragging");
    assert_eq!(d.release(), vec![Event::Selected(vec![10, 11])]);
    assert!(d.marquee_rect().is_none());
    // Up-left from below the last icon back over the last two: the band is normalised.
    press_at(&mut d, r2.right() + 4, r2.bottom() + 4);
    d.pointer_moved(r1.x + 20, r1.y + 20);
    assert_eq!(d.selected_icons(), vec![11, 12]);
    // Shrinking the band deselects again.
    d.pointer_moved(r1.x + 20, r2.bottom() - 2);
    assert_eq!(d.selected_icons(), vec![12]);
    d.release();
    // A band over nothing selects nothing.
    press_at(&mut d, 600, 400);
    d.pointer_moved(650, 450);
    assert!(d.selected_icons().is_empty());
    assert_eq!(d.release(), vec![Event::Selected(Vec::new())]);
}

#[test]
fn a_plain_click_on_the_wallpaper_is_not_a_drag_and_windows_block_the_band() {
    let mut d = icon_desk();
    // Two pixels of jitter stays a click: nothing is drawn or selected.
    press_at(&mut d, 600, 400);
    d.pointer_moved(602, 401);
    let mut shot = Canvas::new(W, H, 0);
    d.compose(&mut shot, d.screen());
    assert!(d.release().is_empty());
    // Pressing inside a window starts no band.
    let c = d.content_rect(1).unwrap();
    press_at(&mut d, c.x + 10, c.y + 10);
    assert!(d.marquee_rect().is_none());
    d.release();
    // While locked, the wallpaper is dead.
    d.show_lock(crate::lock::Mode::Unlock);
    press_at(&mut d, 600, 400);
    assert!(d.marquee_rect().is_none());
}

#[test]
fn double_click_opens_an_icon_but_only_when_quick_close_and_on_the_same_one() {
    let mut d = icon_desk();
    let (x0, y0) = center(d.icon_rect(0));
    let (x1, y1) = center(d.icon_rect(1));
    d.set_time(1000);
    press_at(&mut d, x0, y0);
    d.release();
    d.set_time(1300);
    assert_eq!(press_at(&mut d, x0 + 2, y0), vec![Event::IconOpened(10)]);
    d.release();
    // A third click starts over rather than counting as another double.
    d.set_time(1400);
    assert!(!press_at(&mut d, x0, y0).contains(&Event::IconOpened(10)));
    d.release();
    // Too slow.
    d.set_time(1400 + 2000);
    press_at(&mut d, x0, y0);
    d.release();
    d.set_time(1400 + 2000 + 1000);
    assert!(!press_at(&mut d, x0, y0).contains(&Event::IconOpened(10)));
    d.release();
    // A different icon in between.
    d.set_time(9000);
    press_at(&mut d, x0, y0);
    d.release();
    d.set_time(9100);
    assert!(!press_at(&mut d, x1, y1).contains(&Event::IconOpened(11)));
}

#[test]
fn double_clicking_a_title_bar_maximises_and_restores() {
    let mut d = icon_desk();
    d.set_resizable(1, true);
    let before = d.window(1).unwrap().rect;
    let t = d.chrome(1).unwrap().0;
    let (x, y) = (t.x + 20, t.y + 5);
    d.set_time(100);
    press_at(&mut d, x, y);
    d.release();
    d.set_time(300);
    assert!(press_at(&mut d, x, y).contains(&Event::Resized(1)));
    d.release();
    assert!(d.is_maximized(1));
    assert_eq!(d.window(1).unwrap().rect, d.work_area());
    // Again: restore. Windows that cannot be resized ignore it.
    let t = d.chrome(1).unwrap().0;
    let (x, y) = (t.x + 20, t.y + 5);
    d.set_time(2000);
    press_at(&mut d, x, y);
    d.release();
    d.set_time(2200);
    assert!(press_at(&mut d, x, y).contains(&Event::Resized(1)));
    d.release();
    assert_eq!(d.window(1).unwrap().rect, before);
    let t = d.chrome(2).unwrap().0;
    d.set_time(5000);
    press_at(&mut d, t.x + 20, t.y + 5);
    d.release();
    d.set_time(5100);
    assert!(!press_at(&mut d, t.x + 20, t.y + 5).contains(&Event::Resized(2)));
}

#[test]
fn icons_and_the_band_compose_the_same_in_pieces() {
    let mut d = icon_desk();
    let (r0, r1) = (d.icon_rect(0), d.icon_rect(1));
    press_at(&mut d, r0.x - 2, r0.y - 2);
    d.pointer_moved(r1.x + 30, r1.y + 30);
    let mut whole = Canvas::new(W, H, 0);
    d.compose(&mut whole, d.screen());
    let mut pieces = Canvas::new(W, H, 0);
    for y in (0..H as i32).step_by(43) {
        for x in (0..W as i32).step_by(61) {
            d.compose(&mut pieces, Rect::new(x, y, 61, 43));
        }
    }
    assert_eq!(whole.pixels(), pieces.pixels());
    // The band is visible: pixels inside it differ from the plain wallpaper.
    let mut plain = Canvas::new(W, H, 0);
    d.release();
    d.pointer_moved(700, 20);
    d.compose(&mut plain, d.screen());
    let (bx, by) = (r0.x + 3, r0.y + 3);
    assert_ne!(whole.get(bx, by), plain.get(bx, by));
}

#[test]
fn moving_the_marquee_repaints_only_its_neighbourhood() {
    let mut d = icon_desk();
    press_at(&mut d, 500, 300);
    d.pointer_moved(560, 340);
    d.take_damage();
    d.pointer_moved(570, 345);
    let dmg = d.take_damage().iter().fold(Rect::default(), |a, r| a.union(r));
    assert!(dmg.w < 200 && dmg.h < 150, "{dmg:?}: far less than the whole screen");
}
