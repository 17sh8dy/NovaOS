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
